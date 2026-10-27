//! Stock: manual receipts, adjustments and transfers, and what is on hand.
//!
//! A stock document is posted when it is saved and never edited. A mistake
//! is voided and entered again, so two PCs can't edit the same document's
//! lines concurrently. On-hand stock is never stored: it is the `v_stock`
//! view over posted documents (later also delivered DRs and PO receipts).
//!
//! Saving refuses anything that would take a lot (item, location, expiry)
//! below zero on this PC. PCs that are offline from each other can still
//! both take the same stock; the result shows as negative stock.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::boxes::{self, Serial, SerialNeed, Serials};
use crate::domain::text_opt;
use crate::error::{Error, Result};
use crate::repo::WriteTx;
use crate::sync::numbering::{self, Reservation, series};
use crate::time::{add_days, days_between, manila_date_string, parse_date};

pub const STOCK_DOCS: &str = "stock_docs";
pub const STOCK_DOC_LINES: &str = "stock_doc_lines";

/// Lines per document.
pub const MAX_LINES: usize = 500;
/// Stock expiring within this many days counts as near expiry.
pub const NEAR_EXPIRY_DAYS: i64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum StockKind {
    Receipt,
    Adjustment,
    Transfer,
    /// A delivered DR (stock out). Recorded in Accounting, not as a stock
    /// document.
    Delivery,
    /// A receiving report of a purchase order (stock in). Recorded in
    /// Order, not as a stock document.
    PoReceipt,
}

