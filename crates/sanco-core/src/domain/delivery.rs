//! Delivery receipts (DRs) and invoices: the client's order and its delivery.
//!
//! Every customer order starts here: a receipt is entered with its client,
//! items, quantities and prices (New), then marked delivered. There is no
//! approval step and no Cancelled status: a receipt that isn't wanted is
//! deleted, in any status. Delivering takes the stock out (`v_stock_moves`,
//! kind `delivery`) and creates a receivable. A delivered receipt can also be
//! voided (it stays in the list as Void and can be restored), which puts the
//! stock back.
//!
//! A receipt can be edited in any status, also after delivery or when void.
//! Only integrity checks stay: stock can't go below zero and a delivered
//! receipt's total can't go below what is paid on it.
//!
//! A receipt is a DR (no VAT) or an Invoice: [`ReceiptKind`], stored as
//! `vat`. The type can change at any time; the number then follows the new
//! booklet.
//!
//! The number is the BIR booklet number: series DR for DRs, SI for
//! invoices. Staff type it (required to deliver) unless the series is set to
//! automatic (`numbering::is_manual`), in which case it is given when the
//! receipt is entered. Invoices made before M12 keep their DR number
//! ([`number_series`]); switching the type frees the number.
//!
//! Older receipts were requested from a booking (`booking_id`) and had an
//! approval step; their status `requested` / `approved` reads as New.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::auth::{Field, Perms};
use crate::domain::inventory::{self, Effect};
use crate::domain::lots;
use crate::domain::payments::{self, DrPayment};
use crate::domain::pricing::{self, PriceSource, VatAmounts};
use crate::domain::receivables;
use crate::domain::{
    MAX_LINES, items, line_amount, line_summary, peso, replace_lines, require_date, text_opt,
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
    /// Entered, not delivered yet.
    New,
    Delivered,
    /// Was delivered, then voided: its stock came back.
    Void,
}

impl DrStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            DrStatus::New => "new",
            DrStatus::Delivered => "delivered",
            DrStatus::Void => "void",
        }
    }

    /// `requested` and `approved` are the older two steps of New. Before
    /// 1.8 a void receipt was stored as `cancelled` (with `delivered_at`);
    /// cancelled ones that were never delivered are deleted at start
    /// ([`delete_cancelled`]).
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("delivered") => DrStatus::Delivered,
            Some("void" | "cancelled") => DrStatus::Void,
            _ => DrStatus::New,
        }
    }

    /// Not delivered yet.
    pub fn is_open(self) -> bool {
        self == DrStatus::New
    }
}

/// What the client gets: a DR (no VAT) or an Invoice (VAT).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ReceiptKind {
    Dr,
    Invoice,
}

impl ReceiptKind {
    pub fn from_vat(vat: bool) -> Self {
        if vat {
            ReceiptKind::Invoice
        } else {
            ReceiptKind::Dr
        }
    }

    pub fn vat(self) -> bool {
        self == ReceiptKind::Invoice
    }

