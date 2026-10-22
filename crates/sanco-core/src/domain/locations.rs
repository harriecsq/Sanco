//! Stock locations. Office and Warehouse are seeded; more can be added.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::registry;
use crate::domain::{inventory, require_text, text_opt};
use crate::error::{Error, Result};
use crate::ids;
use crate::repo::WriteTx;

pub const LOCATIONS: &str = "locations";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LocationInfo {
    pub id: String,
    pub name: String,
    pub notes: Option<String>,
    pub sort: i64,
    pub active: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LocationInput {
    pub id: Option<String>,
    pub name: String,
    pub notes: String,
    pub sort: i64,
    pub active: bool,
}

pub fn list(conn: &Connection) -> Result<Vec<LocationInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, notes, COALESCE(sort, 0), COALESCE(active, 1) FROM locations
         WHERE _deleted = 0 ORDER BY sort, name COLLATE NOCASE, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(LocationInfo {
            id: r.get(0)?,
            name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            notes: r.get(2)?,
            sort: r.get(3)?,
            active: r.get::<_, i64>(4)? != 0,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Seeds Office and Warehouse with fixed ids, so seeding on several PCs
/// gives the same rows. A seed row that exists (even deleted) is left alone.
pub fn seed(tx: &mut WriteTx) -> Result<()> {
    let def = registry::table(LOCATIONS)?;
    for (key, name, sort) in [("office", "Office", 1), ("warehouse", "Warehouse", 2)] {
        let id = ids::derived_id("location", key);
        if def.load(tx.conn(), &id)?.is_none() {
            tx.insert(
                LOCATIONS,
                json!({ "id": id, "name": name, "sort": sort, "active": 1 }),
            )?;
        }
    }
    Ok(())
}

pub fn save(tx: &mut WriteTx, input: &LocationInput) -> Result<String> {
    let name = require_text(&input.name, "a location name")?;
    let taken: bool = tx.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM locations WHERE _deleted = 0
                        AND name = ?1 COLLATE NOCASE AND id IS NOT ?2)",
        (&name, &input.id),
        |r| r.get(0),
    )?;
    if taken {
        return Err(Error::Invalid(format!(
            "a location named {name} already exists"
        )));
    }
    let data = json!({
        "name": name,
        "notes": text_opt(&input.notes),
        "sort": input.sort,
        "active": input.active,
    });
    match &input.id {
        Some(id) => {
            if tx.get(LOCATIONS, id)?.is_none() {
                return Err(Error::Invalid("that location no longer exists".into()));
            }
            tx.update(LOCATIONS, id, data)?;
            Ok(id.clone())
        }
        None => tx.insert(LOCATIONS, data),
    }
}

/// Deletes a location that no stock document or DR uses, even a voided one.
pub fn delete(tx: &mut WriteTx, id: &str) -> Result<()> {
    if tx.get(LOCATIONS, id)?.is_none() {
        return Err(Error::Invalid("that location no longer exists".into()));
    }
    if inventory::location_in_use(tx.conn(), id)? {
        return Err(Error::Invalid(
            "this location has stock movements or DRs; mark it inactive instead".into(),
        ));
    }
    tx.delete(LOCATIONS, id)
}