impl StockKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StockKind::Receipt => "receipt",
            StockKind::Adjustment => "adjustment",
            StockKind::Transfer => "transfer",
            StockKind::Delivery => "delivery",
            StockKind::PoReceipt => "po_receipt",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "receipt" => Some(StockKind::Receipt),
            "adjustment" => Some(StockKind::Adjustment),
            "transfer" => Some(StockKind::Transfer),
            "delivery" => Some(StockKind::Delivery),
            "po_receipt" => Some(StockKind::PoReceipt),
            _ => None,
        }
    }

    /// The number series of this kind of document.
    pub fn series(self) -> &'static str {
        match self {
            StockKind::Receipt => series::STOCK_RECEIPT,
            StockKind::Adjustment => series::ADJUSTMENT,
            StockKind::Transfer => series::TRANSFER,
            StockKind::Delivery => series::DR,
            StockKind::PoReceipt => series::RECEIVING,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StockLineInput {
    pub item_id: String,
    /// `YYYY-MM-DD`. Required for stock coming in of an item that tracks
    /// expiry; ignored for incoming stock of other items.
    pub expiry: Option<String>,
    /// In the item's base unit. Signed for adjustments, positive otherwise.
    pub qty: i64,
    /// The box the pieces go into or come out of, for items that come in
    /// boxes. Blank on a stock receipt makes new boxes.
    pub box_id: Option<String>,
    /// Size of the new boxes a stock receipt makes; `None` = the item's.
    pub pcs_per_box: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StockDocInput {
    pub kind: StockKind,
    pub date: String,
    /// Receiving, adjusted or (for transfers) source location.
    pub location_id: String,
    /// Destination of a transfer.
    pub to_location_id: Option<String>,
    /// Why stock was adjusted. Required for adjustments.
    pub reason: String,
    pub notes: String,
    /// Year in the serials of new boxes (`YYYY`); blank = this year.
    pub serial_year: String,
    pub lines: Vec<StockLineInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StockDocInfo {
    pub id: String,
    pub no: Option<String>,
    /// False while the number was given offline: it may still change.
    pub no_confirmed: bool,
    pub kind: StockKind,
    pub date: String,
    pub location_id: Option<String>,
    pub location_name: Option<String>,
    pub to_location_id: Option<String>,
    pub to_location_name: Option<String>,
    pub reason: Option<String>,
    pub notes: Option<String>,
    pub voided: bool,
    pub voided_at: Option<i64>,
    pub voided_by_name: Option<String>,
    pub void_reason: Option<String>,
    pub created_at: i64,
    pub created_by_name: Option<String>,
    /// e.g. "GLV-100 ×10, MSK-1 ×−2".
    pub summary: String,
    pub item_ids: Vec<String>,
    pub line_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StockDocLine {
    pub id: String,
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
pub struct StockDocDetail {
    pub doc: StockDocInfo,
    pub lines: Vec<StockDocLine>,
}

/// Stock of one item at one location with one expiry date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StockLot {
    pub item_id: String,
    pub item_code: String,
    pub item_name: String,
    pub unit: Option<String>,
    pub category: Option<String>,
    pub location_id: String,
    pub location_name: Option<String>,
    pub expiry: Option<String>,
    /// Days from today to expiry; negative once expired.
    pub days_left: Option<i64>,
    pub qty: i64,
    /// Boxes still full, boxes started, and the pieces left in those.
    pub full_boxes: i64,
    pub open_boxes: i64,
    pub open_pcs: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LocationQty {
    pub location_id: String,
    pub qty: i64,
    pub full_boxes: i64,
    pub open_boxes: i64,
    pub open_pcs: i64,
    /// The earliest expiry of this location's stock.
    pub next_expiry: Option<String>,
}

/// One item's stock across locations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ItemStock {
    pub item_id: String,
    pub code: String,
    pub name: String,
    pub unit: Option<String>,
    pub category: Option<String>,
    pub supplier_id: Option<String>,
    pub active: bool,
    pub tracks_expiry: bool,
    pub reorder_level: Option<i64>,
    pub on_hand: i64,
    /// Locations with non-zero stock.
    pub by_location: Vec<LocationQty>,
    /// At or below the reorder level.
    pub low: bool,
    /// Some lot is below zero (offline PCs took the same stock).
    pub negative: bool,
    /// Earliest expiry among lots in stock.
    pub next_expiry: Option<String>,
    /// Box serial code; `None` = no boxes.
    pub serial_code: Option<String>,
    pub pcs_per_box: Option<i64>,
    pub full_boxes: i64,
    pub open_boxes: i64,
    pub open_pcs: i64,
}

/// One line of an item's stock history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StockMove {
    pub doc_id: String,
    pub no: Option<String>,
    pub kind: StockKind,
    pub date: String,
    /// Where it moved; for a transfer, where it came from.
    pub location_id: String,
    pub location_name: Option<String>,
    /// A transfer's destination.
    pub to_location_id: Option<String>,
    pub to_location_name: Option<String>,
    pub expiry: Option<String>,
    /// Positive in, negative out; a transfer's moved qty is positive.
    pub qty: i64,
    pub created_at: i64,
    /// The purchase order of a PO receiving report.
    pub po_id: Option<String>,
    pub box_id: Option<String>,
    pub serial: Option<String>,
    /// A delivery on an invoice (else on a DR).
    pub invoice: bool,
}

// Writing -------------------------------------------------------------------

/// A validated document, ready to write.
struct Prepared {
    date: String,
    to: Option<String>,
    reason: Option<String>,
    lines: Vec<PreparedLine>,
    effects: Vec<Effect>,
}

struct PreparedLine {
    item: String,
    expiry: Option<String>,
    qty: i64,
    /// An existing box, or `None`.
    box_id: Option<String>,
    /// A box this stock receipt makes, of this item.
    new_box: Option<Serial>,
}

/// Net change a document makes to one lot (and box).
#[derive(Debug, Clone)]
pub(crate) struct Effect {
    pub item_id: String,
    pub location_id: String,
    pub expiry: Option<String>,
    pub qty: i64,
    pub box_id: Option<String>,
}

/// Whether the item has stock outside boxes at the location.
fn has_loose_stock(conn: &Connection, item_id: &str, location_id: &str) -> Result<bool> {
    Ok(conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM v_stock WHERE item_id = ?1 AND location_id = ?2
                            AND box_id IS NULL AND qty > 0)",
        )?
        .query_row([item_id, location_id], |r| r.get(0))?)
}

pub(crate) fn live_location(conn: &Connection, id: &str) -> Result<()> {
    let found: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM locations WHERE id = ?1 AND _deleted = 0)",
        [id],
        |r| r.get(0),
    )?;
    if found {
        Ok(())
    } else {
        Err(Error::Invalid("that location no longer exists".into()))
    }
}

