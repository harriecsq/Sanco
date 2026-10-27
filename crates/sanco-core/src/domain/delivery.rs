//! Delivery receipts (DRs): requested → approved → delivered, or cancelled.
//!
//! A DR is requested from a confirmed booking, from one location, with the
//! lot (expiry) of each line. Its header and lines stay editable, also after
//! delivery (the stock difference is checked). Delivering it takes the stock out
//! (`v_stock_moves`, kind `delivery`) and, from M8, creates a receivable.
//! A delivered DR is voided (cancelled with `delivered_at` kept), which
//! puts the stock back.
//!
//! The DR number is the BIR booklet number. Staff type it (any time until
//! delivery, required by then) unless the DR series is set to automatic
//! (`numbering::is_manual`), in which case it is given at approval.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::bookings::BookingStatus;
use crate::domain::boxes::{self, BoxDraw, BoxDrawInput, BoxStock};
use crate::domain::inventory::{self, Effect, StockLot};
use crate::domain::payments::{self, DrPayment};
use crate::domain::pricing::{self, VatAmounts};
use crate::domain::receivables;
use crate::domain::{
    MAX_LINES, date_opt, line_amount, line_summary, peso, replace_lines, require_date, settings,
    text_opt,
};
use crate::error::{Error, Result};
use crate::repo::WriteTx;
use crate::sync::numbering::{self, Reservation, series};
use crate::time::manila_date_string;

pub const DELIVERY_RECEIPTS: &str = "delivery_receipts";
pub const DR_LINES: &str = "dr_lines";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum DrStatus {
    Requested,
    Approved,
    Delivered,
    Cancelled,
}

