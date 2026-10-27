//! Bookings: confirmed client orders.
//!
//! draft → confirmed → closed, or cancelled. A booking (header and
//! lines) can be edited in any status but cancelled. A confirmed
//! booking gets DR requests (see [`crate::domain::delivery`]); how much of
//! each line is requested and delivered is computed from its DRs, never
//! stored.
//!
//! Line prices come from price resolution ([`pricing::quote`]) unless typed
//! in; the line remembers which rule gave its price.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::auth::{Field, Perms};
use crate::domain::delivery::{self, DrInfo};
use crate::domain::pricing::{self, PriceRule, VatAmounts};
use crate::domain::{
    MAX_LINES, items, line_amount, line_summary, replace_lines, require_date, settings, text_opt,
};
use crate::error::{Error, Result};
use crate::repo::WriteTx;
use crate::sync::numbering::{self, Reservation, series};

pub const BOOKINGS: &str = "bookings";
pub const BOOKING_LINES: &str = "booking_lines";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum BookingStatus {
    Draft,
    Confirmed,
    Closed,
    Cancelled,
}

impl BookingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            BookingStatus::Draft => "draft",
            BookingStatus::Confirmed => "confirmed",
            BookingStatus::Closed => "closed",
            BookingStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("confirmed") => BookingStatus::Confirmed,
            Some("closed") => BookingStatus::Closed,
            Some("cancelled") => BookingStatus::Cancelled,
            _ => BookingStatus::Draft,
        }
    }
}

/// Where a line's price came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum PriceSource {
    /// The client's manual rate.
    Manual,
    /// Landed cost at the client's margin tier.
    ClientTier,
    /// Landed cost at the item's default margin.
    ItemDefault,
    /// The price quoted on the BD request.
    Quoted,
    /// Typed in on the booking.
    Custom,
}