/// Checks a document against the current data, without writing anything.
fn prepare(conn: &Connection, input: &StockDocInput, today: &str) -> Result<Prepared> {
    let kind = input.kind;
    if kind == StockKind::Delivery {
        return Err(Error::Invalid(
            "deliveries are recorded on DRs and invoices, in Accounting".into(),
        ));
    }
    if kind == StockKind::PoReceipt {
        return Err(Error::Invalid(
            "goods from a purchase order are received in Order".into(),
        ));
    }
    let date =
        parse_date(&input.date).ok_or_else(|| Error::Invalid("enter a valid date".into()))?;
    if date.as_str() > today {
        return Err(Error::Invalid("the date can't be in the future".into()));
    }
    live_location(conn, &input.location_id)?;
    let to = match (
        kind,
        input.to_location_id.as_deref().filter(|s| !s.is_empty()),
    ) {
        (StockKind::Transfer, None) => {
            return Err(Error::Invalid("choose where the stock goes".into()));
        }
        (StockKind::Transfer, Some(to)) if to == input.location_id => {
            return Err(Error::Invalid(
                "a transfer needs two different locations".into(),
            ));
        }
        (StockKind::Transfer, Some(to)) => {
            live_location(conn, to)?;
            Some(to.to_string())
        }
        _ => None,
    };
    let reason = text_opt(&input.reason);
    if kind == StockKind::Adjustment && reason.is_none() {
        return Err(Error::Invalid("enter a reason for the adjustment".into()));
    }
    if input.lines.is_empty() {
        return Err(Error::Invalid("add at least one item".into()));
    }
    if input.lines.len() > MAX_LINES {
        return Err(Error::Invalid(format!(
            "a document can have at most {MAX_LINES} lines"
        )));
    }

    let mut lines = Vec::with_capacity(input.lines.len());
    type Key = (String, String, Option<String>, Option<String>);
    let mut effects: BTreeMap<Key, i64> = BTreeMap::new();
    for line in &input.lines {
        let (code, tracks): (String, bool) = conn
            .query_row(
                "SELECT COALESCE(code, ''), COALESCE(tracks_expiry, 0) FROM items
                 WHERE id = ?1 AND _deleted = 0",
                [&line.item_id],
                |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
            )
            .optional()?
            .ok_or_else(|| Error::Invalid("one of the items no longer exists".into()))?;
        if line.qty == 0 {
            return Err(Error::Invalid(format!("enter a quantity for {code}")));
        }
        if line.qty < 0 && kind != StockKind::Adjustment {
            return Err(Error::Invalid(format!(
                "the quantity of {code} must be positive"
            )));
        }
        let expiry = match line
            .expiry
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(text) => Some(parse_date(text).ok_or_else(|| {
                Error::Invalid(format!("the expiry date of {code} is not a valid date"))
            })?),
            None => None,
        };
        let incoming = kind != StockKind::Transfer && line.qty > 0;
        // Items in boxes name the box; a stock receipt without one makes
        // new boxes.
        let serial = boxes::serial_of(conn, &line.item_id)?;
        let box_id = line
            .box_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let mut box_lot = None;
        if let Some(b) = box_id {
            let (item, _) = boxes::box_row(conn, b)?
                .ok_or_else(|| Error::Invalid("that box no longer exists".into()))?;
            if item != line.item_id {
                return Err(Error::Invalid(format!("that box doesn't hold {code}")));
            }
            box_lot = boxes::box_expiry(conn, b)?;
        } else if let Some(s) = &serial {
            if kind == StockKind::Receipt {
                let size = s.size(line.pcs_per_box)?;
                boxes::check_size(Some(size), &code)?;
                let expiry = match (tracks, expiry) {
                    (true, None) => {
                        return Err(Error::Invalid(format!("enter the expiry date of {code}")));
                    }
                    (true, e) => e,
                    (false, _) => None,
                };
                *effects
                    .entry((
                        line.item_id.clone(),
                        input.location_id.clone(),
                        expiry.clone(),
                        None,
                    ))
                    .or_default() += line.qty;
                for pcs in boxes::box_sizes(line.qty, size) {
                    lines.push(PreparedLine {
                        item: line.item_id.clone(),
                        expiry: expiry.clone(),
                        qty: pcs,
                        box_id: None,
                        new_box: Some(s.clone()),
                    });
                }
                if lines.len() > boxes::MAX_BOXES {
                    return Err(Error::Invalid(format!(
                        "a document can make at most {} boxes",
                        boxes::MAX_BOXES
                    )));
                }
                continue;
            }
            if line.qty > 0 && kind == StockKind::Adjustment
                || !has_loose_stock(conn, &line.item_id, &input.location_id)?
            {
                return Err(Error::Invalid(format!("choose the box of {code}")));
            }
        }
        // Incoming stock gets an expiry exactly when the item tracks one;
        // outgoing stock names the lot it comes from, as it was received.
        // A box keeps the expiry it came in with.
        let expiry = match (box_lot, incoming, tracks, expiry) {
            (Some(lot), ..) => Some(lot),
            (None, true, true, None) => {
                return Err(Error::Invalid(format!("enter the expiry date of {code}")));
            }
            (None, true, false, _) => None,
            (None, _, _, e) => e,
        };
        let box_id = box_id.map(String::from);
        let key = |loc: &str| {
            (
                line.item_id.clone(),
                loc.to_string(),
                expiry.clone(),
                box_id.clone(),
            )
        };
        match &to {
            Some(to) => {
                *effects.entry(key(&input.location_id)).or_default() -= line.qty;
                *effects.entry(key(to)).or_default() += line.qty;
            }
            None => *effects.entry(key(&input.location_id)).or_default() += line.qty,
        }
        lines.push(PreparedLine {
            item: line.item_id.clone(),
            expiry,
            qty: line.qty,
            box_id,
            new_box: None,
        });
    }
    let effects = effects
        .into_iter()
        .map(|((item_id, location_id, expiry, box_id), qty)| Effect {
            item_id,
            location_id,
            expiry,
            qty,
            box_id,
        })
        .collect();
    Ok(Prepared {
        date,
        to,
        reason,
        lines,
        effects,
    })
}