impl DrStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            DrStatus::Requested => "requested",
            DrStatus::Approved => "approved",
            DrStatus::Delivered => "delivered",
            DrStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("approved") => DrStatus::Approved,
            Some("delivered") => DrStatus::Delivered,
            Some("cancelled") => DrStatus::Cancelled,
            _ => DrStatus::Requested,
        }
    }

    /// Requested or approved: not delivered yet.
    pub fn is_open(self) -> bool {
        matches!(self, DrStatus::Requested | DrStatus::Approved)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DrLineInput {
    /// The DR line being edited; `None` for a new line.
    pub id: Option<String>,
    pub booking_line_id: String,
    /// The lot. Required for items that track expiry, ignored otherwise
    /// and for items in boxes (each box has its lot).
    pub expiry: Option<String>,
    pub qty: i64,
    /// For items in boxes: pieces from each box, adding up to `qty`. Empty =
    /// picked automatically (started boxes, then earliest expiry, then
    /// lowest serial).
    pub boxes: Vec<BoxDrawInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DrRequestInput {
    pub booking_id: String,
    /// Planned delivery date.
    pub date: String,
    pub from_location_id: String,
    pub deliver_to: String,
    pub notes: String,
    pub lines: Vec<DrLineInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DeliverInput {
    /// When the goods were delivered (not in the future).
    pub date: String,
    /// Who received them at the client.
    pub received_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DrInfo {
    pub id: String,
    /// The DR (booklet) number; `None` until entered.
    pub no: Option<String>,
    /// False if other PCs were offline when the number was entered.
    pub no_confirmed: bool,
    /// Typed by staff.
    pub no_manual: bool,
    /// Another document has the same number (entered while offline).
    pub no_clash: bool,
    pub booking_id: Option<String>,
    pub booking_no: Option<String>,
    pub client_id: Option<String>,
    pub client_code: Option<String>,
    pub client_name: Option<String>,
    pub date: String,
    pub from_location_id: Option<String>,
    pub from_location_name: Option<String>,
    pub status: DrStatus,
    /// Cancelled after it was delivered: its stock came back.
    pub voided: bool,
    pub vat: bool,
    pub vat_bp: i64,
    pub totals: VatAmounts,
    pub deliver_to: Option<String>,
    pub notes: Option<String>,
    pub summary: String,
    pub item_ids: Vec<String>,
    pub line_count: i64,
    pub created_at: i64,
    pub created_by_name: Option<String>,
    pub approved_at: Option<i64>,
    pub approved_by_name: Option<String>,
    pub delivered_date: Option<String>,
    pub delivered_at: Option<i64>,
    pub delivered_by_name: Option<String>,
    pub received_by: Option<String>,
    pub cancelled_at: Option<i64>,
    pub cancelled_by_name: Option<String>,
    pub cancel_reason: Option<String>,
    /// Paid by counting payments (delivered DRs only).
    pub paid_c: i64,
    /// Gross minus paid while delivered, else 0. Negative if overpaid.
    pub balance_c: i64,
    /// Delivery date plus the client's payment terms.
    pub due_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DrLine {
    pub id: String,
    pub booking_line_id: Option<String>,
    pub item_id: String,
    pub item_code: Option<String>,
    pub item_name: Option<String>,
    pub unit: Option<String>,
    pub expiry: Option<String>,
    pub qty: i64,
    pub price_c: i64,
    pub amount_c: i64,
    /// The item's box size, for items in boxes.
    pub pcs_per_box: Option<i64>,
    pub boxed: bool,
    /// What it takes from each box.
    pub boxes: Vec<BoxDraw>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DrDetail {
    pub dr: DrInfo,
    pub lines: Vec<DrLine>,
    /// DR numbers are typed by staff (else given at approval).
    pub manual_numbers: bool,
    /// Payments allocated to it.
    pub payments: Vec<DrPayment>,
}

// Writing -------------------------------------------------------------------

struct PreparedRequest {
    client_id: Option<String>,
    vat: bool,
    date: String,
    lines: Vec<PreparedLine>,
}

struct PreparedLine {
    id: Option<String>,
    booking_line_id: String,
    item_id: String,
    expiry: Option<String>,
    qty: i64,
    price_c: i64,
    /// (box, expiry, qty) for items in boxes.
    draws: Vec<(String, Option<String>, i64)>,
}

/// The DR being edited, as it is now.
struct Current {
    status: DrStatus,
    location: String,
    lines: Vec<DrLine>,
}

/// Lines of a DR (or all), as (dr, line), with their box draws.
fn lines_of(conn: &Connection, dr: Option<&str>) -> Result<Vec<(String, DrLine)>> {
    let mut draws = boxes::draws_of(conn, dr)?;
    let mut stmt = conn.prepare_cached(
        "SELECT l.dr_id, l.id, l.booking_line_id, l.item_id, i.code, i.name, i.unit, l.expiry,
                COALESCE(l.qty, 0), COALESCE(l.price_c, 0), i.serial_code, i.pcs_per_box
         FROM dr_lines l LEFT JOIN items i ON i.id = l.item_id
         WHERE l._deleted = 0 AND (?1 IS NULL OR l.dr_id = ?1)
         ORDER BY l.dr_id, l.line_no, l.id",
    )?;
    let rows = stmt.query_map([dr], |r| {
        let (qty, price_c): (i64, i64) = (r.get(8)?, r.get(9)?);
        let boxed = r.get::<_, Option<String>>(10)?.is_some_and(|c| !c.is_empty());
        Ok((
            r.get(0)?,
            DrLine {
                id: r.get(1)?,
                booking_line_id: r.get(2)?,
                item_id: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                item_code: r.get(4)?,
                item_name: r.get(5)?,
                unit: r.get(6)?,
                expiry: r.get(7)?,
                qty,
                price_c,
                amount_c: line_amount(qty, price_c),
                pcs_per_box: if boxed { r.get(11)? } else { None },
                boxed,
                boxes: Vec::new(),
            },
        ))
    })?;
    let mut out: Vec<(String, DrLine)> = rows.collect::<Result<_, _>>()?;
    for (_, l) in &mut out {
        l.boxes = draws.remove(&l.id).unwrap_or_default();
    }
    Ok(out)
}

/// What delivering the DR takes out of stock, per lot and box.
fn effects(conn: &Connection, dr_id: &str, location: &str) -> Result<Vec<Effect>> {
    let mut lots: BTreeMap<(String, Option<String>, Option<String>), i64> = BTreeMap::new();
    for (_, l) in lines_of(conn, Some(dr_id))? {
        if l.boxes.is_empty() {
            *lots.entry((l.item_id, l.expiry, None)).or_default() -= l.qty;
            continue;
        }
        for d in l.boxes {
            *lots
                .entry((l.item_id.clone(), d.expiry, Some(d.box_id)))
                .or_default() -= d.qty;
        }
    }
    Ok(lots
        .into_iter()
        .map(|((item_id, expiry, box_id), qty)| Effect {
            item_id,
            location_id: location.to_string(),
            expiry,
            qty,
            box_id,
        })
        .collect())
}

/// Checks the draws typed for a line, or picks them. `back`: pieces the
/// DR itself took from each box at this location before (a delivered DR
/// being edited); `used`: what earlier lines of this DR take.
#[allow(clippy::too_many_arguments)]
fn line_draws(
    conn: &Connection,
    item_id: &str,
    code: &str,
    unit: Option<&str>,
    location: &str,
    line: &DrLineInput,
    back: &HashMap<String, (Option<String>, i64)>,
    used: &HashMap<String, i64>,
) -> Result<Vec<(String, Option<String>, i64)>> {
    let unit = unit.map(|u| format!(" {u}")).unwrap_or_default();
    if !line.boxes.is_empty() {
        let mut merged: Vec<(String, Option<String>, i64)> = Vec::new();
        for d in &line.boxes {
            let (item, _) = boxes::box_row(conn, &d.box_id)?
                .ok_or_else(|| Error::Invalid("one of the boxes no longer exists".into()))?;
            if item != item_id {
                return Err(Error::Invalid(format!("one of the boxes doesn't hold {code}")));
            }
            if d.qty <= 0 {
                continue;
            }
            match merged.iter_mut().find(|m| m.0 == d.box_id) {
                Some(m) => m.2 += d.qty,
                None => merged.push((d.box_id.clone(), boxes::box_expiry(conn, &d.box_id)?, d.qty)),
            }
        }
        let total: i64 = merged.iter().map(|m| m.2).sum();
        if total != line.qty {
            return Err(Error::Invalid(format!(
                "the boxes of {code} add up to {total}{unit}, not {}{unit}",
                line.qty
            )));
        }
        return Ok(merged);
    }
    let stock = boxes::box_stock(conn, Some(item_id), Some(location), "")?;
    let mut order: Vec<(String, Option<String>)> = Vec::new();
    let mut available: HashMap<String, i64> = HashMap::new();
    let mut sizes: HashMap<String, i64> = HashMap::new();
    for b in stock {
        order.push((b.box_id.clone(), b.expiry.clone()));
        available.insert(b.box_id.clone(), b.qty);
        sizes.insert(b.box_id, b.pcs);
    }
    for (box_id, (expiry, qty)) in back {
        if !available.contains_key(box_id) {
            order.push((box_id.clone(), expiry.clone()));
            sizes.insert(box_id.clone(), boxes::box_row(conn, box_id)?.map_or(0, |b| b.1));
        }
        *available.entry(box_id.clone()).or_default() += qty;
    }
    for (box_id, qty) in used {
        if let Some(a) = available.get_mut(box_id) {
            *a -= qty;
        }
    }
    let draws = boxes::auto_draws(&order, &available, &sizes, line.qty);
    let filled: i64 = draws.iter().map(|d| d.2).sum();
    if filled < line.qty {
        return Err(Error::Invalid(format!(
            "only {filled}{unit} of {code} {} in boxes at that location",
            if filled == 1 { "is" } else { "are" }
        )));
    }
    Ok(draws)
}

fn check_location(conn: &Connection, id: &str) -> Result<()> {
    let found: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM locations WHERE id = ?1 AND _deleted = 0)",
        [id],
        |r| r.get(0),
    )?;
    if found {
        Ok(())
    } else {
        Err(Error::Invalid(
            "choose the location the stock comes from".into(),
        ))
    }
}

fn prepare_request(
    conn: &Connection,
    input: &DrRequestInput,
    current: Option<&Current>,
) -> Result<PreparedRequest> {
    let booking: Option<(Option<String>, Option<String>, i64)> = conn
        .query_row(
            "SELECT client_id, status, COALESCE(vat, 0) FROM bookings
             WHERE id = ?1 AND _deleted = 0",
            [&input.booking_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (client_id, status, vat) =
        booking.ok_or_else(|| Error::Invalid("that booking no longer exists".into()))?;
    if current.is_none() && BookingStatus::parse(status.as_deref()) != BookingStatus::Confirmed {
        return Err(Error::Invalid(
            "DRs can only be requested for a confirmed booking".into(),
        ));
    }
    let date = require_date(&input.date, "delivery date")?;
    check_location(conn, &input.from_location_id)?;
    if input.lines.is_empty() {
        return Err(Error::Invalid(
            "enter a quantity for at least one item".into(),
        ));
    }
    if input.lines.len() > MAX_LINES {
        return Err(Error::Invalid(format!(
            "a DR can have at most {MAX_LINES} lines"
        )));
    }

    let progress = line_progress(conn, Some(&input.booking_id))?;
    // An edited DR's own lines don't count against what's left.
    let mut own: HashMap<String, i64> = HashMap::new();
    type Lot = (String, String, Option<String>, Option<String>);
    let mut lots: BTreeMap<Lot, i64> = BTreeMap::new();
    // What a delivered DR took from each box, if it stays at that location.
    let mut back: HashMap<String, (Option<String>, i64)> = HashMap::new();
    if let Some(cur) = current {
        for l in cur.lines.iter() {
            if let Some(b) = &l.booking_line_id {
                *own.entry(b.clone()).or_default() += l.qty;
            }
            // Delivered stock comes back before the new lines take theirs.
            if cur.status != DrStatus::Delivered {
                continue;
            }
            if l.boxes.is_empty() {
                *lots
                    .entry((l.item_id.clone(), cur.location.clone(), l.expiry.clone(), None))
                    .or_default() += l.qty;
            }
            for d in &l.boxes {
                *lots
                    .entry((
                        l.item_id.clone(),
                        cur.location.clone(),
                        d.expiry.clone(),
                        Some(d.box_id.clone()),
                    ))
                    .or_default() += d.qty;
                if cur.location == input.from_location_id {
                    let e = back.entry(d.box_id.clone()).or_insert((d.expiry.clone(), 0));
                    e.1 += d.qty;
                }
            }
        }
    }
    let mut used: HashMap<String, i64> = HashMap::new();
    let mut on_this: HashMap<String, i64> = HashMap::new();
    let mut lines = Vec::with_capacity(input.lines.len());
    for line in &input.lines {
        type BookingLineRow = (
            Option<String>,
            i64,
            i64,
            Option<String>,
            Option<String>,
            i64,
        );
        let row: Option<BookingLineRow> = conn
            .query_row(
                "SELECT l.item_id, COALESCE(l.qty, 0), COALESCE(l.price_c, 0), i.code, i.unit,
                        COALESCE(i.tracks_expiry, 0)
                 FROM booking_lines l LEFT JOIN items i ON i.id = l.item_id
                 WHERE l.id = ?1 AND l.booking_id = ?2 AND l._deleted = 0",
                (&line.booking_line_id, &input.booking_id),
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .optional()?;
        let (item_id, ordered, price_c, code, unit, tracks) = row.ok_or_else(|| {
            Error::Invalid("a line of this booking no longer exists; reload it".into())
        })?;
        let item_id = item_id.unwrap_or_default();
        let code = code.unwrap_or_default();
        if line.qty <= 0 {
            return Err(Error::Invalid(format!(
                "the quantity of {code} must be positive"
            )));
        }
        let boxed = boxes::serial_of(conn, &item_id)?.is_some();
        let expiry = date_opt(line.expiry.as_deref(), &format!("expiry date of {code}"))?;
        let expiry = match (boxed, tracks != 0, expiry) {
            (true, ..) => None,
            (false, true, None) => {
                return Err(Error::Invalid(format!(
                    "choose the lot (expiry date) of {code}"
                )));
            }
            (false, false, _) => None,
            (false, true, e) => e,
        };
        let qty = on_this.entry(line.booking_line_id.clone()).or_default();
        *qty += line.qty;
        let (pending, delivered) = progress
            .get(&line.booking_line_id)
            .copied()
            .unwrap_or_default();
        let left =
            ordered - pending - delivered + own.get(&line.booking_line_id).copied().unwrap_or(0);
        if *qty > left {
            let unit = unit.map(|u| format!(" {u}")).unwrap_or_default();
            return Err(Error::Invalid(format!(
                "only {}{unit} of {code} {} left to deliver on this booking",
                left.max(0),
                if left == 1 { "is" } else { "are" },
            )));
        }
        let draws = if boxed {
            line_draws(
                conn,
                &item_id,
                &code,
                unit.as_deref(),
                &input.from_location_id,
                line,
                &back,
                &used,
            )?
        } else {
            Vec::new()
        };
        for (box_id, box_expiry, n) in &draws {
            *used.entry(box_id.clone()).or_default() += n;
            *lots
                .entry((
                    item_id.clone(),
                    input.from_location_id.clone(),
                    box_expiry.clone(),
                    Some(box_id.clone()),
                ))
                .or_default() -= n;
        }
        if !boxed {
            *lots
                .entry((
                    item_id.clone(),
                    input.from_location_id.clone(),
                    expiry.clone(),
                    None,
                ))
                .or_default() -= line.qty;
        }
        // A kept line keeps its price; new lines take the booking's.
        let kept = current
            .and_then(|c| c.lines.iter().find(|l| Some(&l.id) == line.id.as_ref()))
            .filter(|l| l.booking_line_id.as_deref() == Some(line.booking_line_id.as_str()));
        lines.push(PreparedLine {
            id: kept.map(|l| l.id.clone()),
            booking_line_id: line.booking_line_id.clone(),
            item_id,
            expiry,
            qty: line.qty,
            price_c: kept.map_or(price_c, |l| l.price_c),
            draws,
        });
    }
    let effects: Vec<Effect> = lots
        .into_iter()
        .map(|((item_id, location_id, expiry, box_id), qty)| Effect {
            item_id,
            location_id,
            expiry,
            qty,
            box_id,
        })
        .collect();
    // Only lots the DR takes more from than before are checked.
    inventory::check_not_negative(conn, &effects, "")?;
    Ok(PreparedRequest {
        client_id,
        vat: vat != 0,
        date,
        lines,
    })
}

/// Checks a DR request as [`request`] would, without writing.
pub fn validate_request(conn: &Connection, input: &DrRequestInput) -> Result<()> {
    prepare_request(conn, input, None).map(drop)
}

fn line_rows(lines: Vec<PreparedLine>) -> Vec<(Option<String>, serde_json::Value)> {
    lines
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            (
                l.id,
                json!({
                    "line_no": i as i64 + 1,
                    "booking_line_id": l.booking_line_id,
                    "item_id": l.item_id,
                    "expiry": l.expiry,
                    "qty": l.qty,
                    "price_c": l.price_c,
                }),
            )
        })
        .collect()
}

/// Writes the DR's lines and each line's box draws.
fn write_lines(tx: &mut WriteTx, dr_id: &str, lines: Vec<PreparedLine>) -> Result<()> {
    let draws: Vec<Vec<(String, Option<String>, i64)>> =
        lines.iter().map(|l| l.draws.clone()).collect();
    replace_lines(tx, DR_LINES, "dr_id", dr_id, line_rows(lines))?;
    let ids: Vec<String> = {
        let mut stmt = tx.conn().prepare_cached(
            "SELECT id FROM dr_lines WHERE dr_id = ?1 AND _deleted = 0 ORDER BY line_no, id",
        )?;
        let rows = stmt.query_map([dr_id], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for (id, d) in ids.iter().zip(draws) {
        boxes::set_draws(tx, dr_id, id, &d)?;
    }
    Ok(())
}

/// Requests a DR for a confirmed booking. It gets its number later.
/// Returns its id.
pub fn request(tx: &mut WriteTx, input: &DrRequestInput) -> Result<String> {
    let p = prepare_request(tx.conn(), input, None)?;
    let vat_bp = settings::vat_rate_bp(tx.conn())?;
    let id = tx.insert(
        DELIVERY_RECEIPTS,
        json!({
            "booking_id": input.booking_id,
            "client_id": p.client_id,
            "date": p.date,
            "from_location_id": input.from_location_id,
            "status": DrStatus::Requested.as_str(),
            "vat": p.vat,
            "vat_bp": vat_bp,
            "deliver_to": text_opt(&input.deliver_to),
            "notes": text_opt(&input.notes),
        }),
    )?;
    write_lines(tx, &id, p.lines)?;
    Ok(id)
}

fn current(conn: &Connection, id: &str) -> Result<Current> {
    let (status, location) = status_of(conn, id)?;
    if status == DrStatus::Cancelled {
        return Err(Error::Invalid("this DR is cancelled".into()));
    }
    Ok(Current {
        status,
        location,
        lines: lines_of(conn, Some(id))?
            .into_iter()
            .map(|(_, l)| l)
            .collect(),
    })
}

/// Checks an edit as [`update`] would, without writing. `today` is Manila's.
pub fn check_update(
    conn: &Connection,
    id: &str,
    input: &DrRequestInput,
    delivery: Option<&DeliverInput>,
    today: &str,
) -> Result<()> {
    let cur = current(conn, id)?;
    let booking: Option<String> = conn.query_row(
        "SELECT booking_id FROM delivery_receipts WHERE id = ?1",
        [id],
        |r| r.get(0),
    )?;
    if booking.as_deref() != Some(input.booking_id.as_str()) {
        return Err(Error::Invalid("a DR stays with its booking".into()));
    }
    if let (DrStatus::Delivered, Some(d)) = (cur.status, delivery) {
        let date = require_date(&d.date, "delivery date")?;
        if date.as_str() > today {
            return Err(Error::Invalid(
                "the delivery date can't be in the future".into(),
            ));
        }
    }
    let p = prepare_request(conn, input, Some(&cur))?;
    if cur.status == DrStatus::Delivered {
        let paid = receivables::dr_paid(conn, id)?;
        if paid > 0 {
            let (vat, vat_bp): (i64, i64) = conn.query_row(
                "SELECT COALESCE(vat, 0), COALESCE(vat_bp, 0) FROM delivery_receipts WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let net = p.lines.iter().map(|l| line_amount(l.qty, l.price_c)).sum();
            let gross = pricing::with_vat(net, if vat != 0 { vat_bp } else { 0 }).gross_c;
            if gross < paid {
                return Err(Error::Invalid(format!(
                    "payments of {} are applied to this DR; its total can't go below that ({} now). Change the payments first",
                    peso(paid),
                    peso(gross)
                )));
            }
        }
    }
    Ok(())
}

/// Edits a DR that isn't cancelled: its date, location, address, notes and
/// lines, and for a delivered DR the delivery date and who received it.
/// Stock moves by the difference.
pub fn update(
    tx: &mut WriteTx,
    id: &str,
    input: &DrRequestInput,
    delivery: Option<&DeliverInput>,
) -> Result<()> {
    let today = manila_date_string(tx.now_ms());
    check_update(tx.conn(), id, input, delivery, &today)?;
    let cur = current(tx.conn(), id)?;
    let p = prepare_request(tx.conn(), input, Some(&cur))?;
    let mut patch = json!({
        "date": p.date,
        "from_location_id": input.from_location_id,
        "deliver_to": text_opt(&input.deliver_to),
        "notes": text_opt(&input.notes),
    });
    if let (DrStatus::Delivered, Some(d)) = (cur.status, delivery) {
        patch["delivered_date"] = json!(require_date(&d.date, "delivery date")?);
        patch["received_by"] = json!(text_opt(&d.received_by));
    }
    tx.update(DELIVERY_RECEIPTS, id, patch)?;
    write_lines(tx, id, p.lines)
}

/// A live DR's status and source location.
fn status_of(conn: &Connection, id: &str) -> Result<(DrStatus, String)> {
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT status, from_location_id FROM delivery_receipts
             WHERE id = ?1 AND _deleted = 0",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (status, location) =
        row.ok_or_else(|| Error::Invalid("that DR no longer exists".into()))?;
    Ok((
        DrStatus::parse(status.as_deref()),
        location.unwrap_or_default(),
    ))
}

/// The DR's number, if it has one.
pub fn number_of(conn: &Connection, id: &str) -> Result<Option<String>> {
    numbering::number_of(conn, series::DR, id)
}

fn check_reservation(id: &str, res: &Reservation) -> Result<()> {
    if res.series != series::DR || res.ref_id != id {
        return Err(Error::Invalid("wrong number for this DR".into()));
    }
    Ok(())
}

/// Checks that the DR can be approved, without writing.
pub fn check_approve(conn: &Connection, id: &str) -> Result<()> {
    match status_of(conn, id)?.0 {
        DrStatus::Requested => Ok(()),
        s => Err(Error::Invalid(format!("this DR is already {}", s.as_str()))),
    }
}

/// Approves a requested DR, giving it `res` as its number if set.
pub fn approve(tx: &mut WriteTx, id: &str, res: Option<&Reservation>) -> Result<()> {
    check_approve(tx.conn(), id)?;
    if let Some(res) = res {
        check_reservation(id, res)?;
        numbering::assign_or_change(tx, res, DELIVERY_RECEIPTS)?;
    }
    let (now, user) = (tx.now_ms(), tx.actor().user_id.clone());
    tx.update(
        DELIVERY_RECEIPTS,
        id,
        json!({
            "status": DrStatus::Approved.as_str(),
            "approved_at": now,
            "approved_by": user,
        }),
    )
    .map(drop)
}

/// Checks that the DR can be marked delivered on `input.date`, without
/// writing. `today` is Manila's.
pub fn check_deliver(conn: &Connection, id: &str, input: &DeliverInput, today: &str) -> Result<()> {
    let (status, location) = status_of(conn, id)?;
    match status {
        DrStatus::Approved => {}
        DrStatus::Requested => {
            return Err(Error::Invalid("approve the DR before delivering it".into()));
        }
        s => {
            return Err(Error::Invalid(format!("this DR is already {}", s.as_str())));
        }
    }
    let date = require_date(&input.date, "delivery date")?;
    if date.as_str() > today {
        return Err(Error::Invalid(
            "the delivery date can't be in the future".into(),
        ));
    }
    inventory::check_not_negative(conn, &effects(conn, id, &location)?, "")
}

/// Marks an approved DR delivered: its stock goes out. The DR must have a
/// number by then; `res` gives or corrects it.
pub fn deliver(
    tx: &mut WriteTx,
    id: &str,
    input: &DeliverInput,
    res: Option<&Reservation>,
) -> Result<()> {
    let today = manila_date_string(tx.now_ms());
    check_deliver(tx.conn(), id, input, &today)?;
    if let Some(res) = res {
        check_reservation(id, res)?;
        numbering::assign_or_change(tx, res, DELIVERY_RECEIPTS)?;
    }
    if number_of(tx.conn(), id)?.is_none() {
        return Err(Error::Invalid("enter the DR number first".into()));
    }
    let vat_bp = settings::vat_rate_bp(tx.conn())?;
    let (now, user) = (tx.now_ms(), tx.actor().user_id.clone());
    tx.update(
        DELIVERY_RECEIPTS,
        id,
        json!({
            "status": DrStatus::Delivered.as_str(),
            "delivered_date": require_date(&input.date, "delivery date")?,
            "delivered_at": now,
            "delivered_by": user,
            "received_by": text_opt(&input.received_by),
            "vat_bp": vat_bp,
        }),
    )
    .map(drop)
}

/// Checks that the DR's number can be set or corrected, without writing.
pub fn check_set_number(conn: &Connection, id: &str) -> Result<()> {
    match status_of(conn, id)?.0 {
        DrStatus::Cancelled => Err(Error::Invalid("this DR is cancelled".into())),
        _ => Ok(()),
    }
}

/// Sets or corrects a DR's typed number.
pub fn set_number(tx: &mut WriteTx, id: &str, res: &Reservation) -> Result<()> {
    check_set_number(tx.conn(), id)?;
    check_reservation(id, res)?;
    numbering::assign_or_change(tx, res, DELIVERY_RECEIPTS).map(drop)
}

/// Cancels a DR that wasn't delivered.
pub fn cancel(tx: &mut WriteTx, id: &str, reason: &str) -> Result<()> {
    let (status, _) = status_of(tx.conn(), id)?;
    if !status.is_open() {
        return Err(Error::Invalid(match status {
            DrStatus::Delivered => "this DR was delivered; void it instead".into(),
            s => format!("this DR is already {}", s.as_str()),
        }));
    }
    let (now, user) = (tx.now_ms(), tx.actor().user_id.clone());
    tx.update(
        DELIVERY_RECEIPTS,
        id,
        json!({
            "status": DrStatus::Cancelled.as_str(),
            "cancelled_at": now,
            "cancelled_by": user,
            "cancel_reason": text_opt(reason),
        }),
    )
    .map(drop)
}

/// Voids a delivered DR: its stock comes back. Needs a reason.
pub fn void(tx: &mut WriteTx, id: &str, reason: &str) -> Result<()> {
    let (status, _) = status_of(tx.conn(), id)?;
    if status != DrStatus::Delivered {
        return Err(Error::Invalid(
            "only a delivered DR can be voided; cancel it instead".into(),
        ));
    }
    let paid = receivables::dr_paid(tx.conn(), id)?;
    if paid > 0 {
        return Err(Error::Invalid(format!(
            "payments of {} are applied to this DR; take them off the payments first",
            peso(paid)
        )));
    }
    let reason =
        text_opt(reason).ok_or_else(|| Error::Invalid("enter why the DR is voided".into()))?;
    let (now, user) = (tx.now_ms(), tx.actor().user_id.clone());
    tx.update(
        DELIVERY_RECEIPTS,
        id,
        json!({
            "status": DrStatus::Cancelled.as_str(),
            "cancelled_at": now,
            "cancelled_by": user,
            "cancel_reason": reason,
        }),
    )
    .map(drop)
}

// Reading -------------------------------------------------------------------

/// Per booking line: qty on open DRs and qty delivered. Of one booking, or
/// of all.
pub fn line_progress(
    conn: &Connection,
    booking: Option<&str>,
) -> Result<HashMap<String, (i64, i64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT l.booking_line_id,
                SUM(CASE WHEN r.status IN ('requested', 'approved') THEN l.qty ELSE 0 END),
                SUM(CASE WHEN r.status = 'delivered' THEN l.qty ELSE 0 END)
         FROM dr_lines l
         JOIN delivery_receipts r ON r.id = l.dr_id AND r._deleted = 0
         WHERE l._deleted = 0 AND l.booking_line_id IS NOT NULL
           AND (?1 IS NULL OR r.booking_id = ?1)
         GROUP BY l.booking_line_id",
    )?;
    let rows = stmt.query_map([booking], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Open DRs (requested or approved) per booking.
pub fn open_counts(conn: &Connection) -> Result<HashMap<String, i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT booking_id, COUNT(*) FROM delivery_receipts
         WHERE _deleted = 0 AND status IN ('requested', 'approved') AND booking_id IS NOT NULL
         GROUP BY booking_id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A booking's (open, not cancelled) DRs.
pub fn dr_counts(conn: &Connection, booking_id: &str) -> Result<(i64, i64)> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(status IN ('requested', 'approved')), 0),
                COALESCE(SUM(status IN ('requested', 'approved', 'delivered')), 0)
         FROM delivery_receipts WHERE booking_id = ?1 AND _deleted = 0",
        [booking_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

const SELECT: &str = "
    SELECT r.id, n.no, COALESCE(n.confirmed, 1), COALESCE(n.manual, 0),
           EXISTS (SELECT 1 FROM doc_numbers o WHERE o._deleted = 0 AND o.series = n.series
                   AND o.period = n.period AND o.seq = n.seq AND o.id <> n.id),
           r.booking_id, bn.no, r.client_id, cn.no, c.name, r.date, r.from_location_id, l.name,
           r.status, COALESCE(r.vat, 0), COALESCE(r.vat_bp, 0), r.deliver_to, r.notes,
           r.created_at, uc.display_name, r.approved_at, ua.display_name, r.delivered_date,
           r.delivered_at, ud.display_name, r.received_by, r.cancelled_at, ux.display_name,
           r.cancel_reason, c.terms_days
    FROM delivery_receipts r
    LEFT JOIN doc_numbers n ON n.ref_id = r.id AND n.series = 'DR' AND n._deleted = 0
    LEFT JOIN doc_numbers bn ON bn.ref_id = r.booking_id AND bn.series = 'BK' AND bn._deleted = 0
    LEFT JOIN clients c ON c.id = r.client_id
    LEFT JOIN doc_numbers cn ON cn.ref_id = r.client_id AND cn.series = 'CL' AND cn._deleted = 0
    LEFT JOIN locations l ON l.id = r.from_location_id
    LEFT JOIN users uc ON uc.id = r.created_by
    LEFT JOIN users ua ON ua.id = r.approved_by
    LEFT JOIN users ud ON ud.id = r.delivered_by
    LEFT JOIN users ux ON ux.id = r.cancelled_by
    WHERE r._deleted = 0";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<DrInfo> {
    let status = DrStatus::parse(r.get::<_, Option<String>>(13)?.as_deref());
    let delivered_at: Option<i64> = r.get(23)?;
    Ok(DrInfo {
        id: r.get(0)?,
        no: r.get(1)?,
        no_confirmed: r.get::<_, i64>(2)? != 0,
        no_manual: r.get::<_, i64>(3)? != 0,
        no_clash: r.get(4)?,
        booking_id: r.get(5)?,
        booking_no: r.get(6)?,
        client_id: r.get(7)?,
        client_code: r.get(8)?,
        client_name: r.get(9)?,
        date: r.get::<_, Option<String>>(10)?.unwrap_or_default(),
        from_location_id: r.get(11)?,
        from_location_name: r.get(12)?,
        voided: status == DrStatus::Cancelled && delivered_at.is_some(),
        status,
        vat: r.get::<_, i64>(14)? != 0,
        vat_bp: r.get(15)?,
        deliver_to: r.get(16)?,
        notes: r.get(17)?,
        created_at: r.get(18)?,
        created_by_name: r.get(19)?,
        approved_at: r.get(20)?,
        approved_by_name: r.get(21)?,
        delivered_date: r.get(22)?,
        delivered_at,
        delivered_by_name: r.get(24)?,
        received_by: r.get(25)?,
        cancelled_at: r.get(26)?,
        cancelled_by_name: r.get(27)?,
        cancel_reason: r.get(28)?,
        paid_c: 0,
        balance_c: 0,
        due_date: receivables::due_date(r.get::<_, Option<String>>(22)?.as_deref(), r.get(29)?),
        totals: pricing::with_vat(0, 0),
        summary: String::new(),
        item_ids: Vec::new(),
        line_count: 0,
    })
}

fn fill(d: &mut DrInfo, lines: &[DrLine], paid_c: i64) {
    let net = lines.iter().map(|l| l.amount_c).sum();
    d.totals = pricing::with_vat(net, if d.vat { d.vat_bp } else { 0 });
    if d.status == DrStatus::Delivered {
        d.paid_c = paid_c;
        d.balance_c = d.totals.gross_c - paid_c;
    } else {
        d.due_date = None;
    }
    d.summary = line_summary(lines.iter().map(|l| (l.item_code.as_deref(), l.qty)));
    d.item_ids = lines.iter().map(|l| l.item_id.clone()).collect();
    d.item_ids.sort();
    d.item_ids.dedup();
    d.line_count = lines.len() as i64;
}

fn list_where(conn: &Connection, booking: Option<&str>) -> Result<Vec<DrInfo>> {
    let mut by_dr: HashMap<String, Vec<DrLine>> = HashMap::new();
    for (dr, line) in lines_of(conn, None)? {
        by_dr.entry(dr).or_default().push(line);
    }
    let paid = receivables::paid_by_dr(conn)?;
    let mut stmt = conn.prepare(&format!(
        "{SELECT} AND (?1 IS NULL OR r.booking_id = ?1)
         ORDER BY COALESCE(r.delivered_date, r.date) DESC, r.created_at DESC, r.id DESC"
    ))?;
    let rows = stmt.query_map([booking], from_row)?;
    let mut out = Vec::new();
    for row in rows {
        let mut d = row?;
        let lines = by_dr.remove(&d.id).unwrap_or_default();
        let p = paid.get(&d.id).copied().unwrap_or(0);
        fill(&mut d, &lines, p);
        out.push(d);
    }
    Ok(out)
}

/// Every DR, newest first.
pub fn list(conn: &Connection) -> Result<Vec<DrInfo>> {
    list_where(conn, None)
}

pub fn list_for_booking(conn: &Connection, booking_id: &str) -> Result<Vec<DrInfo>> {
    list_where(conn, Some(booking_id))
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<DrDetail>> {
    let Some(mut dr) = conn
        .query_row(&format!("{SELECT} AND r.id = ?1"), [id], from_row)
        .optional()?
    else {
        return Ok(None);
    };
    let lines: Vec<DrLine> = lines_of(conn, Some(id))?
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    fill(&mut dr, &lines, receivables::dr_paid(conn, id)?);
    Ok(Some(DrDetail {
        dr,
        lines,
        manual_numbers: numbering::is_manual(conn, series::DR)?,
        payments: payments::for_dr(conn, id)?,
    }))
}

/// Stock lots of a booking's items, for picking a DR's lots.
pub fn lots_for_booking(conn: &Connection, booking_id: &str, today: &str) -> Result<Vec<StockLot>> {
    let items: Vec<String> = {
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT item_id FROM booking_lines WHERE booking_id = ?1 AND _deleted = 0",
        )?;
        let rows = stmt.query_map([booking_id], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    Ok(inventory::lots(conn, None, today)?
        .into_iter()
        .filter(|l| items.contains(&l.item_id))
        .collect())
}

/// Boxes in stock of a booking's items, for picking a DR's draws.
pub fn boxes_for_booking(conn: &Connection, booking_id: &str, today: &str) -> Result<Vec<BoxStock>> {
    let items: Vec<String> = {
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT item_id FROM booking_lines WHERE booking_id = ?1 AND _deleted = 0",
        )?;
        let rows = stmt.query_map([booking_id], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    Ok(boxes::box_stock(conn, None, None, today)?
        .into_iter()
        .filter(|b| items.contains(&b.item_id))
        .collect())
}