impl PriceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PriceSource::Manual => "manual",
            PriceSource::ClientTier => "client_tier",
            PriceSource::ItemDefault => "item_default",
            PriceSource::Quoted => "quoted",
            PriceSource::Custom => "custom",
        }
    }

    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("manual") => PriceSource::Manual,
            Some("client_tier") => PriceSource::ClientTier,
            Some("item_default") => PriceSource::ItemDefault,
            Some("quoted") => PriceSource::Quoted,
            _ => PriceSource::Custom,
        }
    }

    fn from_rule(rule: PriceRule) -> Self {
        match rule {
            PriceRule::Manual => PriceSource::Manual,
            PriceRule::ClientTier => PriceSource::ClientTier,
            PriceRule::ItemDefault => PriceSource::ItemDefault,
            PriceRule::None => PriceSource::Custom,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BookingLineInput {
    /// The line being edited; `None` for a new line.
    pub id: Option<String>,
    pub item_id: String,
    pub qty: i64,
    /// Net unit price. `None` uses the price resolution.
    pub price_c: Option<i64>,
    pub notes: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BookingInput {
    /// `None` creates a draft booking.
    pub id: Option<String>,
    pub client_id: String,
    pub date: String,
    pub vat: bool,
    /// The client's own PO or reference number.
    pub client_ref: String,
    pub deliver_to: String,
    pub notes: String,
    pub lines: Vec<BookingLineInput>,
}

/// How much of a booking has been delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum DeliveryProgress {
    None,
    Partial,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BookingInfo {
    pub id: String,
    pub no: Option<String>,
    /// False while the number was given offline: it may still change.
    pub no_confirmed: bool,
    pub client_id: Option<String>,
    pub client_code: Option<String>,
    pub client_name: Option<String>,
    pub bd_request_id: Option<String>,
    pub bd_no: Option<String>,
    pub date: String,
    pub status: BookingStatus,
    pub vat: bool,
    /// VAT rate when the booking was last saved.
    pub vat_bp: i64,
    pub client_ref: Option<String>,
    pub deliver_to: Option<String>,
    pub notes: Option<String>,
    pub totals: VatAmounts,
    pub summary: String,
    pub item_ids: Vec<String>,
    pub line_count: i64,
    pub progress: DeliveryProgress,
    /// DRs requested or approved, not yet delivered.
    pub open_drs: i64,
    pub created_at: i64,
    pub created_by_name: Option<String>,
    pub confirmed_at: Option<i64>,
    pub confirmed_by_name: Option<String>,
    pub closed_at: Option<i64>,
    pub closed_by_name: Option<String>,
    pub cancel_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BookingLine {
    pub id: String,
    pub item_id: String,
    pub item_code: Option<String>,
    pub item_name: Option<String>,
    pub unit: Option<String>,
    pub tracks_expiry: bool,
    pub qty: i64,
    pub price_c: i64,
    pub price_rule: PriceSource,
    pub notes: Option<String>,
    pub amount_c: i64,
    /// On DRs requested or approved, not yet delivered.
    pub pending: i64,
    pub delivered: i64,
    /// Not on any DR yet (negative if DRs from offline PCs overshoot).
    pub remaining: i64,
    /// Stripped without the costs permission.
    pub cost_c: Option<i64>,
    /// Stripped without the margins permission.
    pub margin_bp: Option<i64>,
    pub below_cost: bool,
    /// The item's box size, for items in boxes.
    pub pcs_per_box: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BookingDetail {
    pub booking: BookingInfo,
    pub lines: Vec<BookingLine>,
    pub drs: Vec<DrInfo>,
}

impl BookingDetail {
    pub fn strip(&mut self, perms: &Perms) {
        for l in &mut self.lines {
            if !perms.sees(Field::Costs) {
                l.cost_c = None;
            }
            if !perms.sees(Field::Margins) {
                l.margin_bp = None;
            }
        }
    }
}

/// A status change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum BookingAction {
    /// draft → confirmed: DRs can be requested.
    Confirm,
    /// confirmed → draft, to edit lines. Only without DRs.
    BackToDraft,
    /// confirmed → closed: nothing more will be delivered.
    Close,
    /// closed → confirmed.
    Reopen,
    /// draft or confirmed → cancelled. Only without DRs.
    Cancel,
}

// Writing -------------------------------------------------------------------

struct PreparedLine {
    id: Option<String>,
    item_id: String,
    qty: i64,
    price_c: i64,
    rule: PriceSource,
    notes: Option<String>,
}

fn live_client(conn: &Connection, id: &str) -> Result<()> {
    let found: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM clients WHERE id = ?1 AND _deleted = 0)",
        [id],
        |r| r.get(0),
    )?;
    if found {
        Ok(())
    } else {
        Err(Error::Invalid("choose a client".into()))
    }
}

/// Checks a booking and resolves its prices, without writing.
/// `existing`: price and rule of the booking's current lines, by id, so an
/// unchanged price keeps its rule. `quoted[i]`: line i's price is the BD
/// quote.
fn prepare(
    conn: &Connection,
    input: &BookingInput,
    existing: &HashMap<String, (i64, PriceSource)>,
    quoted: &[bool],
) -> Result<(String, Vec<PreparedLine>)> {
    live_client(conn, &input.client_id)?;
    let date = require_date(&input.date, "booking date")?;
    if input.lines.is_empty() {
        return Err(Error::Invalid("add at least one item".into()));
    }
    if input.lines.len() > MAX_LINES {
        return Err(Error::Invalid(format!(
            "a booking can have at most {MAX_LINES} lines"
        )));
    }
    let mut lines = Vec::with_capacity(input.lines.len());
    for (i, line) in input.lines.iter().enumerate() {
        let code: String = conn
            .query_row(
                "SELECT COALESCE(code, '') FROM items WHERE id = ?1 AND _deleted = 0",
                [&line.item_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::Invalid("choose an item on every line".into()))?;
        if line.qty <= 0 {
            return Err(Error::Invalid(format!("enter a quantity for {code}")));
        }
        let quote = pricing::quote(conn, &input.client_id, &line.item_id)?;
        let kept = line.id.as_ref().and_then(|id| existing.get(id));
        let (price_c, rule) = match line.price_c {
            Some(p) if p < 0 => {
                return Err(Error::Invalid(format!(
                    "the price of {code} can't be negative"
                )));
            }
            Some(p) if quoted.get(i) == Some(&true) => (p, PriceSource::Quoted),
            Some(p) => match kept {
                Some((old, rule)) if *old == p => (p, *rule),
                _ if quote.net_c == Some(p) => (p, PriceSource::from_rule(quote.rule)),
                _ => (p, PriceSource::Custom),
            },
            None => match quote.net_c {
                Some(n) => (n, PriceSource::from_rule(quote.rule)),
                None => {
                    return Err(Error::Invalid(format!(
                        "enter a price for {code}: it has no client rate or landed cost"
                    )));
                }
            },
        };
        lines.push(PreparedLine {
            id: line.id.clone(),
            item_id: line.item_id.clone(),
            qty: line.qty,
            price_c,
            rule,
            notes: text_opt(&line.notes),
        });
    }
    Ok((date, lines))
}

/// Checks a new booking as [`create`] would, without writing. Call it before
/// reserving a number.
pub fn validate(conn: &Connection, input: &BookingInput) -> Result<()> {
    prepare(conn, input, &HashMap::new(), &[]).map(drop)
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
                    "item_id": l.item_id,
                    "qty": l.qty,
                    "price_c": l.price_c,
                    "price_rule": l.rule.as_str(),
                    "notes": l.notes,
                }),
            )
        })
        .collect()
}

