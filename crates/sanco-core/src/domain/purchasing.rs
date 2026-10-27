//! Purchase orders to suppliers, their TT payments and receiving.
//!
//! draft → ordered → shipped → arrived → closed, or cancelled. A PO
//! (header and lines) stays editable in any status but cancelled; a line
//! that has been received can't be removed, change item or go below what
//! was received.
//!
//! Goods come in through receiving reports (series RR), each into one
//! location, with an expiry per line for items that track one. A report is
//! posted when saved and never edited: a mistake is voided and entered
//! again, like a stock document. How much of a PO was received is computed
//! from the posted reports (`v_po_received`), never stored.
//!
//! TT payments are plain rows on the PO, in any currency with their own FX.
//! Amounts are minor units of the currency; FX is PHP per unit, as text.
//! Prices, totals, FX and payments are cost data: callers strip them for
//! users without [`Field::Costs`], and saves from such users keep the
//! stored prices.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::auth::{Field, Perms};
use crate::domain::bank_accounts;
use crate::domain::boxes::{self, LineSpec, PoBox, SerialNeed, Serials};
use crate::domain::inventory::{self, Effect};
use crate::domain::items::{self, ITEM_COSTS};
use crate::domain::pricing::{self, check_currency};
use crate::domain::{
    MAX_LINES, date_opt, line_amount, line_summary, replace_lines, require_date, text_opt,
};
use crate::error::{Error, Result};
use crate::repo::WriteTx;
use crate::sync::numbering::{self, Reservation, series};
use crate::time::{manila_date_string, parse_date};

pub const PURCHASE_ORDERS: &str = "purchase_orders";
pub const PO_LINES: &str = "po_lines";
pub const SUPPLIER_PAYMENTS: &str = "supplier_payments";
pub const PO_RECEIPTS: &str = "po_receipts";
pub const PO_RECEIPT_LINES: &str = "po_receipt_lines";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum PoStatus {
    Draft,
    Ordered,
    Shipped,
    Arrived,
    Closed,
    Cancelled,
}

impl PoStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PoStatus::Draft => "draft",
            PoStatus::Ordered => "ordered",
            PoStatus::Shipped => "shipped",
            PoStatus::Arrived => "arrived",
            PoStatus::Closed => "closed",
            PoStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("ordered") => PoStatus::Ordered,
            Some("shipped") => PoStatus::Shipped,
            Some("arrived") => PoStatus::Arrived,
            Some("closed") => PoStatus::Closed,
            Some("cancelled") => PoStatus::Cancelled,
            _ => PoStatus::Draft,
        }
    }

    /// Ordered and not yet closed or cancelled: goods can be received.
    pub fn is_open(self) -> bool {
        matches!(
            self,
            PoStatus::Ordered | PoStatus::Shipped | PoStatus::Arrived
        )
    }
}

/// A status change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum PoAction {
    /// draft → ordered: sent to the supplier.
    Order,
    /// ordered → draft. Only before anything was received.
    BackToDraft,
    /// ordered → shipped, on a date.
    Ship,
    /// ordered or shipped → arrived, on a date.
    Arrive,
    /// ordered, shipped or arrived → closed: nothing more will come.
    Close,
    /// closed → back to where it was.
    Reopen,
    /// → cancelled. Only before anything was received.
    Cancel,
}

