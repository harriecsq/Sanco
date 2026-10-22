//! Suppliers. Codes (`SUP-0001`) are shared numbers in series `SUP`.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::pricing::check_currency;
use crate::domain::{require_text, text_opt};
use crate::error::{Error, Result};
use crate::repo::WriteTx;
use crate::sync::numbering::{self, Reservation, series};

pub const SUPPLIERS: &str = "suppliers";

/// Taiwan, China, or anywhere else.
pub const COUNTRIES: [&str; 3] = ["TW", "CN", "OTHER"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SupplierInfo {
    pub id: String,
    pub code: Option<String>,
    pub code_confirmed: bool,
    pub name: String,
    /// TW | CN | OTHER
    pub country: String,
    /// Default currency of its POs and prices.
    pub currency: String,
    pub contact_person: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub active: bool,
    pub duplicate_name: bool,
    pub item_count: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SupplierInput {
    pub id: Option<String>,
    pub name: String,
    pub country: String,
    pub currency: String,
    pub contact_person: String,
    pub phone: String,
    pub email: String,
    pub address: String,
    pub notes: String,
    pub active: bool,
}

const SELECT: &str = "
    SELECT s.id, n.no, COALESCE(n.confirmed, 1), s.name, s.country, s.currency,
           s.contact_person, s.phone, s.email, s.address, s.notes, COALESCE(s.active, 1),
           EXISTS (SELECT 1 FROM suppliers d WHERE d._deleted = 0 AND d.id <> s.id
                   AND d.name = s.name COLLATE NOCASE),
           (SELECT COUNT(*) FROM items i WHERE i.supplier_id = s.id AND i._deleted = 0),
           s.created_at
    FROM suppliers s
    LEFT JOIN doc_numbers n ON n.ref_id = s.id AND n.series = 'SUP' AND n._deleted = 0
    WHERE s._deleted = 0";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<SupplierInfo> {
    Ok(SupplierInfo {
        id: r.get(0)?,
        code: r.get(1)?,
        code_confirmed: r.get::<_, i64>(2)? != 0,
        name: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        country: r
            .get::<_, Option<String>>(4)?
            .unwrap_or_else(|| "OTHER".into()),
        currency: r
            .get::<_, Option<String>>(5)?
            .unwrap_or_else(|| "USD".into()),
        contact_person: r.get(6)?,
        phone: r.get(7)?,
        email: r.get(8)?,
        address: r.get(9)?,
        notes: r.get(10)?,
        active: r.get::<_, i64>(11)? != 0,
        duplicate_name: r.get(12)?,
        item_count: r.get(13)?,
        created_at: r.get(14)?,
    })
}

pub fn list(conn: &Connection) -> Result<Vec<SupplierInfo>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY s.name COLLATE NOCASE, s.id"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<SupplierInfo>> {
    Ok(conn
        .query_row(&format!("{SELECT} AND s.id = ?1"), [id], from_row)
        .optional()?)
}

fn data(input: &SupplierInput) -> Result<Value> {
    let country = input.country.trim().to_ascii_uppercase();
    if !COUNTRIES.contains(&country.as_str()) {
        return Err(Error::Invalid("choose Taiwan, China or Other".into()));
    }
    Ok(json!({
        "name": require_text(&input.name, "a supplier name")?,
        "country": country,
        "currency": check_currency(&input.currency)?,
        "contact_person": text_opt(&input.contact_person),
        "phone": text_opt(&input.phone),
        "email": text_opt(&input.email),
        "address": text_opt(&input.address),
        "notes": text_opt(&input.notes),
        "active": input.active,
    }))
}

pub fn create(tx: &mut WriteTx, input: &SupplierInput, res: &Reservation) -> Result<String> {
    if res.series != series::SUPPLIER {
        return Err(Error::Invalid("wrong number series for a supplier".into()));
    }
    let mut v = data(input)?;
    v["id"] = json!(res.ref_id);
    let id = tx.insert(SUPPLIERS, v)?;
    numbering::assign(tx, res, SUPPLIERS)?;
    Ok(id)
}

pub fn update(tx: &mut WriteTx, input: &SupplierInput) -> Result<()> {
    let id = input.id.as_deref().unwrap_or_default();
    if tx.get(SUPPLIERS, id)?.is_none() {
        return Err(Error::Invalid("that supplier no longer exists".into()));
    }
    tx.update(SUPPLIERS, id, data(input)?).map(drop)
}

/// Deletes a supplier that no item points to.
pub fn delete(tx: &mut WriteTx, id: &str) -> Result<()> {
    if tx.get(SUPPLIERS, id)?.is_none() {
        return Err(Error::Invalid("that supplier no longer exists".into()));
    }
    let items: i64 = tx.conn().query_row(
        "SELECT COUNT(*) FROM items WHERE supplier_id = ?1 AND _deleted = 0",
        [id],
        |r| r.get(0),
    )?;
    if items > 0 {
        return Err(Error::Invalid(format!(
            "{items} item(s) still use this supplier; change them first or deactivate the supplier"
        )));
    }
    tx.delete(SUPPLIERS, id)
}
