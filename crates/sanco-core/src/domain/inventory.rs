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
}

impl StockKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StockKind::Receipt => "receipt",
            StockKind::Adjustment => "adjustment",
            StockKind::Transfer => "transfer",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "receipt" => Some(StockKind::Receipt),
            "adjustment" => Some(StockKind::Adjustment),
            "transfer" => Some(StockKind::Transfer),
            _ => None,
        }
    }

    /// The number series of this kind of document.
    pub fn series(self) -> &'static str {
        match self {
            StockKind::Receipt => series::STOCK_RECEIPT,
            StockKind::Adjustment => series::ADJUSTMENT,
            StockKind::Transfer => series::TRANSFER,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LocationQty {
    pub location_id: String,
    pub qty: i64,
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
}

/// One line of an item's stock history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StockMove {
    pub doc_id: String,
    pub no: Option<String>,
    pub kind: StockKind,
    pub date: String,
    pub location_id: String,
    pub location_name: Option<String>,
    pub expiry: Option<String>,
    /// Positive in, negative out.
    pub qty: i64,
    pub created_at: i64,
}

// Writing -------------------------------------------------------------------

/// A validated document, ready to write.
struct Prepared {
    date: String,
    to: Option<String>,
    reason: Option<String>,
    /// (item, expiry, qty) per line.
    lines: Vec<(String, Option<String>, i64)>,
    effects: Vec<Effect>,
}

/// Net change a document makes to one lot.
#[derive(Debug, Clone)]
struct Effect {
    item_id: String,
    location_id: String,
    expiry: Option<String>,
    qty: i64,
}

fn live_location(conn: &Connection, id: &str) -> Result<()> {
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
    let mut effects: BTreeMap<(String, String, Option<String>), i64> = BTreeMap::new();
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
        // Incoming stock gets an expiry exactly when the item tracks one;
        // outgoing stock names the lot it comes from, as it was received.
        let incoming = kind != StockKind::Transfer && line.qty > 0;
        let expiry = match (incoming, tracks, expiry) {
            (true, true, None) => {
                return Err(Error::Invalid(format!("enter the expiry date of {code}")));
            }
            (true, false, _) => None,
            (_, _, e) => e,
        };
        let key = |loc: &str| (line.item_id.clone(), loc.to_string(), expiry.clone());
        match &to {
            Some(to) => {
                *effects.entry(key(&input.location_id)).or_default() -= line.qty;
                *effects.entry(key(to)).or_default() += line.qty;
            }
            None => *effects.entry(key(&input.location_id)).or_default() += line.qty,
        }
        lines.push((line.item_id.clone(), expiry, line.qty));
    }
    let effects = effects
        .into_iter()
        .map(|((item_id, location_id, expiry), qty)| Effect {
            item_id,
            location_id,
            expiry,
            qty,
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

/// Posts a stock document with the number reserved for it (`res.ref_id` is
/// the new document's id). Returns the id.
pub fn create(tx: &mut WriteTx, input: &StockDocInput, res: &Reservation) -> Result<String> {
    if res.series != input.kind.series() {
        return Err(Error::Invalid(
            "wrong number series for this document".into(),
        ));
    }
    let today = manila_date_string(tx.now_ms());
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
    for (i, (item, expiry, qty)) in p.lines.into_iter().enumerate() {
        tx.insert(
            STOCK_DOC_LINES,
            json!({
                "doc_id": id,
                "line_no": i as i64 + 1,
                "item_id": item,
                "expiry": expiry,
                "qty": qty,
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
    )
    .map(drop)
}

fn doc_effect(conn: &Connection, doc_id: &str) -> Result<Vec<Effect>> {
    let mut stmt = conn.prepare_cached(
        "SELECT item_id, location_id, expiry, SUM(qty) FROM v_stock_moves
         WHERE doc_id = ?1 GROUP BY item_id, location_id, expiry",
    )?;
    let rows = stmt.query_map([doc_id], |r| {
        Ok(Effect {
            item_id: r.get(0)?,
            location_id: r.get(1)?,
            expiry: r.get(2)?,
            qty: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Fails if applying `effects` would take a lot below zero.
fn check_not_negative(conn: &Connection, effects: &[Effect], prefix: &str) -> Result<()> {
    for e in effects.iter().filter(|e| e.qty < 0) {
        let before: i64 = conn
            .prepare_cached(
                "SELECT COALESCE(SUM(qty), 0) FROM v_stock
                 WHERE item_id = ?1 AND location_id = ?2 AND expiry IS ?3",
            )?
            .query_row((&e.item_id, &e.location_id, &e.expiry), |r| r.get(0))?;
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
        let lot = match &e.expiry {
            Some(x) => format!(" (expiry {x})"),
            None => String::new(),
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

/// Whether any stock document (posted or void) uses the location.
pub fn location_in_use(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM stock_docs WHERE _deleted = 0
                        AND (location_id = ?1 OR to_location_id = ?1))",
        [id],
        |r| r.get(0),
    )?)
}

/// Whether any stock document (posted or void) has a line for the item.
pub fn item_in_use(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM stock_doc_lines l
                        JOIN stock_docs d ON d.id = l.doc_id AND d._deleted = 0
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
        "SELECT l.doc_id, l.id, l.item_id, i.code, i.name, i.unit, l.expiry, COALESCE(l.qty, 0)
         FROM stock_doc_lines l LEFT JOIN items i ON i.id = l.item_id
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
                s.expiry, s.qty
         FROM v_stock s
         JOIN items i ON i.id = s.item_id AND i._deleted = 0
         LEFT JOIN locations l ON l.id = s.location_id
         WHERE s.qty <> 0 AND (?1 IS NULL OR s.item_id = ?1)
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
    let mut stmt = conn.prepare(
        "SELECT id, code, name, unit, category, supplier_id, COALESCE(active, 1),
                COALESCE(tracks_expiry, 0), reorder_level
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
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        let mut s = row?;
        let lots = by_item.remove(&s.item_id).unwrap_or_default();
        let mut per_loc: BTreeMap<String, i64> = BTreeMap::new();
        for lot in &lots {
            *per_loc.entry(lot.location_id.clone()).or_default() += lot.qty;
        }
        // Keep the lots' location order (sort, name).
        let mut order: Vec<String> = lots.iter().map(|l| l.location_id.clone()).collect();
        order.dedup();
        s.by_location = order
            .into_iter()
            .map(|id| LocationQty {
                qty: per_loc[&id],
                location_id: id,
            })
            .filter(|l| l.qty != 0)
            .collect();
        s.on_hand = lots.iter().map(|l| l.qty).sum();
        s.negative = lots.iter().any(|l| l.qty < 0);
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

/// An item's posted movements, newest first.
pub fn item_moves(conn: &Connection, item_id: &str) -> Result<Vec<StockMove>> {
    let mut stmt = conn.prepare_cached(
        "SELECT m.doc_id, n.no, m.kind, m.date, m.location_id, l.name, m.expiry, m.qty,
                d.created_at
         FROM v_stock_moves m
         JOIN stock_docs d ON d.id = m.doc_id
         LEFT JOIN doc_numbers n ON n.ref_id = m.doc_id AND n.ref_tbl = 'stock_docs'
                                AND n._deleted = 0
         LEFT JOIN locations l ON l.id = m.location_id
         WHERE m.item_id = ?1
         ORDER BY m.date DESC, d.created_at DESC, m.doc_id DESC, m.qty",
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
                expiry: r.get(6)?,
                qty: r.get(7)?,
                created_at: r.get(8)?,
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