/// Checks a document as [`create`] would, without writing. Call it before
/// reserving a number, so a refused document doesn't use one up.
pub fn validate(conn: &Connection, input: &StockDocInput, today: &str) -> Result<()> {
    let p = prepare(conn, input, today)?;
    check_not_negative(conn, &p.effects, "")
}

/// Checks a document as [`create`] would and returns the serials of the
/// boxes it makes. Call it before reserving them and the number.
pub fn serials_for_doc(
    conn: &Connection,
    input: &StockDocInput,
    today: &str,
    now_ms: i64,
) -> Result<Vec<SerialNeed>> {
    let p = prepare(conn, input, today)?;
    check_not_negative(conn, &p.effects, "")?;
    let year = serial_year(input, now_ms)?;
    Ok(boxes::needs_for(
        p.lines.iter().filter_map(|l| l.new_box.as_ref()),
        &year,
    ))
}

fn serial_year(input: &StockDocInput, now_ms: i64) -> Result<String> {
    Ok(boxes::check_year(&input.serial_year)?.unwrap_or_else(|| boxes::this_year(now_ms)))
}

/// Posts a stock document with the number reserved for it (`res.ref_id` is
/// the new document's id). New boxes take their serials from `serials`.
/// Returns the id.
pub fn create(
    tx: &mut WriteTx,
    input: &StockDocInput,
    res: &Reservation,
    serials: &mut Serials,
) -> Result<String> {
    if res.series != input.kind.series() {
        return Err(Error::Invalid(
            "wrong number series for this document".into(),
        ));
    }
    let today = manila_date_string(tx.now_ms());
    let year = serial_year(input, tx.now_ms())?;
    let p = prepare(tx.conn(), input, &today)?;
    check_not_negative(tx.conn(), &p.effects, "")?;
    let id = tx.insert(
        STOCK_DOCS,
        json!({
            "id": res.ref_id,
            "kind": input.kind.as_str(),
            "date": p.date,
            "location_id": input.location_id,
            "to_location_id": p.to,
            "reason": p.reason,
            "notes": text_opt(&input.notes),
            "status": "posted",
        }),
    )?;
    for (i, l) in p.lines.into_iter().enumerate() {
        let box_id = match &l.new_box {
            Some(serial) => Some(boxes::create_box(
                tx,
                serials,
                serial,
                &year,
                json!({ "item_id": l.item, "doc_id": id, "pcs": l.qty }),
            )?),
            None => l.box_id,
        };
        tx.insert(
            STOCK_DOC_LINES,
            json!({
                "doc_id": id,
                "line_no": i as i64 + 1,
                "item_id": l.item,
                "expiry": l.expiry,
                "qty": l.qty,
                "box_id": box_id,
            }),
        )?;
    }
    numbering::assign(tx, res, STOCK_DOCS)?;
    Ok(id)
}

/// Voids a posted document: its stock movements no longer count.
pub fn void(tx: &mut WriteTx, id: &str, reason: &str) -> Result<()> {
    let row = tx
        .get(STOCK_DOCS, id)?
        .ok_or_else(|| Error::Invalid("that document no longer exists".into()))?;
    if row["status"].as_str() == Some("void") {
        return Err(Error::Invalid("that document is already void".into()));
    }
    let reason = text_opt(reason)
        .ok_or_else(|| Error::Invalid("enter why the document is voided".into()))?;
    let undo: Vec<Effect> = doc_effect(tx.conn(), id)?
        .into_iter()
        .map(|e| Effect { qty: -e.qty, ..e })
        .collect();
    check_not_negative(tx.conn(), &undo, "can't void: ")?;
    let (now, user) = (tx.now_ms(), tx.actor().user_id.clone());
    tx.update(
        STOCK_DOCS,
        id,
        json!({
            "status": "void",
            "voided_at": now,
            "voided_by": user,
            "void_reason": reason,
        }),
    )?;
    // Boxes a voided stock receipt made are empty now.
    boxes::drop_unused_boxes_of_doc(tx, id)
}

