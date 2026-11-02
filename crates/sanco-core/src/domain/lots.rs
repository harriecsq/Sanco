//! Lots: one item's stock that arrived in one month.
//!
//! Stock is tracked per item, location and lot. A receipt's date picks its
//! lot; everything that moves stock afterwards names the lot by hand. A lot
//! has one expiry and a serial range (`serial_from`, `serial_to`), which is
//! free text for looking things up: nothing reads meaning into it. Both stay
//! editable, because they describe the lot and don't move stock.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::{date_opt, text_opt};
use crate::error::{Error, Result};
use crate::ids::derived_id;
use crate::repo::WriteTx;
use crate::time::days_between;

pub const LOTS: &str = "lots";

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `YYYY-MM` of a `YYYY-MM-DD` date.
pub fn month_of(date: &str) -> &str {
    date.get(..7).unwrap_or(date)
}

/// The id of an item's lot of a month.
pub fn lot_id(item_id: &str, month: &str) -> String {
    derived_id(item_id, &format!("lot/{month}"))
}

/// `Jul 2026`, for messages.
pub fn month_label(month: &str) -> String {
    let name = month
        .get(5..7)
        .and_then(|m| m.parse::<usize>().ok())
        .and_then(|m| MONTHS.get(m.wrapping_sub(1)));
    match (name, month.get(..4)) {
        (Some(name), Some(year)) => format!("{name} {year}"),
        _ => month.to_string(),
    }
}

/// A lot's expiry and month, if it exists for this item.
pub(crate) fn expiry_of(
    conn: &Connection,
    lot_id: &str,
    item_id: &str,
) -> Result<Option<Option<String>>> {
    Ok(conn
        .prepare_cached("SELECT expiry FROM lots WHERE id = ?1 AND item_id = ?2 AND _deleted = 0")?
        .query_row([lot_id, item_id], |r| r.get(0))
        .optional()?)
}

/// The label of a lot for messages, e.g. ` (lot Jul 2026)`; blank if unknown.
pub(crate) fn label(conn: &Connection, lot_id: Option<&str>) -> Result<String> {
    let Some(id) = lot_id else {
        return Ok(String::new());
    };
    let month: Option<Option<String>> = conn
        .prepare_cached("SELECT month FROM lots WHERE id = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()?;
    Ok(month
        .flatten()
        .map(|m| format!(" (lot {})", month_label(&m)))
        .unwrap_or_default())
}

/// What a receiving line says about its lot.
#[derive(Debug, Clone, Default)]
pub(crate) struct LotDetails {
    pub expiry: Option<String>,
    pub serial_from: Option<String>,
    pub serial_to: Option<String>,
}

impl LotDetails {
    /// Trimmed serial texts; `expiry` is checked by the caller.
    pub fn new(expiry: Option<String>, from: Option<&str>, to: Option<&str>) -> Self {
        Self {
            expiry,
            serial_from: text_opt(from.unwrap_or("")),
            serial_to: text_opt(to.unwrap_or("")),
        }
    }
}

/// Checks an incoming line's expiry against its item and lot: items that
/// track expiry need one, unless their lot already has it; others have none.
pub(crate) fn incoming_expiry(
    conn: &Connection,
    item_id: &str,
    code: &str,
    tracks: bool,
    month: &str,
    typed: Option<&str>,
) -> Result<Option<String>> {
    if !tracks {
        return Ok(None);
    }
    let typed = date_opt(typed, &format!("expiry date of {code}"))?;
    let known = expiry_of(conn, &lot_id(item_id, month), item_id)?.flatten();
    if typed.is_none() && known.is_none() {
        return Err(Error::Invalid(format!("enter the expiry date of {code}")));
    }
    Ok(typed)
}

/// Makes the item's lot of a month, or fills it in. What is left blank stays
/// as it is, so a second shipment in the month doesn't wipe the first's.
/// Returns the lot's id.
pub(crate) fn receive(
    tx: &mut WriteTx,
    item_id: &str,
    month: &str,
    details: &LotDetails,
) -> Result<String> {
    let id = lot_id(item_id, month);
    let mut data = json!({ "item_id": item_id, "month": month });
    if let Some(e) = &details.expiry {
        data["expiry"] = json!(e);
    }
    if let Some(s) = &details.serial_from {
        data["serial_from"] = json!(s);
    }
    if let Some(s) = &details.serial_to {
        data["serial_to"] = json!(s);
    }
    tx.upsert(LOTS, &id, data)?;
    Ok(id)
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LotInput {
    pub id: String,
    pub expiry: Option<String>,
    pub serial_from: String,
    pub serial_to: String,
}

/// Sets a lot's expiry and serial range, exactly as given.
pub fn update(tx: &mut WriteTx, input: &LotInput) -> Result<()> {
    if tx.get(LOTS, &input.id)?.is_none() {
        return Err(Error::Invalid("that lot no longer exists".into()));
    }
    tx.update(
        LOTS,
        &input.id,
        json!({
            "expiry": date_opt(input.expiry.as_deref(), "expiry date")?,
            "serial_from": text_opt(&input.serial_from),
            "serial_to": text_opt(&input.serial_to),
        }),
    )
    .map(drop)
}

/// What one location has of a lot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LotLocation {
    pub location_id: String,
    pub location_name: Option<String>,
    pub qty: i64,
}

/// One lot, in stock or not: a sold-out lot is still looked up for
/// complaints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LotInfo {
    pub id: String,
    pub item_id: String,
    pub item_code: String,
    pub item_name: String,
    pub unit: Option<String>,
    pub tracks_expiry: bool,
    /// `YYYY-MM`.
    pub month: String,
    pub expiry: Option<String>,
    pub days_left: Option<i64>,
    pub serial_from: Option<String>,
    pub serial_to: Option<String>,
    pub on_hand: i64,
    /// Locations with non-zero stock.
    pub by_location: Vec<LotLocation>,
}