/// Creates a draft booking with the number reserved for it (`res.ref_id` is
/// its id). Returns the id.
pub fn create(tx: &mut WriteTx, input: &BookingInput, res: &Reservation) -> Result<String> {
    insert(tx, input, res, None, &[])
}

/// Creates a draft booking, optionally from a BD request.
pub(crate) fn insert(
    tx: &mut WriteTx,
    input: &BookingInput,
    res: &Reservation,
    bd_request_id: Option<&str>,
    quoted: &[bool],
) -> Result<String> {
    if res.series != series::BOOKING {
        return Err(Error::Invalid("wrong number series for a booking".into()));
    }
    let (date, lines) = prepare(tx.conn(), input, &HashMap::new(), quoted)?;
    let vat_bp = settings::vat_rate_bp(tx.conn())?;
    let id = tx.insert(
        BOOKINGS,
        json!({
            "id": res.ref_id,
            "client_id": input.client_id,
            "bd_request_id": bd_request_id,
            "date": date,
            "status": BookingStatus::Draft.as_str(),
            "vat": input.vat,
            "vat_bp": vat_bp,
            "client_ref": text_opt(&input.client_ref),
            "deliver_to": text_opt(&input.deliver_to),
            "notes": text_opt(&input.notes),
        }),
    )?;
    replace_lines(tx, BOOKING_LINES, "booking_id", &id, line_rows(lines))?;
    numbering::assign(tx, res, BOOKINGS)?;
    Ok(id)
}

fn status_of(tx: &WriteTx, id: &str) -> Result<BookingStatus> {
    let row = tx
        .get(BOOKINGS, id)?
        .ok_or_else(|| Error::Invalid("that booking no longer exists".into()))?;
    Ok(BookingStatus::parse(row["status"].as_str()))
}