/// Net change a posted document (stock document, delivered DR or PO
/// receiving report) makes to each lot and box.
pub(crate) fn doc_effect(conn: &Connection, doc_id: &str) -> Result<Vec<Effect>> {
    let mut stmt = conn.prepare_cached(
        "SELECT item_id, location_id, expiry, SUM(qty), box_id FROM v_stock_moves
         WHERE doc_id = ?1 GROUP BY item_id, location_id, expiry, box_id",
    )?;
    let rows = stmt.query_map([doc_id], |r| {
        Ok(Effect {
            item_id: r.get(0)?,
            location_id: r.get(1)?,
            expiry: r.get(2)?,
            qty: r.get(3)?,
            box_id: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Fails if applying `effects` would take a lot (or a box) below zero.
pub(crate) fn check_not_negative(
    conn: &Connection,
    effects: &[Effect],
    prefix: &str,
) -> Result<()> {
    for e in effects.iter().filter(|e| e.qty < 0) {
        let before: i64 = conn
            .prepare_cached(
                "SELECT COALESCE(SUM(qty), 0) FROM v_stock
                 WHERE item_id = ?1 AND location_id = ?2 AND expiry IS ?3 AND box_id IS ?4",
            )?
            .query_row((&e.item_id, &e.location_id, &e.expiry, &e.box_id), |r| {
                r.get(0)
            })?;
        if before + e.qty >= 0 {
            continue;
        }
        let (code, unit): (Option<String>, Option<String>) = conn.query_row(
            "SELECT code, unit FROM items WHERE id = ?1",
            [&e.item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let place: Option<String> = conn
            .query_row(
                "SELECT name FROM locations WHERE id = ?1",
                [&e.location_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let serial = match &e.box_id {
            Some(b) => boxes::serial_no(conn, b)?,
            None => None,
        };
        let lot = match (&serial, &e.expiry) {
            (Some(s), _) => format!(" in box {s}"),
            (None, Some(x)) => format!(" (expiry {x})"),
            (None, None) => String::new(),
        };
        let unit = unit.map(|u| format!(" {u}")).unwrap_or_default();
        return Err(Error::Invalid(format!(
            "{prefix}not enough {}{lot} at {}: {}{unit} on hand, {}{unit} needed",
            code.unwrap_or_default(),
            place.unwrap_or_else(|| "that location".into()),
            before,
            -e.qty,
        )));
    }
    Ok(())
}

/// Whether any stock document, DR or PO receiving report (even a voided or
/// cancelled one) uses the location.
pub fn location_in_use(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM stock_docs WHERE _deleted = 0
                        AND (location_id = ?1 OR to_location_id = ?1))
             OR EXISTS (SELECT 1 FROM delivery_receipts WHERE _deleted = 0
                        AND from_location_id = ?1)
             OR EXISTS (SELECT 1 FROM po_receipts WHERE _deleted = 0 AND location_id = ?1)",
        [id],
        |r| r.get(0),
    )?)
}

/// Whether any stock document, booking, DR or PO (in any
/// status) has a line for the item.
pub fn item_in_use(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM stock_doc_lines l
                        JOIN stock_docs d ON d.id = l.doc_id AND d._deleted = 0
                        WHERE l._deleted = 0 AND l.item_id = ?1)
             OR EXISTS (SELECT 1 FROM booking_lines l
                        JOIN bookings d ON d.id = l.booking_id AND d._deleted = 0
                        WHERE l._deleted = 0 AND l.item_id = ?1)
             OR EXISTS (SELECT 1 FROM dr_lines l
                        JOIN delivery_receipts d ON d.id = l.dr_id AND d._deleted = 0
                        WHERE l._deleted = 0 AND l.item_id = ?1)
             OR EXISTS (SELECT 1 FROM po_lines l
                        JOIN purchase_orders d ON d.id = l.po_id AND d._deleted = 0
                        WHERE l._deleted = 0 AND l.item_id = ?1)",
        [id],
        |r| r.get(0),
    )?)
}

// Reading -------------------------------------------------------------------

const DOC_SELECT: &str = "
    SELECT d.id, n.no, COALESCE(n.confirmed, 1), d.kind, d.date, d.location_id, lf.name,
           d.to_location_id, lt.name, d.reason, d.notes, COALESCE(d.status, 'posted'),
           d.voided_at, uv.display_name, d.void_reason, d.created_at, uc.display_name
    FROM stock_docs d
    LEFT JOIN doc_numbers n ON n.ref_id = d.id AND n.ref_tbl = 'stock_docs' AND n._deleted = 0
    LEFT JOIN locations lf ON lf.id = d.location_id
    LEFT JOIN locations lt ON lt.id = d.to_location_id
    LEFT JOIN users uv ON uv.id = d.voided_by
    LEFT JOIN users uc ON uc.id = d.created_by
    WHERE d._deleted = 0";

fn doc_from_row(r: &rusqlite::Row) -> rusqlite::Result<Option<StockDocInfo>> {
    let Some(kind) = StockKind::parse(&r.get::<_, Option<String>>(3)?.unwrap_or_default()) else {
        return Ok(None);
    };
    Ok(Some(StockDocInfo {
        id: r.get(0)?,
        no: r.get(1)?,
        no_confirmed: r.get::<_, i64>(2)? != 0,
        kind,
        date: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        location_id: r.get(5)?,
        location_name: r.get(6)?,
        to_location_id: r.get(7)?,
        to_location_name: r.get(8)?,
        reason: r.get(9)?,
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
    }))
}

fn lines_of(conn: &Connection, doc_id: Option<&str>) -> Result<Vec<(String, StockDocLine)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT l.doc_id, l.id, l.item_id, i.code, i.name, i.unit, l.expiry, COALESCE(l.qty, 0),
                l.box_id, bn.no
         FROM stock_doc_lines l LEFT JOIN items i ON i.id = l.item_id
         LEFT JOIN doc_numbers bn ON bn.ref_id = l.box_id AND bn.ref_tbl = 'boxes'
                                 AND bn._deleted = 0
         WHERE l._deleted = 0 AND (?1 IS NULL OR l.doc_id = ?1)
         ORDER BY l.doc_id, l.line_no, l.id",
    )?;
    let rows = stmt.query_map([doc_id], |r| {
        Ok((
            r.get(0)?,
            StockDocLine {
                id: r.get(1)?,
                item_id: r.get(2)?,
                item_code: r.get(3)?,
                item_name: r.get(4)?,
                unit: r.get(5)?,
                expiry: r.get(6)?,
                qty: r.get(7)?,
                box_id: r.get(8)?,
                serial: r.get(9)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn summarize(doc: &mut StockDocInfo, lines: &[StockDocLine]) {
    const SHOWN: usize = 3;
    let mut parts: Vec<String> = lines
        .iter()
        .take(SHOWN)
        .map(|l| {
            let code = l.item_code.as_deref().unwrap_or("?");
            match l.qty {
                q if q < 0 => format!("{code} ×−{}", -q),
                q => format!("{code} ×{q}"),
            }
        })
        .collect();
    if lines.len() > SHOWN {
        parts.push(format!("+{} more", lines.len() - SHOWN));
    }
    doc.summary = parts.join(", ");
    doc.item_ids = lines.iter().map(|l| l.item_id.clone()).collect();
    doc.item_ids.sort();
    doc.item_ids.dedup();
    doc.line_count = lines.len() as i64;
}

/// Every stock document, newest first.
pub fn list_docs(conn: &Connection) -> Result<Vec<StockDocInfo>> {
    let mut by_doc: HashMap<String, Vec<StockDocLine>> = HashMap::new();
    for (doc, line) in lines_of(conn, None)? {
        by_doc.entry(doc).or_default().push(line);
    }
    let mut stmt = conn.prepare(&format!(
        "{DOC_SELECT} ORDER BY d.date DESC, d.created_at DESC, d.id DESC"
    ))?;
    let rows = stmt.query_map([], doc_from_row)?;
    let mut out = Vec::new();
    for row in rows {
        if let Some(mut doc) = row? {
            let lines = by_doc.remove(&doc.id).unwrap_or_default();
            summarize(&mut doc, &lines);
            out.push(doc);
        }
    }
    Ok(out)
}

pub fn get_doc(conn: &Connection, id: &str) -> Result<Option<StockDocDetail>> {
    let doc = conn
        .query_row(&format!("{DOC_SELECT} AND d.id = ?1"), [id], doc_from_row)
        .optional()?
        .flatten();
    let Some(mut doc) = doc else {
        return Ok(None);
    };
    let lines: Vec<StockDocLine> = lines_of(conn, Some(id))?
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    summarize(&mut doc, &lines);
    Ok(Some(StockDocDetail { doc, lines }))
}

/// Non-zero lots of live items, optionally of one item. `today` gives
/// `days_left`.
pub fn lots(conn: &Connection, item_id: Option<&str>, today: &str) -> Result<Vec<StockLot>> {
    let mut stmt = conn.prepare_cached(
        "SELECT s.item_id, i.code, i.name, i.unit, i.category, s.location_id, l.name,
                s.expiry, SUM(s.qty),
                SUM(s.box_id IS NOT NULL AND s.qty > 0 AND s.qty >= COALESCE(b.pcs, 0)),
                SUM(s.box_id IS NOT NULL AND s.qty > 0 AND s.qty < COALESCE(b.pcs, 0)),
                SUM(CASE WHEN s.box_id IS NOT NULL AND s.qty > 0 AND s.qty < COALESCE(b.pcs, 0)
                         THEN s.qty ELSE 0 END)
         FROM v_stock s
         JOIN items i ON i.id = s.item_id AND i._deleted = 0
         LEFT JOIN boxes b ON b.id = s.box_id
         LEFT JOIN locations l ON l.id = s.location_id
         WHERE (?1 IS NULL OR s.item_id = ?1)
         GROUP BY s.item_id, s.location_id, s.expiry
         HAVING SUM(s.qty) <> 0
         ORDER BY i.code COLLATE NOCASE, i.id, l.sort, l.name COLLATE NOCASE,
                  s.location_id, s.expiry IS NULL, s.expiry",
    )?;
    let rows = stmt.query_map([item_id], |r| {
        let expiry: Option<String> = r.get(7)?;
        Ok(StockLot {
            item_id: r.get(0)?,
            item_code: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            item_name: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            unit: r.get(3)?,
            category: r.get(4)?,
            location_id: r.get(5)?,
            location_name: r.get(6)?,
            days_left: expiry.as_deref().and_then(|e| days_between(today, e)),
            expiry,
            qty: r.get(8)?,
            full_boxes: r.get(9)?,
            open_boxes: r.get(10)?,
            open_pcs: r.get(11)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Stock of every live item, including items with none.
pub fn item_stock(conn: &Connection, today: &str) -> Result<Vec<ItemStock>> {
    let mut by_item: HashMap<String, Vec<StockLot>> = HashMap::new();
    for lot in lots(conn, None, today)? {
        by_item.entry(lot.item_id.clone()).or_default().push(lot);
    }
    // A box below zero can hide inside a lot that isn't.
    let negative_boxes: Vec<String> = {
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT item_id FROM v_stock WHERE box_id IS NOT NULL AND qty < 0",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut stmt = conn.prepare(
        "SELECT id, code, name, unit, category, supplier_id, COALESCE(active, 1),
                COALESCE(tracks_expiry, 0), reorder_level, serial_code, pcs_per_box
         FROM items WHERE _deleted = 0
         ORDER BY code COLLATE NOCASE, name COLLATE NOCASE, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ItemStock {
            item_id: r.get(0)?,
            code: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            name: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            unit: r.get(3)?,
            category: r.get(4)?,
            supplier_id: r.get(5)?,
            active: r.get::<_, i64>(6)? != 0,
            tracks_expiry: r.get::<_, i64>(7)? != 0,
            reorder_level: r.get(8)?,
            on_hand: 0,
            by_location: Vec::new(),
            low: false,
            negative: false,
            next_expiry: None,
            serial_code: r.get(9)?,
            pcs_per_box: r.get(10)?,
            full_boxes: 0,
            open_boxes: 0,
            open_pcs: 0,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        let mut s = row?;
        let lots = by_item.remove(&s.item_id).unwrap_or_default();
        // Lots come in location order (sort, name), so each location's
        // lots are together.
        let mut by_location: Vec<LocationQty> = Vec::new();
        for lot in &lots {
            if by_location.last().is_none_or(|l| l.location_id != lot.location_id) {
                by_location.push(LocationQty {
                    location_id: lot.location_id.clone(),
                    qty: 0,
                    full_boxes: 0,
                    open_boxes: 0,
                    open_pcs: 0,
                    next_expiry: None,
                });
            }
            let l = by_location.last_mut().expect("pushed above");
            l.qty += lot.qty;
            l.full_boxes += lot.full_boxes;
            l.open_boxes += lot.open_boxes;
            l.open_pcs += lot.open_pcs;
            if lot.qty > 0
                && let Some(e) = &lot.expiry
                && l.next_expiry.as_ref().is_none_or(|n| e < n)
            {
                l.next_expiry = Some(e.clone());
            }
        }
        by_location.retain(|l| l.qty != 0);
        s.by_location = by_location;
        s.on_hand = lots.iter().map(|l| l.qty).sum();
        s.full_boxes = lots.iter().map(|l| l.full_boxes).sum();
        s.open_boxes = lots.iter().map(|l| l.open_boxes).sum();
        s.open_pcs = lots.iter().map(|l| l.open_pcs).sum();
        s.negative = lots.iter().any(|l| l.qty < 0) || negative_boxes.contains(&s.item_id);
        s.low = s.active && s.reorder_level.is_some_and(|r| s.on_hand <= r);
        s.next_expiry = lots
            .iter()
            .filter(|l| l.qty > 0)
            .filter_map(|l| l.expiry.clone())
            .min();
        out.push(s);
    }
    Ok(out)
}

/// Active items at or below their reorder level.
pub fn low_stock(conn: &Connection, today: &str) -> Result<Vec<ItemStock>> {
    Ok(item_stock(conn, today)?
        .into_iter()
        .filter(|s| s.low)
        .collect())
}

/// Lots in stock that expire within `within_days` of `today` (or have
/// expired), soonest first.
pub fn expiring(conn: &Connection, today: &str, within_days: i64) -> Result<Vec<StockLot>> {
    let limit =
        add_days(today, within_days).ok_or_else(|| Error::Invalid("enter a valid date".into()))?;
    let mut out: Vec<StockLot> = lots(conn, None, today)?
        .into_iter()
        .filter(|l| l.qty > 0 && l.expiry.as_deref().is_some_and(|e| e <= limit.as_str()))
        .collect();
    out.sort_by(|a, b| {
        (&a.expiry, &a.item_code, &a.location_name).cmp(&(
            &b.expiry,
            &b.item_code,
            &b.location_name,
        ))
    });
    Ok(out)
}

/// An item's posted movements, newest first. `doc_id` is a stock
/// document, a DR for deliveries or a receiving report for PO receipts.
/// A transfer line is one row, from its source to `to_location_id`.
pub fn item_moves(conn: &Connection, item_id: &str) -> Result<Vec<StockMove>> {
    let mut stmt = conn.prepare_cached(
        "SELECT m.doc_id, COALESCE(n.no, rn.no), m.kind, m.date, m.location_id, l.name,
                m.expiry, CASE WHEN m.kind = 'transfer' THEN -m.qty ELSE m.qty END,
                COALESCE(d.created_at, r.delivered_at, r.created_at, p.created_at, 0) AS at,
                p.po_id, m.box_id, bn.no, COALESCE(r.vat, 0), lt.id, lt.name
         FROM v_stock_moves m
         LEFT JOIN doc_numbers bn ON bn.ref_id = m.box_id AND bn.ref_tbl = 'boxes'
                                 AND bn._deleted = 0
         LEFT JOIN stock_docs d ON d.id = m.doc_id
                               AND m.kind NOT IN ('delivery', 'po_receipt')
         LEFT JOIN stock_doc_lines sl ON sl.id = m.line_id AND d.id IS NOT NULL
         LEFT JOIN delivery_receipts r ON r.id = m.doc_id AND m.kind = 'delivery'
         LEFT JOIN po_receipts p ON p.id = m.doc_id AND m.kind = 'po_receipt'
         LEFT JOIN doc_numbers n ON n.ref_id = m.doc_id
                                AND n.ref_tbl IN ('stock_docs', 'po_receipts')
                                AND n._deleted = 0
         LEFT JOIN v_receipt_no rn ON rn.dr_id = r.id
         LEFT JOIN locations l ON l.id = m.location_id
         LEFT JOIN locations lt ON lt.id = d.to_location_id AND m.kind = 'transfer'
         WHERE m.item_id = ?1
           AND NOT (m.kind = 'transfer' AND m.location_id = d.to_location_id)
         ORDER BY m.date DESC, at DESC, m.doc_id DESC, COALESCE(sl.line_no, 0), m.line_id,
                  m.qty",
    )?;
    let rows = stmt.query_map([item_id], |r| {
        Ok((
            r.get::<_, String>(2)?,
            StockMove {
                doc_id: r.get(0)?,
                no: r.get(1)?,
                kind: StockKind::Receipt,
                date: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                location_id: r.get(4)?,
                location_name: r.get(5)?,
                to_location_id: r.get(13)?,
                to_location_name: r.get(14)?,
                expiry: r.get(6)?,
                qty: r.get(7)?,
                created_at: r.get(8)?,
                po_id: r.get(9)?,
                box_id: r.get(10)?,
                serial: r.get(11)?,
                invoice: r.get::<_, i64>(12)? != 0,
            },
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (kind, mut m) = row?;
        if let Some(k) = StockKind::parse(&kind) {
            m.kind = k;
            out.push(m);
        }
    }
    Ok(out)
}