/// Lots of live items, by item, oldest first.
pub fn list(conn: &Connection, today: &str) -> Result<Vec<LotInfo>> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.id, t.item_id, i.code, i.name, i.unit, COALESCE(i.tracks_expiry, 0), t.month,
                t.expiry, t.serial_from, t.serial_to
         FROM lots t JOIN items i ON i.id = t.item_id AND i._deleted = 0
         WHERE t._deleted = 0
         ORDER BY i.code COLLATE NOCASE, i.id, t.month",
    )?;
    let rows = stmt.query_map([], |r| {
        let expiry: Option<String> = r.get(7)?;
        Ok(LotInfo {
            id: r.get(0)?,
            item_id: r.get(1)?,
            item_code: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            item_name: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            unit: r.get(4)?,
            tracks_expiry: r.get::<_, i64>(5)? != 0,
            month: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            days_left: expiry.as_deref().and_then(|e| days_between(today, e)),
            expiry,
            serial_from: r.get(8)?,
            serial_to: r.get(9)?,
            on_hand: 0,
            by_location: Vec::new(),
        })
    })?;
    let mut lots: Vec<LotInfo> = rows.collect::<Result<_, _>>()?;
    let mut stmt = conn.prepare_cached(
        "SELECT s.lot_id, s.location_id, l.name, s.qty
         FROM v_stock s LEFT JOIN locations l ON l.id = s.location_id
         WHERE s.qty <> 0
         ORDER BY l.sort, l.name COLLATE NOCASE, s.location_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            LotLocation {
                location_id: r.get(1)?,
                location_name: r.get(2)?,
                qty: r.get(3)?,
            },
        ))
    })?;
    let mut by_lot: std::collections::HashMap<String, Vec<LotLocation>> = Default::default();
    for row in rows {
        let (lot, at) = row?;
        if let Some(lot) = lot {
            by_lot.entry(lot).or_default().push(at);
        }
    }
    for lot in &mut lots {
        lot.by_location = by_lot.remove(&lot.id).unwrap_or_default();
        lot.on_hand = lot.by_location.iter().map(|l| l.qty).sum();
    }
    Ok(lots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months() {
        assert_eq!(month_of("2026-07-30"), "2026-07");
        assert_eq!(month_label("2026-07"), "Jul 2026");
        assert_eq!(month_label("2026-13"), "2026-13");
        assert_eq!(lot_id("a", "2026-07"), lot_id("a", "2026-07"));
        assert_ne!(lot_id("a", "2026-07"), lot_id("a", "2026-11"));
    }
}