/// Saves a booking's header and lines (any status but cancelled).
pub fn update(tx: &mut WriteTx, input: &BookingInput) -> Result<()> {
    let id = input.id.clone().unwrap_or_default();
    if status_of(tx, &id)? == BookingStatus::Cancelled {
        return Err(Error::Invalid("this booking is cancelled".into()));
    }
    let existing: HashMap<String, (i64, PriceSource)> = {
        let mut stmt = tx.conn().prepare(
            "SELECT id, COALESCE(price_c, 0), price_rule FROM booking_lines
             WHERE booking_id = ?1 AND _deleted = 0",
        )?;
        let rows = stmt.query_map([&id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (
                    r.get::<_, i64>(1)?,
                    PriceSource::parse(r.get::<_, Option<String>>(2)?.as_deref()),
                ),
            ))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let (date, lines) = prepare(tx.conn(), input, &existing, &[])?;
    let vat_bp = settings::vat_rate_bp(tx.conn())?;
    tx.update(
        BOOKINGS,
        &id,
        json!({
            "client_id": input.client_id,
            "date": date,
            "vat": input.vat,
            "vat_bp": vat_bp,
            "client_ref": text_opt(&input.client_ref),
            "deliver_to": text_opt(&input.deliver_to),
            "notes": text_opt(&input.notes),
        }),
    )?;
    // A line on a DR (not cancelled) has to stay: the DR points at it.
    let kept: Vec<&str> = lines.iter().filter_map(|l| l.id.as_deref()).collect();
    let on_drs: Vec<(String, String)> = {
        let mut stmt = tx.conn().prepare(
            "SELECT DISTINCT bl.id, COALESCE(i.code, '?') FROM booking_lines bl
             JOIN dr_lines l ON l.booking_line_id = bl.id AND l._deleted = 0
             JOIN delivery_receipts r ON r.id = l.dr_id AND r._deleted = 0
                                     AND r.status <> 'cancelled'
             LEFT JOIN items i ON i.id = bl.item_id
             WHERE bl.booking_id = ?1 AND bl._deleted = 0",
        )?;
        let rows = stmt.query_map([&id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    if let Some((_, code)) = on_drs.iter().find(|(l, _)| !kept.contains(&l.as_str())) {
        return Err(Error::Invalid(format!(
            "{code} is on a DR; remove it from the DR first"
        )));
    }
    replace_lines(tx, BOOKING_LINES, "booking_id", &id, line_rows(lines))
}

/// Changes a booking's status. `reason` is kept for cancellations.
pub fn set_status(tx: &mut WriteTx, id: &str, action: BookingAction, reason: &str) -> Result<()> {
    let status = status_of(tx, id)?;
    let (open, active) = delivery::dr_counts(tx.conn(), id)?;
    let (now, user) = (tx.now_ms(), tx.actor().user_id.clone());
    let patch = match (action, status) {
        (BookingAction::Confirm, BookingStatus::Draft) => json!({
            "status": BookingStatus::Confirmed.as_str(),
            "confirmed_at": now,
            "confirmed_by": user,
        }),
        (BookingAction::BackToDraft, BookingStatus::Confirmed) => {
            if active > 0 {
                return Err(Error::Invalid(
                    "this booking has DRs; cancel them before editing it".into(),
                ));
            }
            json!({ "status": BookingStatus::Draft.as_str() })
        }
        (BookingAction::Close, BookingStatus::Confirmed) => {
            if open > 0 {
                return Err(Error::Invalid(
                    "deliver or cancel this booking's open DRs first".into(),
                ));
            }
            json!({
                "status": BookingStatus::Closed.as_str(),
                "closed_at": now,
                "closed_by": user,
            })
        }
        (BookingAction::Reopen, BookingStatus::Closed) => json!({
            "status": BookingStatus::Confirmed.as_str(),
            "closed_at": null,
            "closed_by": null,
        }),
        (BookingAction::Cancel, BookingStatus::Draft | BookingStatus::Confirmed) => {
            if active > 0 {
                return Err(Error::Invalid(
                    "this booking has DRs; cancel them first, or close the booking".into(),
                ));
            }
            json!({
                "status": BookingStatus::Cancelled.as_str(),
                "closed_at": now,
                "closed_by": user,
                "cancel_reason": text_opt(reason),
            })
        }
        (_, s) => {
            return Err(Error::Invalid(format!(
                "a {} booking can't be changed that way",
                s.as_str()
            )));
        }
    };
    tx.update(BOOKINGS, id, patch).map(drop)
}

/// Deletes a draft booking and its lines. Its number is not given out again.
pub fn delete(tx: &mut WriteTx, id: &str) -> Result<()> {
    if status_of(tx, id)? != BookingStatus::Draft {
        return Err(Error::Invalid(
            "only a draft booking can be deleted; cancel it instead".into(),
        ));
    }
    for line in crate::domain::line_ids(tx, BOOKING_LINES, "booking_id", id)? {
        tx.delete(BOOKING_LINES, &line)?;
    }
    tx.delete(BOOKINGS, id)
}

// Reading -------------------------------------------------------------------

const SELECT: &str = "
    SELECT b.id, n.no, COALESCE(n.confirmed, 1), b.client_id, cn.no, c.name,
           b.bd_request_id, bn.no, b.date, b.status, COALESCE(b.vat, 0), COALESCE(b.vat_bp, 0),
           b.client_ref, b.deliver_to, b.notes, b.created_at, uc.display_name,
           b.confirmed_at, uf.display_name, b.closed_at, ul.display_name, b.cancel_reason
    FROM bookings b
    LEFT JOIN doc_numbers n ON n.ref_id = b.id AND n.series = 'BK' AND n._deleted = 0
    LEFT JOIN clients c ON c.id = b.client_id
    LEFT JOIN doc_numbers cn ON cn.ref_id = b.client_id AND cn.series = 'CL' AND cn._deleted = 0
    LEFT JOIN doc_numbers bn ON bn.ref_id = b.bd_request_id AND bn.series = 'BD'
                            AND bn._deleted = 0
    LEFT JOIN users uc ON uc.id = b.created_by
    LEFT JOIN users uf ON uf.id = b.confirmed_by
    LEFT JOIN users ul ON ul.id = b.closed_by
    WHERE b._deleted = 0";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<BookingInfo> {
    Ok(BookingInfo {
        id: r.get(0)?,
        no: r.get(1)?,
        no_confirmed: r.get::<_, i64>(2)? != 0,
        client_id: r.get(3)?,
        client_code: r.get(4)?,
        client_name: r.get(5)?,
        bd_request_id: r.get(6)?,
        bd_no: r.get(7)?,
        date: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
        status: BookingStatus::parse(r.get::<_, Option<String>>(9)?.as_deref()),
        vat: r.get::<_, i64>(10)? != 0,
        vat_bp: r.get(11)?,
        client_ref: r.get(12)?,
        deliver_to: r.get(13)?,
        notes: r.get(14)?,
        created_at: r.get(15)?,
        created_by_name: r.get(16)?,
        confirmed_at: r.get(17)?,
        confirmed_by_name: r.get(18)?,
        closed_at: r.get(19)?,
        closed_by_name: r.get(20)?,
        cancel_reason: r.get(21)?,
        totals: pricing::with_vat(0, 0),
        summary: String::new(),
        item_ids: Vec::new(),
        line_count: 0,
        progress: DeliveryProgress::None,
        open_drs: 0,
    })
}

/// Lines of one booking (or all), with what is on DRs, as (booking, line).
fn lines_of(conn: &Connection, booking: Option<&str>) -> Result<Vec<(String, BookingLine)>> {
    let progress = delivery::line_progress(conn, booking)?;
    let costs = items::landed_costs(conn)?;
    let mut stmt = conn.prepare_cached(
        "SELECT l.booking_id, l.id, l.item_id, i.code, i.name, i.unit,
                COALESCE(i.tracks_expiry, 0), COALESCE(l.qty, 0), COALESCE(l.price_c, 0),
                l.price_rule, l.notes,
                CASE WHEN COALESCE(i.serial_code, '') <> '' THEN i.pcs_per_box END
         FROM booking_lines l LEFT JOIN items i ON i.id = l.item_id
         WHERE l._deleted = 0 AND (?1 IS NULL OR l.booking_id = ?1)
         ORDER BY l.booking_id, l.line_no, l.id",
    )?;
    let rows = stmt.query_map([booking], |r| {
        let id: String = r.get(1)?;
        let item_id: String = r.get::<_, Option<String>>(2)?.unwrap_or_default();
        let qty: i64 = r.get(7)?;
        let price_c: i64 = r.get(8)?;
        let (pending, delivered) = progress.get(&id).copied().unwrap_or_default();
        let cost = costs.get(&item_id).map(|l| l.total_c);
        Ok((
            r.get(0)?,
            BookingLine {
                item_code: r.get(3)?,
                item_name: r.get(4)?,
                unit: r.get(5)?,
                tracks_expiry: r.get::<_, i64>(6)? != 0,
                qty,
                price_c,
                price_rule: PriceSource::parse(r.get::<_, Option<String>>(9)?.as_deref()),
                notes: r.get(10)?,
                amount_c: line_amount(qty, price_c),
                pending,
                delivered,
                remaining: qty - pending - delivered,
                cost_c: cost,
                margin_bp: cost.and_then(|c| pricing::margin_of(price_c, c)),
                below_cost: cost.is_some_and(|c| price_c < c),
                pcs_per_box: r.get(11)?,
                id,
                item_id,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn fill(b: &mut BookingInfo, lines: &[BookingLine], open_drs: i64) {
    let net = lines.iter().map(|l| l.amount_c).sum();
    b.totals = pricing::with_vat(net, if b.vat { b.vat_bp } else { 0 });
    b.summary = line_summary(lines.iter().map(|l| (l.item_code.as_deref(), l.qty)));
    b.item_ids = lines.iter().map(|l| l.item_id.clone()).collect();
    b.item_ids.sort();
    b.item_ids.dedup();
    b.line_count = lines.len() as i64;
    b.open_drs = open_drs;
    b.progress = if !lines.is_empty() && lines.iter().all(|l| l.delivered >= l.qty) {
        DeliveryProgress::Full
    } else if lines.iter().any(|l| l.delivered > 0) {
        DeliveryProgress::Partial
    } else {
        DeliveryProgress::None
    };
}

/// Every booking, newest first.
pub fn list(conn: &Connection) -> Result<Vec<BookingInfo>> {
    let mut by_booking: HashMap<String, Vec<BookingLine>> = HashMap::new();
    for (b, line) in lines_of(conn, None)? {
        by_booking.entry(b).or_default().push(line);
    }
    let open = delivery::open_counts(conn)?;
    let mut stmt = conn.prepare(&format!(
        "{SELECT} ORDER BY b.date DESC, b.created_at DESC, b.id DESC"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    let mut out = Vec::new();
    for row in rows {
        let mut b = row?;
        let lines = by_booking.remove(&b.id).unwrap_or_default();
        let open_drs = open.get(&b.id).copied().unwrap_or(0);
        fill(&mut b, &lines, open_drs);
        out.push(b);
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<BookingDetail>> {
    let Some(mut booking) = conn
        .query_row(&format!("{SELECT} AND b.id = ?1"), [id], from_row)
        .optional()?
    else {
        return Ok(None);
    };
    let lines: Vec<BookingLine> = lines_of(conn, Some(id))?
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    let drs = delivery::list_for_booking(conn, id)?;
    let open = drs.iter().filter(|d| d.status.is_open()).count() as i64;
    fill(&mut booking, &lines, open);
    Ok(Some(BookingDetail {
        booking,
        lines,
        drs,
    }))
}