    /// The number series of new numbers.
    pub fn series(self) -> &'static str {
        match self {
            ReceiptKind::Dr => series::DR,
            ReceiptKind::Invoice => series::INVOICE,
        }
    }

    /// "DR" or "invoice", for messages.
    pub fn name(self) -> &'static str {
        match self {
            ReceiptKind::Dr => "DR",
            ReceiptKind::Invoice => "invoice",
        }
    }

    /// "DR 123", "Invoice 456", or "DR (no number)".
    pub fn label(self, no: Option<&str>) -> String {
        let title = match self {
            ReceiptKind::Dr => "DR",
            ReceiptKind::Invoice => "Invoice",
        };
        match no {
            Some(n) => format!("{title} {n}"),
            None => format!("{title} (no number)"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DrLineInput {
    /// The line being edited; `None` for a new line.
    pub id: Option<String>,
    pub item_id: String,
    pub qty: i64,
    /// Unit price. `None` uses the price resolution.
    pub price_c: Option<i64>,
    /// The lot it comes from. Needed to deliver.
    pub lot_id: Option<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DrInput {
    pub client_id: String,
    pub kind: ReceiptKind,
    /// Delivery date (planned until it is delivered).
    pub date: String,
    pub from_location_id: String,
    /// The client's own PO or reference number.
    pub client_ref: String,
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
    /// DR (no VAT) or Invoice (VAT, same as `vat`).
    pub kind: ReceiptKind,
    /// The booklet number; `None` until entered.
    pub no: Option<String>,
    /// Series of the number (or of the number it will get): DR or SI.
    pub no_series: String,
    /// False if other PCs were offline when the number was entered.
    pub no_confirmed: bool,
    /// Typed by staff.
    pub no_manual: bool,
    /// Another document has the same number (entered while offline).
    pub no_clash: bool,
    /// The booking an older receipt was requested from (a removed step).
    pub booking_id: Option<String>,
    pub booking_no: Option<String>,
    pub client_id: Option<String>,
    pub client_code: Option<String>,
    pub client_name: Option<String>,
    /// The client's own PO or reference.
    pub client_ref: Option<String>,
    pub date: String,
    pub from_location_id: Option<String>,
    pub from_location_name: Option<String>,
    pub status: DrStatus,
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
    pub delivered_date: Option<String>,
    pub delivered_at: Option<i64>,
    pub delivered_by_name: Option<String>,
    pub received_by: Option<String>,
    pub voided_at: Option<i64>,
    pub voided_by_name: Option<String>,
    pub void_reason: Option<String>,
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
    pub lot_id: Option<String>,
    /// The lot's arrival month, `YYYY-MM`.
    pub month: Option<String>,
    pub expiry: Option<String>,
    pub serial_from: Option<String>,
    pub serial_to: Option<String>,
    pub qty: i64,
    pub price_c: i64,
    pub price_rule: PriceSource,
    pub notes: Option<String>,
    pub amount_c: i64,
    /// Stripped without the costs permission. Only in [`get`].
    pub cost_c: Option<i64>,
    /// Stripped without the margins permission. Only in [`get`].
    pub margin_bp: Option<i64>,
    pub below_cost: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DrDetail {
    pub dr: DrInfo,
    pub lines: Vec<DrLine>,
    /// Its numbers are typed by staff (else given when it is entered).
    pub manual_numbers: bool,
    /// Payments allocated to it.
    pub payments: Vec<DrPayment>,
}

impl DrDetail {
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

/// How a receipt's number is handled when it is saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberPlan {
    /// The series the number is (or will be) in.
    pub series: &'static str,
    /// Staff type it.
    pub manual: bool,
    /// Its number in that series now.
    pub current: Option<String>,
}

/// The number series a receipt of `kind` uses, and what it has there now.
/// `id`: the receipt being edited, `None` for a new one.
pub fn number_plan(conn: &Connection, id: Option<&str>, kind: ReceiptKind) -> Result<NumberPlan> {
    let series = match id {
        Some(id) if kind_of(conn, id)? == kind => number_series(conn, id)?,
        _ => kind.series(),
    };
    let current = match id {
        Some(id) => numbering::number_of(conn, series, id)?,
        None => None,
    };
    Ok(NumberPlan {
        series,
        manual: numbering::is_manual(conn, series)?,
        current,
    })
}

// Writing -------------------------------------------------------------------

struct Prepared {
    kind: ReceiptKind,
    date: String,
    lines: Vec<PreparedLine>,
}

struct PreparedLine {
    id: Option<String>,
    item_id: String,
    lot_id: Option<String>,
    qty: i64,
    price_c: i64,
    rule: PriceSource,
    notes: Option<String>,
}

/// The receipt being edited, as it is now.
struct Current {
    status: DrStatus,
    kind: ReceiptKind,
    location: String,
    /// It was delivered at some time (so it may be void now).
    was_delivered: bool,
    lines: Vec<DrLine>,
}

/// Lines of a DR (or all), as (dr, line).
fn lines_of(conn: &Connection, dr: Option<&str>) -> Result<Vec<(String, DrLine)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT l.dr_id, l.id, l.booking_line_id, l.item_id, i.code, i.name, i.unit, l.lot_id,
                COALESCE(l.qty, 0), COALESCE(l.price_c, 0), t.month, t.expiry, t.serial_from,
                t.serial_to, COALESCE(l.price_rule, bl.price_rule), l.notes
         FROM dr_lines l LEFT JOIN items i ON i.id = l.item_id
         LEFT JOIN lots t ON t.id = l.lot_id
         LEFT JOIN booking_lines bl ON bl.id = l.booking_line_id
         WHERE l._deleted = 0 AND (?1 IS NULL OR l.dr_id = ?1)
         ORDER BY l.dr_id, l.line_no, l.id",
    )?;
    let rows = stmt.query_map([dr], |r| {
        let (qty, price_c): (i64, i64) = (r.get(8)?, r.get(9)?);
        Ok((
            r.get(0)?,
            DrLine {
                id: r.get(1)?,
                booking_line_id: r.get(2)?,
                item_id: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                item_code: r.get(4)?,
                item_name: r.get(5)?,
                unit: r.get(6)?,
                lot_id: r.get(7)?,
                month: r.get(10)?,
                expiry: r.get(11)?,
                serial_from: r.get(12)?,
                serial_to: r.get(13)?,
                qty,
                price_c,
                price_rule: PriceSource::parse(r.get::<_, Option<String>>(14)?.as_deref()),
                notes: r.get(15)?,
                amount_c: line_amount(qty, price_c),
                cost_c: None,
                margin_bp: None,
                below_cost: false,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// What the receipt's lines take out of stock, per lot, as negative effects.
fn effects_of(lines: &[DrLine], location: &str) -> Vec<Effect> {
    let mut lots: BTreeMap<(String, Option<String>), i64> = BTreeMap::new();
    for l in lines {
        *lots
            .entry((l.item_id.clone(), l.lot_id.clone()))
            .or_default() -= l.qty;
    }
    lots.into_iter()
        .map(|((item_id, lot_id), qty)| Effect {
            item_id,
            location_id: location.to_string(),
            lot_id,
            qty,
        })
        .collect()
}

/// Every line needs its lot before the stock can go out.
fn require_lots(lines: &[DrLine]) -> Result<()> {
    if let Some(l) = lines.iter().find(|l| l.lot_id.is_none()) {
        return Err(Error::Invalid(format!(
            "choose the lot of {}",
            l.item_code.as_deref().unwrap_or("every item")
        )));
    }
    Ok(())
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

/// Checks a receipt and resolves its prices, without writing. `current`: the
/// receipt being edited.
fn prepare(conn: &Connection, input: &DrInput, current: Option<&Current>) -> Result<Prepared> {
    live_client(conn, &input.client_id)?;
    let date = require_date(&input.date, "delivery date")?;
    check_location(conn, &input.from_location_id)?;
    if input.lines.is_empty() {
        return Err(Error::Invalid("add at least one item".into()));
    }
    if input.lines.len() > MAX_LINES {
        return Err(Error::Invalid(format!(
            "a DR or invoice can have at most {MAX_LINES} lines"
        )));
    }
    let status = current.map_or(DrStatus::New, |c| c.status);
    let delivered = status == DrStatus::Delivered;
    let kept_lines: HashMap<&str, &DrLine> = current
        .map(|c| c.lines.iter().map(|l| (l.id.as_str(), l)).collect())
        .unwrap_or_default();

    type Lot = (String, String, Option<String>);
    let mut stock: BTreeMap<Lot, i64> = BTreeMap::new();
    if let Some(cur) = current
        && delivered
    {
        // Delivered stock comes back before the new lines take theirs.
        for l in &cur.lines {
            *stock
                .entry((l.item_id.clone(), cur.location.clone(), l.lot_id.clone()))
                .or_default() += l.qty;
        }
    }
    let mut lines = Vec::with_capacity(input.lines.len());
    for line in &input.lines {
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
        let lot_id = line
            .lot_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        match &lot_id {
            Some(lot) if lots::expiry_of(conn, lot, &line.item_id)?.is_none() => {
                return Err(Error::Invalid(format!("that lot doesn't hold {code}")));
            }
            None if delivered => {
                return Err(Error::Invalid(format!("choose the lot of {code}")));
            }
            _ => {}
        }
        let quote = pricing::quote(conn, &input.client_id, &line.item_id)?;
        let kept = line
            .id
            .as_deref()
            .and_then(|id| kept_lines.get(id))
            .filter(|l| l.item_id == line.item_id);
        let (price_c, rule) = match line.price_c {
            Some(p) if p < 0 => {
                return Err(Error::Invalid(format!(
                    "the price of {code} can't be negative"
                )));
            }
            Some(p) => match kept {
                Some(old) if old.price_c == p => (p, old.price_rule),
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
        if delivered {
            *stock
                .entry((
                    line.item_id.clone(),
                    input.from_location_id.clone(),
                    lot_id.clone(),
                ))
                .or_default() -= line.qty;
        }
        lines.push(PreparedLine {
            id: kept.map(|l| l.id.clone()),
            item_id: line.item_id.clone(),
            lot_id,
            qty: line.qty,
            price_c,
            rule,
            notes: text_opt(&line.notes),
        });
    }
    if delivered {
        let effects: Vec<Effect> = stock
            .into_iter()
            .map(|((item_id, location_id, lot_id), qty)| Effect {
                item_id,
                location_id,
                lot_id,
                qty,
            })
            .collect();
        // Only lots the receipt takes more from than before are checked.
        inventory::check_not_negative(conn, &effects, "")?;
    }
    Ok(Prepared {
        kind: input.kind,
        date,
        lines,
    })
}

/// Checks a new receipt as [`create`] would, without writing. Call it before
/// reserving a number.
pub fn validate(conn: &Connection, input: &DrInput) -> Result<()> {
    prepare(conn, input, None).map(drop)
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
                    "lot_id": l.lot_id,
                    "qty": l.qty,
                    "price_c": l.price_c,
                    "price_rule": l.rule.as_str(),
                    "notes": l.notes,
                }),
            )
        })
        .collect()
}

fn check_reservation(conn: &Connection, id: &str, res: &Reservation) -> Result<()> {
    if res.ref_id != id || res.series != number_series(conn, id)? {
        return Err(Error::Invalid(
            "wrong number for this receipt; reload it".into(),
        ));
    }
    Ok(())
}

/// Enters a new receipt as `id` (made with `Db::new_id`). `res` is its
/// number, if it gets one now.
pub fn create(
    tx: &mut WriteTx,
    id: &str,
    input: &DrInput,
    res: Option<&Reservation>,
) -> Result<String> {
    let p = prepare(tx.conn(), input, None)?;
    let vat_bp = pricing::DOC_VAT_BP;
    let id = tx.insert(
        DELIVERY_RECEIPTS,
        json!({
            "id": id,
            "client_id": input.client_id,
            "date": p.date,
            "from_location_id": input.from_location_id,
            "status": DrStatus::New.as_str(),
            "vat": p.kind.vat(),
            "vat_bp": vat_bp,
            "client_ref": text_opt(&input.client_ref),
            "deliver_to": text_opt(&input.deliver_to),
            "notes": text_opt(&input.notes),
        }),
    )?;
    replace_lines(tx, DR_LINES, "dr_id", &id, line_rows(p.lines))?;
    if let Some(res) = res {
        check_reservation(tx.conn(), &id, res)?;
        numbering::assign_or_change(tx, res, DELIVERY_RECEIPTS)?;
    }
    Ok(id)
}

fn current(conn: &Connection, id: &str) -> Result<Current> {
    let (status, location) = status_of(conn, id)?;
    let kind = kind_of(conn, id)?;
    let delivered_at: Option<i64> = conn.query_row(
        "SELECT delivered_at FROM delivery_receipts WHERE id = ?1",
        [id],
        |r| r.get(0),
    )?;
    Ok(Current {
        status,
        kind,
        location,
        was_delivered: delivered_at.is_some(),
        lines: lines_of(conn, Some(id))?
            .into_iter()
            .map(|(_, l)| l)
            .collect(),
    })
}

/// Checks an edit as [`update`] would, without writing. `today` is Manila's.
/// `numbered`: the caller will give the receipt a number in the same save.
pub fn check_update(
    conn: &Connection,
    id: &str,
    input: &DrInput,
    delivery: Option<&DeliverInput>,
    numbered: bool,
    today: &str,
) -> Result<()> {
    let cur = current(conn, id)?;
    if cur.was_delivered
        && let Some(d) = delivery
    {
        let date = require_date(&d.date, "delivery date")?;
        if date.as_str() > today {
            return Err(Error::Invalid(
                "the delivery date can't be in the future".into(),
            ));
        }
    }
    let p = prepare(conn, input, Some(&cur))?;
    if cur.status == DrStatus::Delivered {
        let plan = number_plan(conn, Some(id), p.kind)?;
        if plan.current.is_none() && !numbered {
            return Err(Error::Invalid(format!(
                "enter the {} number first",
                p.kind.name()
            )));
        }
        let paid = receivables::dr_paid(conn, id)?;
        if paid > 0 {
            let vat_bp: i64 = conn.query_row(
                "SELECT COALESCE(vat_bp, 0) FROM delivery_receipts WHERE id = ?1",
                [id],
                |r| r.get(0),
            )?;
            let net = p.lines.iter().map(|l| line_amount(l.qty, l.price_c)).sum();
            let gross = pricing::with_vat(net, if p.kind.vat() { vat_bp } else { 0 }).gross_c;
            if gross < paid {
                return Err(Error::Invalid(format!(
                    "payments of {} are applied to this {}; its total can't go below that ({} now). Change the payments first",
                    peso(paid),
                    cur.kind.name(),
                    peso(gross)
                )));
            }
        }
    }
    Ok(())
}

/// Edits a receipt in any status: client, type, date, location, address,
/// notes and lines, and for a delivered one the delivery date and who
/// received it. Stock moves by the difference. `res` gives or corrects its
/// number.
pub fn update(
    tx: &mut WriteTx,
    id: &str,
    input: &DrInput,
    delivery: Option<&DeliverInput>,
    res: Option<&Reservation>,
) -> Result<()> {
    let today = manila_date_string(tx.now_ms());
    check_update(tx.conn(), id, input, delivery, res.is_some(), &today)?;
    let cur = current(tx.conn(), id)?;
    let p = prepare(tx.conn(), input, Some(&cur))?;
    let mut patch = json!({
        "client_id": input.client_id,
        "date": p.date,
        "from_location_id": input.from_location_id,
        "client_ref": text_opt(&input.client_ref),
        "deliver_to": text_opt(&input.deliver_to),
        "notes": text_opt(&input.notes),
    });
    if cur.was_delivered
        && let Some(d) = delivery
    {
        patch["delivered_date"] = json!(require_date(&d.date, "delivery date")?);
        patch["received_by"] = json!(text_opt(&d.received_by));
    }
    if p.kind != cur.kind {
        // The number was from the other booklet: free it (an Invoice made
        // before M12 that becomes a DR keeps its DR number).
        let series = number_series(tx.conn(), id)?;
        if series != p.kind.series() {
            numbering::release(tx, id, series)?;
        }
        patch["vat"] = json!(p.kind.vat());
        patch["vat_bp"] = json!(pricing::DOC_VAT_BP);
    }
    tx.update(DELIVERY_RECEIPTS, id, patch)?;
    replace_lines(tx, DR_LINES, "dr_id", id, line_rows(p.lines))?;
    if let Some(res) = res {
        check_reservation(tx.conn(), id, res)?;
        numbering::assign_or_change(tx, res, DELIVERY_RECEIPTS)?;
    }
    if cur.status == DrStatus::Delivered && number_of(tx.conn(), id)?.is_none() {
        return Err(Error::Invalid(format!(
            "enter the {} number first",
            p.kind.name()
        )));
    }
    Ok(())
}

/// A live receipt's type.
fn kind_of(conn: &Connection, id: &str) -> Result<ReceiptKind> {
    let vat: Option<i64> = conn
        .query_row(
            "SELECT COALESCE(vat, 0) FROM delivery_receipts WHERE id = ?1 AND _deleted = 0",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(ReceiptKind::from_vat(vat.unwrap_or(0) != 0))
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
        row.ok_or_else(|| Error::Invalid("that DR or invoice no longer exists".into()))?;
    Ok((
        DrStatus::parse(status.as_deref()),
        location.unwrap_or_default(),
    ))
}

/// The receipt's number, if it has one.
pub fn number_of(conn: &Connection, id: &str) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT no FROM v_receipt_no WHERE dr_id = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()?)
}

/// The series a receipt's number is (or will be) in: that of the number it
/// has, else that of its type. Invoices made before M12 have DR numbers.
pub fn number_series(conn: &Connection, id: &str) -> Result<&'static str> {
    let held: Option<String> = conn
        .prepare_cached("SELECT series FROM v_receipt_no WHERE dr_id = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()?;
    Ok(match held.as_deref() {
        Some(series::DR) => series::DR,
        Some(series::INVOICE) => series::INVOICE,
        _ => kind_of(conn, id)?.series(),
    })
}

/// Checks that the receipt can be marked delivered on `input.date`, without
/// writing. `today` is Manila's.
pub fn check_deliver(conn: &Connection, id: &str, input: &DeliverInput, today: &str) -> Result<()> {
    let (status, location) = status_of(conn, id)?;
    let kind = kind_of(conn, id)?.name();
    match status {
        DrStatus::New => {}
        DrStatus::Delivered => {
            return Err(Error::Invalid(format!("this {kind} is already delivered")));
        }
        DrStatus::Void => {
            return Err(Error::Invalid(format!(
                "this {kind} is void; restore it first"
            )));
        }
    }
    let date = require_date(&input.date, "delivery date")?;
    if date.as_str() > today {
        return Err(Error::Invalid(
            "the delivery date can't be in the future".into(),
        ));
    }
    let lines: Vec<DrLine> = lines_of(conn, Some(id))?
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    require_lots(&lines)?;
    inventory::check_not_negative(conn, &effects_of(&lines, &location), "")
}

/// Marks a new receipt delivered: its stock goes out. The receipt must have
/// a number by then; `res` gives or corrects it.
pub fn deliver(
    tx: &mut WriteTx,
    id: &str,
    input: &DeliverInput,
    res: Option<&Reservation>,
) -> Result<()> {
    let today = manila_date_string(tx.now_ms());
    check_deliver(tx.conn(), id, input, &today)?;
    if let Some(res) = res {
        check_reservation(tx.conn(), id, res)?;
        numbering::assign_or_change(tx, res, DELIVERY_RECEIPTS)?;
    }
    if number_of(tx.conn(), id)?.is_none() {
        return Err(Error::Invalid(format!(
            "enter the {} number first",
            kind_of(tx.conn(), id)?.name()
        )));
    }
    let vat_bp = pricing::DOC_VAT_BP;
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

/// Checks that the receipt's number can be set or corrected, without writing.
pub fn check_set_number(conn: &Connection, id: &str) -> Result<()> {
    status_of(conn, id).map(drop)
}

/// Sets or corrects a receipt's typed number.
pub fn set_number(tx: &mut WriteTx, id: &str, res: &Reservation) -> Result<()> {
    check_set_number(tx.conn(), id)?;
    check_reservation(tx.conn(), id, res)?;
    numbering::assign_or_change(tx, res, DELIVERY_RECEIPTS).map(drop)
}

/// Deletes a receipt in any status. A delivered one's stock comes back and
/// the payments applied to it become the client's credit. A typed number
/// can be used again.
pub fn delete(tx: &mut WriteTx, id: &str) -> Result<()> {
    status_of(tx.conn(), id)?;
    let allocations: Vec<String> = {
        let mut stmt = tx.conn().prepare(
            "SELECT id FROM payment_allocations
             WHERE dr_id = ?1 AND _deleted = 0 AND amount_c IS NOT NULL",
        )?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    for a in allocations {
        // Derived ids: cleared, never deleted.
        tx.update(
            payments::PAYMENT_ALLOCATIONS,
            &a,
            json!({ "amount_c": null }),
        )?;
    }
    for line in crate::domain::line_ids(tx, DR_LINES, "dr_id", id)? {
        tx.delete(DR_LINES, &line)?;
    }
    numbering::delete_for(tx, id)?;
    tx.delete(DELIVERY_RECEIPTS, id)
}

/// Deletes the receipts cancelled before 1.8 that were never delivered
/// (there is no Cancelled status any more). Runs at every start; PCs
/// deleting the same row at once is not a conflict.
pub fn delete_cancelled(tx: &mut WriteTx) -> Result<()> {
    let ids: Vec<String> = {
        let mut stmt = tx.conn().prepare(
            "SELECT id FROM delivery_receipts
             WHERE _deleted = 0 AND status = 'cancelled' AND delivered_at IS NULL",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    for id in ids {
        delete(tx, &id)?;
    }
    Ok(())
}

/// Voids a delivered receipt: its stock comes back. Needs a reason.
pub fn void(tx: &mut WriteTx, id: &str, reason: &str) -> Result<()> {
    let (status, _) = status_of(tx.conn(), id)?;
    let kind = kind_of(tx.conn(), id)?.name();
    if status != DrStatus::Delivered {
        return Err(Error::Invalid(format!(
            "only a delivered {kind} can be voided; delete it instead"
        )));
    }
    let paid = receivables::dr_paid(tx.conn(), id)?;
    if paid > 0 {
        return Err(Error::Invalid(format!(
            "payments of {} are applied to this {kind}; take them off the payments first",
            peso(paid)
        )));
    }
    let reason = text_opt(reason)
        .ok_or_else(|| Error::Invalid(format!("enter why the {kind} is voided")))?;
    let (now, user) = (tx.now_ms(), tx.actor().user_id.clone());
    tx.update(
        DELIVERY_RECEIPTS,
        id,
        json!({
            "status": DrStatus::Void.as_str(),
            "cancelled_at": now,
            "cancelled_by": user,
            "cancel_reason": reason,
        }),
    )
    .map(drop)
}

/// Checks that a void receipt can be restored, without writing.
pub fn check_restore(conn: &Connection, id: &str) -> Result<DrStatus> {
    let cur = current(conn, id)?;
    let kind = cur.kind.name();
    if cur.status != DrStatus::Void {
        return Err(Error::Invalid(format!("this {kind} isn't void")));
    }
    // It takes its stock out again.
    require_lots(&cur.lines)?;
    inventory::check_not_negative(conn, &effects_of(&cur.lines, &cur.location), "")?;
    Ok(DrStatus::Delivered)
}

/// Restores a void receipt: delivered again, its stock goes out again.
pub fn restore(tx: &mut WriteTx, id: &str) -> Result<()> {
    let status = check_restore(tx.conn(), id)?;
    tx.update(
        DELIVERY_RECEIPTS,
        id,
        json!({
            "status": status.as_str(),
            "cancelled_at": null,
            "cancelled_by": null,
            "cancel_reason": null,
        }),
    )
    .map(drop)
}

// Reading -------------------------------------------------------------------

const SELECT: &str = "
    SELECT r.id, n.no, COALESCE(n.confirmed, 1), COALESCE(n.manual, 0),
           EXISTS (SELECT 1 FROM doc_numbers o WHERE o._deleted = 0 AND o.series = n.series
                   AND o.period = n.period AND o.seq = n.seq AND o.id <> n.number_id),
           r.booking_id, bn.no, r.client_id, cn.no, c.name, r.date, r.from_location_id, l.name,
           r.status, COALESCE(r.vat, 0), COALESCE(r.vat_bp, 0), r.deliver_to, r.notes,
           r.created_at, uc.display_name, r.delivered_date,
           r.delivered_at, ud.display_name, r.received_by, r.cancelled_at, ux.display_name,
           r.cancel_reason, c.terms_days, n.series, r.client_ref
    FROM delivery_receipts r
    LEFT JOIN v_receipt_no n ON n.dr_id = r.id
    LEFT JOIN doc_numbers bn ON bn.ref_id = r.booking_id AND bn.series = 'BK' AND bn._deleted = 0
    LEFT JOIN clients c ON c.id = r.client_id
    LEFT JOIN doc_numbers cn ON cn.ref_id = r.client_id AND (cn.series = 'CL' OR cn.series LIKE 'CL.%') AND cn.seq IS NOT NULL AND cn._deleted = 0
    LEFT JOIN locations l ON l.id = r.from_location_id
    LEFT JOIN users uc ON uc.id = r.created_by
    LEFT JOIN users ud ON ud.id = r.delivered_by
    LEFT JOIN users ux ON ux.id = r.cancelled_by
    WHERE r._deleted = 0";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<DrInfo> {
    let status = DrStatus::parse(r.get::<_, Option<String>>(13)?.as_deref());
    let delivered_at: Option<i64> = r.get(21)?;
    let kind = ReceiptKind::from_vat(r.get::<_, i64>(14)? != 0);
    Ok(DrInfo {
        id: r.get(0)?,
        kind,
        no: r.get(1)?,
        no_series: r
            .get::<_, Option<String>>(28)?
            .unwrap_or_else(|| kind.series().to_string()),
        no_confirmed: r.get::<_, i64>(2)? != 0,
        no_manual: r.get::<_, i64>(3)? != 0,
        no_clash: r.get(4)?,
        booking_id: r.get(5)?,
        booking_no: r.get(6)?,
        client_id: r.get(7)?,
        client_code: r.get(8)?,
        client_name: r.get(9)?,
        client_ref: r.get(29)?,
        date: r.get::<_, Option<String>>(10)?.unwrap_or_default(),
        from_location_id: r.get(11)?,
        from_location_name: r.get(12)?,
        status,
        vat: r.get::<_, i64>(14)? != 0,
        vat_bp: r.get(15)?,
        deliver_to: r.get(16)?,
        notes: r.get(17)?,
        created_at: r.get(18)?,
        created_by_name: r.get(19)?,
        delivered_date: r.get(20)?,
        delivered_at,
        delivered_by_name: r.get(22)?,
        received_by: r.get(23)?,
        voided_at: r.get(24)?,
        voided_by_name: r.get(25)?,
        void_reason: r.get(26)?,
        paid_c: 0,
        balance_c: 0,
        due_date: receivables::due_date(r.get::<_, Option<String>>(20)?.as_deref(), r.get(27)?),
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

/// Every receipt, newest first.
pub fn list(conn: &Connection) -> Result<Vec<DrInfo>> {
    let mut by_dr: HashMap<String, Vec<DrLine>> = HashMap::new();
    for (dr, line) in lines_of(conn, None)? {
        by_dr.entry(dr).or_default().push(line);
    }
    let paid = receivables::paid_by_dr(conn)?;
    let mut stmt = conn.prepare(&format!(
        "{SELECT} ORDER BY COALESCE(r.delivered_date, r.date) DESC, r.created_at DESC, r.id DESC"
    ))?;
    let rows = stmt.query_map([], from_row)?;
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

pub fn get(conn: &Connection, id: &str) -> Result<Option<DrDetail>> {
    let Some(mut dr) = conn
        .query_row(&format!("{SELECT} AND r.id = ?1"), [id], from_row)
        .optional()?
    else {
        return Ok(None);
    };
    let mut lines: Vec<DrLine> = lines_of(conn, Some(id))?
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    let costs = items::landed_costs(conn)?;
    for l in &mut lines {
        let cost = costs.get(&l.item_id).map(|c| c.total_c);
        l.cost_c = cost;
        l.margin_bp = cost.and_then(|c| pricing::margin_of(l.price_c, c));
        l.below_cost = cost.is_some_and(|c| l.price_c < c);
    }
    fill(&mut dr, &lines, receivables::dr_paid(conn, id)?);
    let manual_numbers = numbering::is_manual(conn, &dr.no_series)?;
    Ok(Some(DrDetail {
        dr,
        lines,
        manual_numbers,
        payments: payments::for_dr(conn, id)?,
    }))
}