/// How much of a PO has been received.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ReceiveProgress {
    None,
    Partial,
    Full,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PoLineInput {
    /// The line being edited; `None` for a new line.
    pub id: Option<String>,
    pub item_id: String,
    pub qty: i64,
    /// Unit price in the PO's currency (minor units). `None` = not known
    /// yet. Ignored for users who can't see costs.
    pub price_minor: Option<i64>,
    pub notes: String,
    /// Pieces per box, for items that come in boxes; `None` = the item's.
    pub pcs_per_box: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PoInput {
    /// `None` creates a draft PO.
    pub id: Option<String>,
    pub supplier_id: String,
    pub date: String,
    /// Ignored (kept, or the supplier's) for users who can't see costs.
    pub currency: String,
    /// PHP per unit of `currency`; blank if not known yet. Ignored for PHP
    /// and for users who can't see costs.
    pub fx_rate: String,
    /// Expected arrival, `YYYY-MM-DD` or blank.
    pub expected_date: String,
    /// The supplier's proforma invoice or order number.
    pub supplier_ref: String,
    pub notes: String,
    /// Year printed in the box serials (`YYYY`); blank = this year. Fixed
    /// once the PO has boxes.
    pub serial_year: String,
    pub lines: Vec<PoLineInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PoStatusInput {
    pub action: PoAction,
    /// Shipped or arrival date for `ship`/`arrive`; blank = today.
    pub date: String,
    /// Why a PO is cancelled.
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct TtInput {
    /// `None` adds a payment.
    pub id: Option<String>,
    pub po_id: String,
    pub date: String,
    pub currency: String,
    pub amount_minor: i64,
    /// PHP per unit of `currency`. Required unless PHP.
    pub fx_rate: String,
    /// Bank reference of the transfer.
    pub reference: String,
    /// The bank account it was paid from.
    pub bank_account_id: Option<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ReceiptLineInput {
    pub po_line_id: String,
    /// The box received, for lines that have boxes (one input per box).
    pub box_id: Option<String>,
    /// Required for items that track expiry; ignored for others.
    pub expiry: Option<String>,
    pub qty: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ReceiptInput {
    pub po_id: String,
    pub date: String,
    pub location_id: String,
    pub notes: String,
    pub lines: Vec<ReceiptLineInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PoInfo {
    pub id: String,
    pub no: Option<String>,
    /// False while the number was given offline: it may still change.
    pub no_confirmed: bool,
    pub supplier_id: Option<String>,
    pub supplier_code: Option<String>,
    pub supplier_name: Option<String>,
    pub date: String,
    pub currency: String,
    /// Stripped without the costs permission.
    pub fx_rate: Option<String>,
    pub expected_date: Option<String>,
    pub status: PoStatus,
    pub supplier_ref: Option<String>,
    pub notes: Option<String>,
    /// Priced lines, in the PO's currency. Stripped without costs.
    pub total_minor: Option<i64>,
    /// The total in PHP at the PO's FX. Stripped without costs.
    pub total_php_c: Option<i64>,
    /// TT payments in the PO's currency. Stripped without costs.
    pub paid_minor: Option<i64>,
    /// All TT payments in PHP, each at its own FX. Stripped without costs.
    pub paid_php_c: Option<i64>,
    /// Lines without a price yet.
    pub unpriced_lines: i64,
    pub summary: String,
    pub item_ids: Vec<String>,
    pub line_count: i64,
    pub ordered_qty: i64,
    pub received_qty: i64,
    pub progress: ReceiveProgress,
    /// Some line got more than ordered (offline PCs received it twice).
    pub over_received: bool,
    /// Posted receiving reports.
    pub receipt_count: i64,
    /// Year of the box serials.
    pub serial_year: Option<String>,
    pub box_count: i64,
    pub created_at: i64,
    pub created_by_name: Option<String>,
    pub ordered_at: Option<i64>,
    pub ordered_by_name: Option<String>,
    pub shipped_date: Option<String>,
    pub arrived_date: Option<String>,
    pub closed_at: Option<i64>,
    pub closed_by_name: Option<String>,
    pub cancel_reason: Option<String>,
}

impl PoInfo {
    pub fn strip(&mut self, perms: &Perms) {
        if !perms.sees(Field::Costs) {
            self.fx_rate = None;
            self.total_minor = None;
            self.total_php_c = None;
            self.paid_minor = None;
            self.paid_php_c = None;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PoLine {
    pub id: String,
    pub item_id: String,
    pub item_code: Option<String>,
    pub item_name: Option<String>,
    pub unit: Option<String>,
    pub tracks_expiry: bool,
    pub qty: i64,
    /// Stripped without the costs permission.
    pub price_minor: Option<i64>,
    /// Stripped without the costs permission.
    pub amount_minor: Option<i64>,
    pub notes: Option<String>,
    pub received: i64,
    /// Still to come (negative if receipts from offline PCs overshoot).
    pub remaining: i64,
    /// Box size set on the line.
    pub pcs_per_box: Option<i64>,
    /// The item's box serial code and default box size.
    pub serial_code: Option<String>,
    pub item_pcs_per_box: Option<i64>,
    /// Lowest serial first. Empty for drafts and items without boxes.
    pub boxes: Vec<PoBox>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct TtInfo {
    pub id: String,
    pub po_id: String,
    pub po_no: Option<String>,
    pub supplier_id: Option<String>,
    pub supplier_name: Option<String>,
    pub date: String,
    pub currency: String,
    pub amount_minor: i64,
    pub fx_rate: Option<String>,
    /// The amount in PHP, if the FX is known.
    pub php_c: Option<i64>,
    pub reference: Option<String>,
    /// Paid from.
    pub bank_account_id: Option<String>,
    pub bank_account_name: Option<String>,
    pub notes: Option<String>,
    pub created_at: i64,
    pub created_by_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ReceiptInfo {
    pub id: String,
    pub no: Option<String>,
    pub no_confirmed: bool,
    pub po_id: String,
    pub po_no: Option<String>,
    pub supplier_id: Option<String>,
    pub supplier_name: Option<String>,
    pub date: String,
    pub location_id: Option<String>,
    pub location_name: Option<String>,
    pub notes: Option<String>,
    pub voided: bool,
    pub voided_at: Option<i64>,
    pub voided_by_name: Option<String>,
    pub void_reason: Option<String>,
    pub created_at: i64,
    pub created_by_name: Option<String>,
    pub summary: String,
    pub item_ids: Vec<String>,
    pub line_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ReceiptLine {
    pub id: String,
    pub po_line_id: Option<String>,
    pub item_id: String,
    pub item_code: Option<String>,
    pub item_name: Option<String>,
    pub unit: Option<String>,
    pub expiry: Option<String>,
    pub qty: i64,
    pub box_id: Option<String>,
    pub serial: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ReceiptDetail {
    pub receipt: ReceiptInfo,
    pub lines: Vec<ReceiptLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PoDetail {
    pub po: PoInfo,
    pub lines: Vec<PoLine>,
    /// Empty without the costs permission.
    pub payments: Vec<TtInfo>,
    pub receipts: Vec<ReceiptInfo>,
}

impl PoDetail {
    pub fn strip(&mut self, perms: &Perms) {
        self.po.strip(perms);
        if !perms.sees(Field::Costs) {
            for l in &mut self.lines {
                l.price_minor = None;
                l.amount_minor = None;
            }
            self.payments.clear();
        }
    }
}

// Writing: purchase orders --------------------------------------------------

/// What a PO looks like now, for an update.
struct Existing {
    status: PoStatus,
    currency: String,
    fx_rate: Option<String>,
    serial_year: Option<String>,
    has_boxes: bool,
    /// Per line: item, price and qty received.
    lines: HashMap<String, (String, Option<i64>, i64)>,
}

fn existing(conn: &Connection, id: &str) -> Result<Existing> {
    let (status, currency, fx_rate, serial_year) = conn
        .query_row(
            "SELECT status, currency, fx_rate, serial_year FROM purchase_orders
             WHERE id = ?1 AND _deleted = 0",
            [id],
            |r| {
                Ok((
                    PoStatus::parse(r.get::<_, Option<String>>(0)?.as_deref()),
                    r.get::<_, Option<String>>(1)?
                        .unwrap_or_else(|| "PHP".into()),
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| Error::Invalid("that purchase order no longer exists".into()))?;
    let mut stmt = conn.prepare_cached(
        "SELECT l.id, COALESCE(l.item_id, ''), l.price_minor, COALESCE(v.qty, 0)
         FROM po_lines l LEFT JOIN v_po_received v ON v.po_line_id = l.id
         WHERE l.po_id = ?1 AND l._deleted = 0",
    )?;
    let rows = stmt.query_map([id], |r| {
        Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))
    })?;
    Ok(Existing {
        status,
        currency,
        fx_rate,
        serial_year,
        has_boxes: boxes::po_has_boxes(conn, id)?,
        lines: rows.collect::<Result<_, _>>()?,
    })
}

struct Prepared {
    date: String,
    currency: String,
    fx_rate: Option<String>,
    expected: Option<String>,
    serial_year: Option<String>,
    /// (existing id, data) per line.
    lines: Vec<(Option<String>, serde_json::Value)>,
    /// The same lines, for planning boxes.
    specs: Vec<LineSpec>,
}

fn item_code(conn: &Connection, id: &str) -> Result<Option<(String, bool)>> {
    Ok(conn
        .query_row(
            "SELECT COALESCE(code, ''), COALESCE(tracks_expiry, 0) FROM items
             WHERE id = ?1 AND _deleted = 0",
            [id],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
        )
        .optional()?)
}

/// Checks a PO without writing. `costs`: the user may set prices, currency
/// and FX; otherwise the stored ones are kept.
fn prepare(
    conn: &Connection,
    input: &PoInput,
    costs: bool,
    old: Option<&Existing>,
) -> Result<Prepared> {
    let supplier_currency: Option<String> = conn
        .query_row(
            "SELECT currency FROM suppliers WHERE id = ?1 AND _deleted = 0",
            [&input.supplier_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::Invalid("choose a supplier".into()))?;
    let date = require_date(&input.date, "PO date")?;
    let expected = date_opt(Some(&input.expected_date), "expected arrival date")?;
    let (currency, fx_rate) = if costs {
        let currency = check_currency(&input.currency)?;
        let fx = match (currency.as_str(), input.fx_rate.trim()) {
            ("PHP", _) | (_, "") => None,
            (_, text) => Some(pricing::parse_fx(text)?.to_string()),
        };
        (currency, fx)
    } else {
        match old {
            Some(o) => (o.currency.clone(), o.fx_rate.clone()),
            None => (supplier_currency.unwrap_or_else(|| "PHP".into()), None),
        }
    };
    let serial_year = match (boxes::check_year(&input.serial_year)?, old) {
        (Some(y), Some(o)) if o.has_boxes && o.serial_year.as_deref() != Some(y.as_str()) => {
            return Err(Error::Invalid(format!(
                "the boxes of this PO already have {} serials; put it back to draft to change the year",
                o.serial_year.as_deref().unwrap_or("other")
            )));
        }
        (Some(y), _) => Some(y),
        (None, o) => o.and_then(|o| o.serial_year.clone()),
    };
    if input.lines.is_empty() {
        return Err(Error::Invalid("add at least one item".into()));
    }
    if input.lines.len() > MAX_LINES {
        return Err(Error::Invalid(format!(
            "a purchase order can have at most {MAX_LINES} lines"
        )));
    }
    let mut lines = Vec::with_capacity(input.lines.len());
    let mut specs = Vec::with_capacity(input.lines.len());
    for (i, line) in input.lines.iter().enumerate() {
        let (code, _) = item_code(conn, &line.item_id)?
            .ok_or_else(|| Error::Invalid("choose an item on every line".into()))?;
        if line.qty <= 0 {
            return Err(Error::Invalid(format!("enter a quantity for {code}")));
        }
        let kept = line
            .id
            .as_ref()
            .and_then(|id| old.and_then(|o| o.lines.get(id)));
        if let Some((item, _, received)) = kept {
            if *received > 0 && *item != line.item_id {
                return Err(Error::Invalid(
                    "a line that was received can't change its item; add a new line instead".into(),
                ));
            }
            if line.qty < *received {
                return Err(Error::Invalid(format!(
                    "{received} of {code} were already received; the quantity can't be lower"
                )));
            }
        }
        let price = if costs {
            if line.price_minor.is_some_and(|p| p < 0) {
                return Err(Error::Invalid(format!(
                    "the price of {code} can't be negative"
                )));
            }
            line.price_minor
        } else {
            kept.and_then(|(_, price, _)| *price)
        };
        let per_box = boxes::check_size(line.pcs_per_box, &code)?;
        specs.push(LineSpec {
            id: kept.and(line.id.clone()),
            item_id: line.item_id.clone(),
            qty: line.qty,
            per_box,
        });
        lines.push((
            kept.and(line.id.clone()),
            json!({
                "line_no": i as i64 + 1,
                "item_id": line.item_id,
                "qty": line.qty,
                "price_minor": price,
                "notes": text_opt(&line.notes),
                "pcs_per_box": per_box,
            }),
        ));
    }
    // A received line has to stay: its receiving reports point at it.
    if let Some(o) = old {
        let kept: Vec<&str> = input.lines.iter().filter_map(|l| l.id.as_deref()).collect();
        for (id, (item, _, received)) in &o.lines {
            if *received > 0 && !kept.contains(&id.as_str()) {
                let code = item_code(conn, item)?.map(|(c, _)| c).unwrap_or_default();
                return Err(Error::Invalid(format!(
                    "{code} has been received; it can't be removed (void its receiving report first)"
                )));
            }
        }
    }
    Ok(Prepared {
        date,
        currency,
        fx_rate,
        expected,
        serial_year,
        lines,
        specs,
    })
}

/// Checks a new PO as [`create`] would, without writing. Call it before
/// reserving a number.
pub fn validate(conn: &Connection, input: &PoInput, costs: bool) -> Result<()> {
    prepare(conn, input, costs, None).map(drop)
}

/// Creates a draft PO with the number reserved for it (`res.ref_id` is its
/// id). Returns the id.
pub fn create(tx: &mut WriteTx, input: &PoInput, res: &Reservation, costs: bool) -> Result<String> {
    if res.series != series::PO {
        return Err(Error::Invalid(
            "wrong number series for a purchase order".into(),
        ));
    }
    let p = prepare(tx.conn(), input, costs, None)?;
    let id = tx.insert(
        PURCHASE_ORDERS,
        json!({
            "id": res.ref_id,
            "supplier_id": input.supplier_id,
            "date": p.date,
            "currency": p.currency,
            "fx_rate": p.fx_rate,
            "expected_date": p.expected,
            "serial_year": p.serial_year.unwrap_or_else(|| boxes::this_year(tx.now_ms())),
            "status": PoStatus::Draft.as_str(),
            "supplier_ref": text_opt(&input.supplier_ref),
            "notes": text_opt(&input.notes),
        }),
    )?;
    replace_lines(tx, PO_LINES, "po_id", &id, p.lines)?;
    numbering::assign(tx, res, PURCHASE_ORDERS)?;
    Ok(id)
}

/// Placed with the supplier, so its lines have boxes.
fn placed(status: PoStatus) -> bool {
    !matches!(status, PoStatus::Draft | PoStatus::Cancelled)
}

fn year_or_now(year: Option<&str>, now_ms: i64) -> String {
    year.map(String::from)
        .unwrap_or_else(|| boxes::this_year(now_ms))
}

/// The PO's lines as stored, in order.
fn stored_specs(conn: &Connection, po_id: &str) -> Result<Vec<LineSpec>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, COALESCE(item_id, ''), COALESCE(qty, 0), pcs_per_box FROM po_lines
         WHERE po_id = ?1 AND _deleted = 0 ORDER BY line_no, id",
    )?;
    let rows = stmt.query_map([po_id], |r| {
        Ok(LineSpec {
            id: Some(r.get(0)?),
            item_id: r.get(1)?,
            qty: r.get(2)?,
            per_box: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Gives the PO's lines the boxes they need (or takes away the ones not
/// received, for a draft or cancelled PO). `specs` are the lines as saved,
/// in order.
fn sync_boxes(
    tx: &mut WriteTx,
    po_id: &str,
    status: PoStatus,
    year: &str,
    specs: &[LineSpec],
    serials: &mut Serials,
) -> Result<()> {
    let plan = boxes::plan_po(tx.conn(), po_id, placed(status), year, specs)?;
    if plan.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = stored_specs(tx.conn(), po_id)?
        .into_iter()
        .filter_map(|s| s.id)
        .collect();
    boxes::apply_po(tx, po_id, plan, &ids, serials)
}

/// Checks an edit as [`update`] would, without writing, and returns the
/// box serials it needs. Call it before reserving them.
pub fn serials_for_update(
    conn: &Connection,
    input: &PoInput,
    costs: bool,
    now_ms: i64,
) -> Result<Vec<SerialNeed>> {
    let id = input.id.clone().unwrap_or_default();
    let old = existing(conn, &id)?;
    if old.status == PoStatus::Cancelled {
        return Err(Error::Invalid("this purchase order is cancelled".into()));
    }
    let p = prepare(conn, input, costs, Some(&old))?;
    let year = year_or_now(p.serial_year.as_deref(), now_ms);
    Ok(boxes::plan_po(conn, &id, placed(old.status), &year, &p.specs)?.needs())
}

/// Saves a PO's header and lines (any status but cancelled). A placed PO's
/// boxes follow its lines, taking new serials from `serials`.
pub fn update(
    tx: &mut WriteTx,
    input: &PoInput,
    costs: bool,
    serials: &mut Serials,
) -> Result<()> {
    let id = input.id.clone().unwrap_or_default();
    let old = existing(tx.conn(), &id)?;
    if old.status == PoStatus::Cancelled {
        return Err(Error::Invalid("this purchase order is cancelled".into()));
    }
    let p = prepare(tx.conn(), input, costs, Some(&old))?;
    let year = year_or_now(p.serial_year.as_deref(), tx.now_ms());
    tx.update(
        PURCHASE_ORDERS,
        &id,
        json!({
            "supplier_id": input.supplier_id,
            "date": p.date,
            "currency": p.currency,
            "fx_rate": p.fx_rate,
            "expected_date": p.expected,
            "supplier_ref": text_opt(&input.supplier_ref),
            "notes": text_opt(&input.notes),
            "serial_year": year,
        }),
    )?;
    replace_lines(tx, PO_LINES, "po_id", &id, p.lines)?;
    sync_boxes(tx, &id, old.status, &year, &p.specs, serials)
}

fn posted_receipts(conn: &Connection, po_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM po_receipts WHERE po_id = ?1 AND _deleted = 0 AND status = 'posted'",
        [po_id],
        |r| r.get(0),
    )?)
}

/// The box serials marking a draft PO ordered needs ([`set_status`]).
/// Call it before reserving them; other changes need none.
pub fn serials_for_status(
    conn: &Connection,
    id: &str,
    input: &PoStatusInput,
    now_ms: i64,
) -> Result<Vec<SerialNeed>> {
    if input.action != PoAction::Order {
        return Ok(Vec::new());
    }
    let old = existing(conn, id)?;
    if old.status != PoStatus::Draft {
        return Ok(Vec::new());
    }
    let year = year_or_now(old.serial_year.as_deref(), now_ms);
    Ok(boxes::plan_po(conn, id, true, &year, &stored_specs(conn, id)?)?.needs())
}

/// Changes a PO's status. Marking it ordered gives its lines their boxes
/// (serials from `serials`); back to draft or cancelled takes them away.
pub fn set_status(
    tx: &mut WriteTx,
    id: &str,
    input: &PoStatusInput,
    serials: &mut Serials,
) -> Result<()> {
    let row = tx
        .get(PURCHASE_ORDERS, id)?
        .ok_or_else(|| Error::Invalid("that purchase order no longer exists".into()))?;
    let status = PoStatus::parse(row["status"].as_str());
    let received = posted_receipts(tx.conn(), id)?;
    let (now, user) = (tx.now_ms(), tx.actor().user_id.clone());
    let today = manila_date_string(now);
    let on = |what: &str| -> Result<String> {
        let date = date_opt(Some(&input.date), what)?.unwrap_or_else(|| today.clone());
        if date > today {
            return Err(Error::Invalid(format!("the {what} can't be in the future")));
        }
        Ok(date)
    };
    let patch = match (input.action, status) {
        (PoAction::Order, PoStatus::Draft) => json!({
            "status": PoStatus::Ordered.as_str(),
            "ordered_at": now,
            "ordered_by": user,
        }),
        (PoAction::BackToDraft, PoStatus::Ordered) => {
            if received > 0 {
                return Err(Error::Invalid(
                    "goods were already received on this PO".into(),
                ));
            }
            json!({ "status": PoStatus::Draft.as_str() })
        }
        (PoAction::Ship, PoStatus::Ordered) => json!({
            "status": PoStatus::Shipped.as_str(),
            "shipped_date": on("shipping date")?,
        }),
        (PoAction::Arrive, PoStatus::Ordered | PoStatus::Shipped) => json!({
            "status": PoStatus::Arrived.as_str(),
            "arrived_date": on("arrival date")?,
        }),
        (PoAction::Close, s) if s.is_open() => json!({
            "status": PoStatus::Closed.as_str(),
            "closed_at": now,
            "closed_by": user,
        }),
        (PoAction::Reopen, PoStatus::Closed) => {
            let back = if row["arrived_date"].is_string() {
                PoStatus::Arrived
            } else if row["shipped_date"].is_string() {
                PoStatus::Shipped
            } else {
                PoStatus::Ordered
            };
            json!({ "status": back.as_str(), "closed_at": null, "closed_by": null })
        }
        (PoAction::Cancel, s) if s == PoStatus::Draft || s.is_open() => {
            if received > 0 {
                return Err(Error::Invalid(
                    "goods were already received on this PO; void the receiving reports first, or close it"
                        .into(),
                ));
            }
            json!({
                "status": PoStatus::Cancelled.as_str(),
                "closed_at": now,
                "closed_by": user,
                "cancel_reason": text_opt(&input.reason),
            })
        }
        (_, s) => {
            return Err(Error::Invalid(format!(
                "this purchase order is {}; it can't be changed that way",
                s.as_str()
            )));
        }
    };
    tx.update(PURCHASE_ORDERS, id, patch.clone())?;
    if matches!(
        input.action,
        PoAction::Order | PoAction::BackToDraft | PoAction::Cancel
    ) {
        let status = PoStatus::parse(patch["status"].as_str());
        let year = year_or_now(row["serial_year"].as_str(), now);
        if row["serial_year"].is_null() {
            tx.update(PURCHASE_ORDERS, id, json!({ "serial_year": year }))?;
        }
        let specs = stored_specs(tx.conn(), id)?;
        sync_boxes(tx, id, status, &year, &specs, serials)?;
    }
    Ok(())
}

/// Deletes a draft PO without payments. Its number is not given out again.
pub fn delete(tx: &mut WriteTx, id: &str) -> Result<()> {
    let row = tx
        .get(PURCHASE_ORDERS, id)?
        .ok_or_else(|| Error::Invalid("that purchase order no longer exists".into()))?;
    if PoStatus::parse(row["status"].as_str()) != PoStatus::Draft {
        return Err(Error::Invalid(
            "only a draft purchase order can be deleted; cancel it instead".into(),
        ));
    }
    let paid: i64 = tx.conn().query_row(
        "SELECT COUNT(*) FROM supplier_payments WHERE po_id = ?1 AND _deleted = 0",
        [id],
        |r| r.get(0),
    )?;
    if paid > 0 {
        return Err(Error::Invalid(
            "this purchase order has TT payments; delete them first, or cancel the PO".into(),
        ));
    }
    for line in crate::domain::line_ids(tx, PO_LINES, "po_id", id)? {
        tx.delete(PO_LINES, &line)?;
    }
    tx.delete(PURCHASE_ORDERS, id)
}

/// Copies the PO's supplier prices, currency and FX into the cost sheets of
/// its items (charge lines are kept). The first priced line of an item
/// wins. The item's total ordered quantity becomes the shipment quantity
/// that shipment charges are divided by. Returns the item codes updated.
pub fn apply_to_cost_sheets(tx: &mut WriteTx, po_id: &str) -> Result<Vec<String>> {
    let old = existing(tx.conn(), po_id)?;
    if old.currency != "PHP" && old.fx_rate.is_none() {
        return Err(Error::Invalid("enter the PO's exchange rate first".into()));
    }
    let lines: Vec<(String, String, i64)> = {
        let mut stmt = tx.conn().prepare(
            "SELECT l.item_id, COALESCE(i.code, ''), l.price_minor FROM po_lines l
             JOIN items i ON i.id = l.item_id AND i._deleted = 0
             WHERE l.po_id = ?1 AND l._deleted = 0 AND l.price_minor IS NOT NULL
             ORDER BY l.line_no, l.id",
        )?;
        let rows = stmt.query_map([po_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    if lines.is_empty() {
        return Err(Error::Invalid("enter the PO's prices first".into()));
    }
    let mut done: BTreeMap<String, String> = BTreeMap::new();
    for (item, code, price) in lines {
        if done.contains_key(&item) {
            continue;
        }
        copy_to_sheet(tx, po_id, &item, &old.currency, old.fx_rate.as_deref(), price)?;
        done.insert(item, code);
    }
    let mut codes: Vec<String> = done.into_values().collect();
    codes.sort();
    Ok(codes)
}

/// Writes a PO price into an item's cost sheet, with the item's total
/// ordered quantity on that PO as the shipment quantity.
fn copy_to_sheet(
    tx: &mut WriteTx,
    po_id: &str,
    item: &str,
    currency: &str,
    fx: Option<&str>,
    price: i64,
) -> Result<()> {
    let fx = if currency == "PHP" { None } else { fx };
    let qty: Option<i64> = tx.conn().query_row(
        "SELECT SUM(qty) FROM po_lines
         WHERE po_id = ?1 AND item_id = ?2 AND _deleted = 0 AND qty > 0",
        [po_id, item],
        |r| r.get(0),
    )?;
    let mut data = json!({
        "item_id": item,
        "currency": currency,
        "supplier_price_minor": price,
        "fx_rate": fx,
    });
    if let Some(q) = qty {
        data["shipment_qty"] = json!(q);
    }
    tx.upsert(ITEM_COSTS, &items::cost_id(item), data)
}

/// The newest supplier price of an item: its first priced line on the
/// latest placed PO (not a draft, not cancelled).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SupplierPrice {
    pub po_id: String,
    pub po_no: Option<String>,
    pub po_date: Option<String>,
    pub currency: String,
    pub price_minor: i64,
    /// `None` for PHP, or while the PO has no rate yet.
    pub fx_rate: Option<String>,
    /// Total ordered on that PO; becomes the cost sheet's shipment quantity.
    pub qty: Option<i64>,
}

/// Latest by PO date, then by when it was placed.
pub fn latest_price(conn: &Connection, item_id: &str) -> Result<Option<SupplierPrice>> {
    let found = conn
        .query_row(
            "SELECT p.id, n.no, p.date, COALESCE(p.currency, 'PHP'), l.price_minor, p.fx_rate,
                    (SELECT SUM(q.qty) FROM po_lines q
                     WHERE q.po_id = p.id AND q.item_id = l.item_id AND q._deleted = 0
                       AND q.qty > 0)
             FROM po_lines l
             JOIN purchase_orders p ON p.id = l.po_id AND p._deleted = 0
             LEFT JOIN doc_numbers n ON n.ref_id = p.id AND n.series = 'PO' AND n._deleted = 0
             WHERE l.item_id = ?1 AND l._deleted = 0 AND l.price_minor IS NOT NULL
               AND COALESCE(p.status, 'draft') NOT IN ('draft', 'cancelled')
             ORDER BY p.date DESC, COALESCE(p.ordered_at, 0) DESC, p.id DESC, l.line_no, l.id
             LIMIT 1",
            [item_id],
            |r| {
                Ok(SupplierPrice {
                    po_id: r.get(0)?,
                    po_no: r.get(1)?,
                    po_date: r.get(2)?,
                    currency: r.get(3)?,
                    price_minor: r.get(4)?,
                    fx_rate: r.get(5)?,
                    qty: r.get(6)?,
                })
            },
        )
        .optional()?;
    Ok(found.map(|mut p| {
        if p.currency == "PHP" {
            p.fx_rate = None;
        }
        p
    }))
}

/// Copies an item's latest supplier price ([`latest_price`]) into its cost
/// sheet. Charge lines are kept. Only ever done on request.
pub fn apply_latest_price(tx: &mut WriteTx, item_id: &str) -> Result<SupplierPrice> {
    let price = latest_price(tx.conn(), item_id)?.ok_or_else(|| {
        Error::Invalid("no placed purchase order has a price for this item yet".into())
    })?;
    if price.currency != "PHP" && price.fx_rate.is_none() {
        return Err(Error::Invalid(format!(
            "enter the exchange rate on {} first",
            price.po_no.as_deref().unwrap_or("the purchase order")
        )));
    }
    copy_to_sheet(
        tx,
        &price.po_id,
        item_id,
        &price.currency,
        price.fx_rate.as_deref(),
        price.price_minor,
    )?;
    Ok(price)
}

// Writing: TT payments ------------------------------------------------------

/// Adds or saves a payment to the supplier. Returns its id.
pub fn save_payment(tx: &mut WriteTx, input: &TtInput) -> Result<String> {
    if tx.get(PURCHASE_ORDERS, &input.po_id)?.is_none() {
        return Err(Error::Invalid(
            "that purchase order no longer exists".into(),
        ));
    }
    let today = manila_date_string(tx.now_ms());
    let date = require_date(&input.date, "payment date")?;
    if date > today {
        return Err(Error::Invalid(
            "the payment date can't be in the future".into(),
        ));
    }
    if input.amount_minor <= 0 {
        return Err(Error::Invalid("enter the amount paid".into()));
    }
    let currency = check_currency(&input.currency)?;
    let fx = match (currency.as_str(), input.fx_rate.trim()) {
        ("PHP", _) => None,
        (_, "") => {
            return Err(Error::Invalid(
                "enter the exchange rate of the transfer".into(),
            ));
        }
        (_, text) => Some(pricing::parse_fx(text)?.to_string()),
    };
    let stored = match &input.id {
        Some(id) => {
            let row = tx
                .get(SUPPLIER_PAYMENTS, id)?
                .ok_or_else(|| Error::Invalid("that payment no longer exists".into()))?;
            if row["po_id"].as_str() != Some(input.po_id.as_str()) {
                return Err(Error::Invalid("that payment is on another PO".into()));
            }
            Some(row)
        }
        None => None,
    };
    let bank_account = bank_accounts::usable(
        tx.conn(),
        input.bank_account_id.as_deref(),
        stored.as_ref().and_then(|r| r["bank_account_id"].as_str()),
    )?;
    let data = json!({
        "po_id": input.po_id,
        "date": date,
        "currency": currency,
        "amount_minor": input.amount_minor,
        "fx_rate": fx,
        "reference": text_opt(&input.reference),
        "bank_account_id": bank_account,
        "notes": text_opt(&input.notes),
    });
    match &input.id {
        Some(id) => {
            tx.update(SUPPLIER_PAYMENTS, id, data)?;
            Ok(id.clone())
        }
        None => tx.insert(SUPPLIER_PAYMENTS, data),
    }
}

pub fn delete_payment(tx: &mut WriteTx, id: &str) -> Result<()> {
    if tx.get(SUPPLIER_PAYMENTS, id)?.is_none() {
        return Err(Error::Invalid("that payment no longer exists".into()));
    }
    tx.delete(SUPPLIER_PAYMENTS, id)
}

// Writing: receiving --------------------------------------------------------

struct PreparedReceipt {
    date: String,
    lines: Vec<ReceiptRow>,
}

struct ReceiptRow {
    po_line: String,
    item: String,
    expiry: Option<String>,
    qty: i64,
    box_id: Option<String>,
}

fn prepare_receipt(
    conn: &Connection,
    input: &ReceiptInput,
    today: &str,
) -> Result<PreparedReceipt> {
    let old = existing(conn, &input.po_id)?;
    match old.status {
        s if s.is_open() => {}
        PoStatus::Draft => {
            return Err(Error::Invalid(
                "mark the purchase order as ordered first".into(),
            ));
        }
        PoStatus::Closed => {
            return Err(Error::Invalid(
                "this purchase order is closed; reopen it first".into(),
            ));
        }
        _ => return Err(Error::Invalid("this purchase order is cancelled".into())),
    }
    let date = require_date(&input.date, "receiving date")?;
    if date.as_str() > today {
        return Err(Error::Invalid("the date can't be in the future".into()));
    }
    inventory::live_location(conn, &input.location_id)?;
    if input.lines.is_empty() {
        return Err(Error::Invalid(
            "enter the quantity received of at least one item".into(),
        ));
    }
    if input.lines.len() > boxes::MAX_BOXES {
        return Err(Error::Invalid(format!(
            "a receiving report can have at most {} lines",
            boxes::MAX_BOXES
        )));
    }
    let po_boxes = boxes::po_boxes(conn, &input.po_id)?;
    let mut lines = Vec::with_capacity(input.lines.len());
    let mut taken: HashMap<&str, i64> = HashMap::new();
    let mut seen_boxes: Vec<&str> = Vec::new();
    for line in &input.lines {
        let (item, _, received) = old
            .lines
            .get(&line.po_line_id)
            .ok_or_else(|| Error::Invalid("one of the PO lines no longer exists".into()))?;
        let (code, tracks) = item_code(conn, item)?
            .ok_or_else(|| Error::Invalid("one of the items no longer exists".into()))?;
        if line.qty <= 0 {
            return Err(Error::Invalid(format!(
                "enter the quantity of {code} received"
            )));
        }
        let expiry = match line
            .expiry
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            _ if !tracks => None,
            Some(text) => Some(parse_date(text).ok_or_else(|| {
                Error::Invalid(format!("the expiry date of {code} is not a valid date"))
            })?),
            None => return Err(Error::Invalid(format!("enter the expiry date of {code}"))),
        };
        let ordered: i64 = conn.query_row(
            "SELECT COALESCE(qty, 0) FROM po_lines WHERE id = ?1",
            [&line.po_line_id],
            |r| r.get(0),
        )?;
        let sum = taken.entry(line.po_line_id.as_str()).or_default();
        *sum += line.qty;
        let left = ordered - received;
        if *sum > left {
            return Err(Error::Invalid(format!(
                "only {} of {code} left to receive (ordered {ordered}, received {received}); raise the PO quantity first",
                left.max(0)
            )));
        }
        // A line with boxes is received box by box.
        let box_id = match po_boxes.get(&line.po_line_id) {
            Some(line_boxes) if !line_boxes.is_empty() => {
                let id = line
                    .box_id
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| Error::Invalid(format!("choose the box of {code}")))?;
                let b = line_boxes.iter().find(|b| b.id == id).ok_or_else(|| {
                    Error::Invalid(format!("one of the boxes of {code} is not on this PO line"))
                })?;
                let serial = b.serial.as_deref().unwrap_or("?");
                if b.received > 0 {
                    return Err(Error::Invalid(format!(
                        "box {serial} was already received"
                    )));
                }
                if seen_boxes.contains(&id) {
                    return Err(Error::Invalid(format!(
                        "box {serial} is on the report twice"
                    )));
                }
                if line.qty > b.pcs {
                    return Err(Error::Invalid(format!(
                        "box {serial} holds {}; it can't come in with {}",
                        b.pcs, line.qty
                    )));
                }
                seen_boxes.push(id);
                Some(id.to_string())
            }
            _ => None,
        };
        lines.push(ReceiptRow {
            po_line: line.po_line_id.clone(),
            item: item.clone(),
            expiry,
            qty: line.qty,
            box_id,
        });
    }
    Ok(PreparedReceipt { date, lines })
}

/// Checks a receiving report as [`receive`] would, without writing. Call it
/// before reserving a number.
pub fn validate_receipt(conn: &Connection, input: &ReceiptInput, today: &str) -> Result<()> {
    prepare_receipt(conn, input, today).map(drop)
}

/// Posts a receiving report with the number reserved for it (`res.ref_id`
/// is its id): the goods are in stock from then on. An ordered or shipped
/// PO becomes arrived. Returns the id.
pub fn receive(tx: &mut WriteTx, input: &ReceiptInput, res: &Reservation) -> Result<String> {
    if res.series != series::RECEIVING {
        return Err(Error::Invalid(
            "wrong number series for a receiving report".into(),
        ));
    }
    let today = manila_date_string(tx.now_ms());
    let p = prepare_receipt(tx.conn(), input, &today)?;
    let id = tx.insert(
        PO_RECEIPTS,
        json!({
            "id": res.ref_id,
            "po_id": input.po_id,
            "date": p.date,
            "location_id": input.location_id,
            "notes": text_opt(&input.notes),
            "status": "posted",
        }),
    )?;
    for (i, l) in p.lines.into_iter().enumerate() {
        tx.insert(
            PO_RECEIPT_LINES,
            json!({
                "receipt_id": id,
                "line_no": i as i64 + 1,
                "po_line_id": l.po_line,
                "item_id": l.item,
                "expiry": l.expiry,
                "qty": l.qty,
                "box_id": l.box_id,
            }),
        )?;
    }
    numbering::assign(tx, res, PO_RECEIPTS)?;
    let po = tx
        .get(PURCHASE_ORDERS, &input.po_id)?
        .ok_or_else(|| Error::Invalid("that purchase order no longer exists".into()))?;
    if matches!(
        PoStatus::parse(po["status"].as_str()),
        PoStatus::Ordered | PoStatus::Shipped
    ) {
        let arrived = po["arrived_date"]
            .as_str()
            .map(String::from)
            .unwrap_or(p.date);
        tx.update(
            PURCHASE_ORDERS,
            &input.po_id,
            json!({ "status": PoStatus::Arrived.as_str(), "arrived_date": arrived }),
        )?;
    }
    Ok(id)
}

/// Voids a posted receiving report: its goods no longer count as received
/// or in stock. Refused if the stock was already used.
pub fn void_receipt(tx: &mut WriteTx, id: &str, reason: &str) -> Result<()> {
    let row = tx
        .get(PO_RECEIPTS, id)?
        .ok_or_else(|| Error::Invalid("that receiving report no longer exists".into()))?;
    if row["status"].as_str() == Some("void") {
        return Err(Error::Invalid(
            "that receiving report is already void".into(),
        ));
    }
    let reason = text_opt(reason)
        .ok_or_else(|| Error::Invalid("enter why the receiving report is voided".into()))?;
    let undo: Vec<Effect> = inventory::doc_effect(tx.conn(), id)?
        .into_iter()
        .map(|e| Effect { qty: -e.qty, ..e })
        .collect();
    inventory::check_not_negative(tx.conn(), &undo, "can't void: ")?;
    let (now, user) = (tx.now_ms(), tx.actor().user_id.clone());
    tx.update(
        PO_RECEIPTS,
        id,
        json!({
            "status": "void",
            "voided_at": now,
            "voided_by": user,
            "void_reason": reason,
        }),
    )
    .map(drop)
}

// Reading -------------------------------------------------------------------

/// `minor × fx` in PHP centavos; PHP amounts as they are.
pub(crate) fn php_of(currency: &str, minor: i64, fx: Option<&str>) -> Option<i64> {
    if currency == "PHP" {
        return Some(minor);
    }
    let fx = pricing::parse_fx(fx?).ok()?;
    pricing::base_cost_c(minor, fx)
}

const PO_SELECT: &str = "
    SELECT p.id, n.no, COALESCE(n.confirmed, 1), p.supplier_id, sn.no, s.name, p.date,
           COALESCE(p.currency, 'PHP'), p.fx_rate, p.expected_date, p.status, p.supplier_ref,
           p.notes, p.created_at, uc.display_name, p.ordered_at, uo.display_name,
           p.shipped_date, p.arrived_date, p.closed_at, ul.display_name, p.cancel_reason,
           p.serial_year
    FROM purchase_orders p
    LEFT JOIN doc_numbers n ON n.ref_id = p.id AND n.series = 'PO' AND n._deleted = 0
    LEFT JOIN suppliers s ON s.id = p.supplier_id
    LEFT JOIN doc_numbers sn ON sn.ref_id = p.supplier_id AND sn.series = 'SUP'
                            AND sn._deleted = 0
    LEFT JOIN users uc ON uc.id = p.created_by
    LEFT JOIN users uo ON uo.id = p.ordered_by
    LEFT JOIN users ul ON ul.id = p.closed_by
    WHERE p._deleted = 0";

fn po_from_row(r: &rusqlite::Row) -> rusqlite::Result<PoInfo> {
    Ok(PoInfo {
        id: r.get(0)?,
        no: r.get(1)?,
        no_confirmed: r.get::<_, i64>(2)? != 0,
        supplier_id: r.get(3)?,
        supplier_code: r.get(4)?,
        supplier_name: r.get(5)?,
        date: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        currency: r.get(7)?,
        fx_rate: r.get(8)?,
        expected_date: r.get(9)?,
        status: PoStatus::parse(r.get::<_, Option<String>>(10)?.as_deref()),
        supplier_ref: r.get(11)?,
        notes: r.get(12)?,
        created_at: r.get(13)?,
        created_by_name: r.get(14)?,
        ordered_at: r.get(15)?,
        ordered_by_name: r.get(16)?,
        shipped_date: r.get(17)?,
        arrived_date: r.get(18)?,
        closed_at: r.get(19)?,
        closed_by_name: r.get(20)?,
        cancel_reason: r.get(21)?,
        serial_year: r.get(22)?,
        box_count: 0,
        total_minor: None,
        total_php_c: None,
        paid_minor: None,
        paid_php_c: None,
        unpriced_lines: 0,
        summary: String::new(),
        item_ids: Vec::new(),
        line_count: 0,
        ordered_qty: 0,
        received_qty: 0,
        progress: ReceiveProgress::None,
        over_received: false,
        receipt_count: 0,
    })
}

/// Lines of one PO (or all), with what was received, as (po, line).
fn lines_of(conn: &Connection, po: Option<&str>) -> Result<Vec<(String, PoLine)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT l.po_id, l.id, l.item_id, i.code, i.name, i.unit, COALESCE(i.tracks_expiry, 0),
                COALESCE(l.qty, 0), l.price_minor, l.notes, COALESCE(v.qty, 0), l.pcs_per_box,
                i.serial_code, i.pcs_per_box
         FROM po_lines l
         LEFT JOIN items i ON i.id = l.item_id
         LEFT JOIN v_po_received v ON v.po_line_id = l.id
         WHERE l._deleted = 0 AND (?1 IS NULL OR l.po_id = ?1)
         ORDER BY l.po_id, l.line_no, l.id",
    )?;
    let rows = stmt.query_map([po], |r| {
        let qty: i64 = r.get(7)?;
        let price: Option<i64> = r.get(8)?;
        let received: i64 = r.get(10)?;
        Ok((
            r.get(0)?,
            PoLine {
                id: r.get(1)?,
                item_id: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                item_code: r.get(3)?,
                item_name: r.get(4)?,
                unit: r.get(5)?,
                tracks_expiry: r.get::<_, i64>(6)? != 0,
                qty,
                price_minor: price,
                amount_minor: price.map(|p| line_amount(qty, p)),
                notes: r.get(9)?,
                received,
                remaining: qty - received,
                pcs_per_box: r.get(11)?,
                serial_code: r.get(12)?,
                item_pcs_per_box: r.get(13)?,
                boxes: Vec::new(),
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

const TT_SELECT: &str = "
    SELECT t.id, t.po_id, n.no, p.supplier_id, s.name, t.date, COALESCE(t.currency, 'PHP'),
           COALESCE(t.amount_minor, 0), t.fx_rate, t.reference, t.notes, t.created_at,
           uc.display_name, t.bank_account_id, ba.name
    FROM supplier_payments t
    LEFT JOIN bank_accounts ba ON ba.id = t.bank_account_id
    JOIN purchase_orders p ON p.id = t.po_id AND p._deleted = 0
    LEFT JOIN doc_numbers n ON n.ref_id = t.po_id AND n.series = 'PO' AND n._deleted = 0
    LEFT JOIN suppliers s ON s.id = p.supplier_id
    LEFT JOIN users uc ON uc.id = t.created_by
    WHERE t._deleted = 0 AND (?1 IS NULL OR t.po_id = ?1)
    ORDER BY t.date DESC, t.created_at DESC, t.id DESC";

fn payments_of(conn: &Connection, po: Option<&str>) -> Result<Vec<TtInfo>> {
    let mut stmt = conn.prepare_cached(TT_SELECT)?;
    let rows = stmt.query_map([po], |r| {
        let currency: String = r.get(6)?;
        let amount: i64 = r.get(7)?;
        let fx: Option<String> = r.get(8)?;
        Ok(TtInfo {
            id: r.get(0)?,
            po_id: r.get(1)?,
            po_no: r.get(2)?,
            supplier_id: r.get(3)?,
            supplier_name: r.get(4)?,
            date: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            php_c: php_of(&currency, amount, fx.as_deref()),
            currency,
            amount_minor: amount,
            fx_rate: fx,
            reference: r.get(9)?,
            bank_account_id: r.get(13)?,
            bank_account_name: r.get(14)?,
            notes: r.get(10)?,
            created_at: r.get(11)?,
            created_by_name: r.get(12)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn fill(po: &mut PoInfo, lines: &[PoLine], payments: &[TtInfo], receipts: i64) {
    let total: i64 = lines.iter().filter_map(|l| l.amount_minor).sum();
    po.total_minor = Some(total);
    po.total_php_c = php_of(&po.currency, total, po.fx_rate.as_deref());
    po.paid_minor = Some(
        payments
            .iter()
            .filter(|t| t.currency == po.currency)
            .map(|t| t.amount_minor)
            .sum(),
    );
    po.paid_php_c = payments.iter().map(|t| t.php_c).sum::<Option<i64>>();
    po.unpriced_lines = lines.iter().filter(|l| l.price_minor.is_none()).count() as i64;
    po.summary = line_summary(lines.iter().map(|l| (l.item_code.as_deref(), l.qty)));
    po.item_ids = lines.iter().map(|l| l.item_id.clone()).collect();
    po.item_ids.sort();
    po.item_ids.dedup();
    po.line_count = lines.len() as i64;
    po.ordered_qty = lines.iter().map(|l| l.qty).sum();
    po.received_qty = lines.iter().map(|l| l.received).sum();
    po.receipt_count = receipts;
    po.over_received = lines.iter().any(|l| l.received > l.qty);
    po.progress = if !lines.is_empty() && lines.iter().all(|l| l.received >= l.qty) {
        ReceiveProgress::Full
    } else if lines.iter().any(|l| l.received > 0) {
        ReceiveProgress::Partial
    } else {
        ReceiveProgress::None
    };
}

fn receipt_counts(conn: &Connection) -> Result<HashMap<String, i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT po_id, COUNT(*) FROM po_receipts
         WHERE _deleted = 0 AND status = 'posted' AND po_id IS NOT NULL GROUP BY po_id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Every purchase order, newest first.
pub fn list(conn: &Connection) -> Result<Vec<PoInfo>> {
    let mut by_po: HashMap<String, Vec<PoLine>> = HashMap::new();
    for (po, line) in lines_of(conn, None)? {
        by_po.entry(po).or_default().push(line);
    }
    let mut paid: HashMap<String, Vec<TtInfo>> = HashMap::new();
    for t in payments_of(conn, None)? {
        paid.entry(t.po_id.clone()).or_default().push(t);
    }
    let receipts = receipt_counts(conn)?;
    let box_counts = boxes::box_counts(conn)?;
    let mut stmt = conn.prepare(&format!(
        "{PO_SELECT} ORDER BY p.date DESC, p.created_at DESC, p.id DESC"
    ))?;
    let rows = stmt.query_map([], po_from_row)?;
    let mut out = Vec::new();
    for row in rows {
        let mut po = row?;
        let lines = by_po.remove(&po.id).unwrap_or_default();
        let payments = paid.remove(&po.id).unwrap_or_default();
        let n = receipts.get(&po.id).copied().unwrap_or(0);
        fill(&mut po, &lines, &payments, n);
        po.box_count = box_counts.get(&po.id).copied().unwrap_or(0);
        out.push(po);
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<PoDetail>> {
    let Some(mut po) = conn
        .query_row(&format!("{PO_SELECT} AND p.id = ?1"), [id], po_from_row)
        .optional()?
    else {
        return Ok(None);
    };
    let mut po_boxes = boxes::po_boxes(conn, id)?;
    let lines: Vec<PoLine> = lines_of(conn, Some(id))?
        .into_iter()
        .map(|(_, mut l)| {
            l.boxes = po_boxes.remove(&l.id).unwrap_or_default();
            l
        })
        .collect();
    po.box_count = lines.iter().map(|l| l.boxes.len() as i64).sum();
    let payments = payments_of(conn, Some(id))?;
    let receipts = receipts_of(conn, Some(id))?;
    let posted = receipts.iter().filter(|r| !r.voided).count() as i64;
    fill(&mut po, &lines, &payments, posted);
    Ok(Some(PoDetail {
        po,
        lines,
        payments,
        receipts,
    }))
}

/// Every TT payment, newest first.
pub fn list_payments(conn: &Connection) -> Result<Vec<TtInfo>> {
    payments_of(conn, None)
}

const RECEIPT_SELECT: &str = "
    SELECT r.id, n.no, COALESCE(n.confirmed, 1), r.po_id, pn.no, p.supplier_id, s.name, r.date,
           r.location_id, l.name, r.notes, COALESCE(r.status, 'posted'), r.voided_at,
           uv.display_name, r.void_reason, r.created_at, uc.display_name
    FROM po_receipts r
    LEFT JOIN doc_numbers n ON n.ref_id = r.id AND n.series = 'RR' AND n._deleted = 0
    LEFT JOIN purchase_orders p ON p.id = r.po_id
    LEFT JOIN doc_numbers pn ON pn.ref_id = r.po_id AND pn.series = 'PO' AND pn._deleted = 0
    LEFT JOIN suppliers s ON s.id = p.supplier_id
    LEFT JOIN locations l ON l.id = r.location_id
    LEFT JOIN users uv ON uv.id = r.voided_by
    LEFT JOIN users uc ON uc.id = r.created_by
    WHERE r._deleted = 0";

fn receipt_from_row(r: &rusqlite::Row) -> rusqlite::Result<ReceiptInfo> {
    Ok(ReceiptInfo {
        id: r.get(0)?,
        no: r.get(1)?,
        no_confirmed: r.get::<_, i64>(2)? != 0,
        po_id: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        po_no: r.get(4)?,
        supplier_id: r.get(5)?,
        supplier_name: r.get(6)?,
        date: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
        location_id: r.get(8)?,
        location_name: r.get(9)?,
        notes: r.get(10)?,
        voided: r.get::<_, String>(11)? == "void",
        voided_at: r.get(12)?,
        voided_by_name: r.get(13)?,
        void_reason: r.get(14)?,
        created_at: r.get(15)?,
        created_by_name: r.get(16)?,
        summary: String::new(),
        item_ids: Vec::new(),
        line_count: 0,
    })
}

fn receipt_lines_of(
    conn: &Connection,
    receipt: Option<&str>,
) -> Result<Vec<(String, ReceiptLine)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT l.receipt_id, l.id, l.po_line_id, l.item_id, i.code, i.name, i.unit, l.expiry,
                COALESCE(l.qty, 0), l.box_id, bn.no
         FROM po_receipt_lines l LEFT JOIN items i ON i.id = l.item_id
         LEFT JOIN doc_numbers bn ON bn.ref_id = l.box_id AND bn.ref_tbl = 'boxes'
                                 AND bn._deleted = 0
         WHERE l._deleted = 0 AND (?1 IS NULL OR l.receipt_id = ?1)
         ORDER BY l.receipt_id, l.line_no, l.id",
    )?;
    let rows = stmt.query_map([receipt], |r| {
        Ok((
            r.get(0)?,
            ReceiptLine {
                id: r.get(1)?,
                po_line_id: r.get(2)?,
                item_id: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                item_code: r.get(4)?,
                item_name: r.get(5)?,
                unit: r.get(6)?,
                expiry: r.get(7)?,
                qty: r.get(8)?,
                box_id: r.get(9)?,
                serial: r.get(10)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn summarize(r: &mut ReceiptInfo, lines: &[ReceiptLine]) {
    r.summary = line_summary(lines.iter().map(|l| (l.item_code.as_deref(), l.qty)));
    r.item_ids = lines.iter().map(|l| l.item_id.clone()).collect();
    r.item_ids.sort();
    r.item_ids.dedup();
    r.line_count = lines.len() as i64;
}

fn receipts_of(conn: &Connection, po: Option<&str>) -> Result<Vec<ReceiptInfo>> {
    let mut by_receipt: HashMap<String, Vec<ReceiptLine>> = HashMap::new();
    for (r, line) in receipt_lines_of(conn, None)? {
        by_receipt.entry(r).or_default().push(line);
    }
    let mut stmt = conn.prepare(&format!(
        "{RECEIPT_SELECT} AND (?1 IS NULL OR r.po_id = ?1)
         ORDER BY r.date DESC, r.created_at DESC, r.id DESC"
    ))?;
    let rows = stmt.query_map([po], receipt_from_row)?;
    let mut out = Vec::new();
    for row in rows {
        let mut r = row?;
        let lines = by_receipt.remove(&r.id).unwrap_or_default();
        summarize(&mut r, &lines);
        out.push(r);
    }
    Ok(out)
}

/// Every receiving report, newest first (voided ones too).
pub fn list_receipts(conn: &Connection) -> Result<Vec<ReceiptInfo>> {
    receipts_of(conn, None)
}

pub fn get_receipt(conn: &Connection, id: &str) -> Result<Option<ReceiptDetail>> {
    let Some(mut receipt) = conn
        .query_row(
            &format!("{RECEIPT_SELECT} AND r.id = ?1"),
            [id],
            receipt_from_row,
        )
        .optional()?
    else {
        return Ok(None);
    };
    let lines: Vec<ReceiptLine> = receipt_lines_of(conn, Some(id))?
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    summarize(&mut receipt, &lines);
    Ok(Some(ReceiptDetail { receipt, lines }))
}
