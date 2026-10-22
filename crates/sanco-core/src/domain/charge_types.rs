//! Cost-sheet charge types (Settings → Pricing). Each is per unit (a PHP
//! amount) or a percentage of the PHP base cost by default; an item's cost
//! sheet may override the mode per line.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::registry;
use crate::domain::pricing::ChargeMode;
use crate::domain::require_text;
use crate::error::{Error, Result};
use crate::ids;
use crate::repo::WriteTx;

pub const CHARGE_TYPES: &str = "charge_types";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ChargeTypeInfo {
    pub id: String,
    pub name: String,
    pub mode: ChargeMode,
    pub sort: i64,
    pub active: bool,
    /// Cost sheets with a value for this charge.
    pub used_by: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ChargeTypeInput {
    pub id: Option<String>,
    pub name: String,
    pub mode: ChargeMode,
    pub sort: i64,
    pub active: bool,
}

pub fn list(conn: &Connection) -> Result<Vec<ChargeTypeInfo>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, t.mode, COALESCE(t.sort, 0), COALESCE(t.active, 1),
                (SELECT COUNT(*) FROM item_cost_charges l
                 JOIN items i ON i.id = l.item_id AND i._deleted = 0
                 WHERE l.charge_type_id = t.id AND l._deleted = 0 AND l.value IS NOT NULL)
         FROM charge_types t WHERE t._deleted = 0
         ORDER BY t.sort, t.name COLLATE NOCASE, t.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ChargeTypeInfo {
            id: r.get(0)?,
            name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            mode: ChargeMode::parse(r.get::<_, Option<String>>(2)?.as_deref()),
            sort: r.get(3)?,
            active: r.get::<_, i64>(4)? != 0,
            used_by: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Seeds the default charge types with fixed ids (see `locations::seed`).
pub fn seed(tx: &mut WriteTx) -> Result<()> {
    let def = registry::table(CHARGE_TYPES)?;
    let defaults = [
        ("freight", "Freight", ChargeMode::PerUnit),
        ("duty", "Customs duty", ChargeMode::Pct),
        ("brokerage", "Brokerage", ChargeMode::PerUnit),
        ("bank_fees", "Bank fees", ChargeMode::PerUnit),
        ("import_vat", "Import VAT", ChargeMode::Pct),
    ];
    for (sort, (key, name, mode)) in defaults.into_iter().enumerate() {
        let id = ids::derived_id("charge_type", key);
        if def.load(tx.conn(), &id)?.is_none() {
            tx.insert(
                CHARGE_TYPES,
                json!({
                    "id": id, "name": name, "mode": mode.as_str(),
                    "sort": sort as i64 + 1, "active": 1,
                }),
            )?;
        }
    }
    Ok(())
}

pub fn save(tx: &mut WriteTx, input: &ChargeTypeInput) -> Result<String> {
    let name = require_text(&input.name, "a charge name")?;
    let taken: bool = tx.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM charge_types WHERE _deleted = 0
                        AND name = ?1 COLLATE NOCASE AND id IS NOT ?2)",
        (&name, &input.id),
        |r| r.get(0),
    )?;
    if taken {
        return Err(Error::Invalid(format!("a charge named {name} already exists")));
    }
    let data = json!({
        "name": name,
        "mode": input.mode.as_str(),
        "sort": input.sort,
        "active": input.active,
    });
    match &input.id {
        Some(id) => {
            if tx.get(CHARGE_TYPES, id)?.is_none() {
                return Err(Error::Invalid("that charge no longer exists".into()));
            }
            tx.update(CHARGE_TYPES, id, data)?;
            Ok(id.clone())
        }
        None => tx.insert(CHARGE_TYPES, data),
    }
}

/// Deletes a charge type no cost sheet uses. Used ones can be deactivated.
pub fn delete(tx: &mut WriteTx, id: &str) -> Result<()> {
    if tx.get(CHARGE_TYPES, id)?.is_none() {
        return Err(Error::Invalid("that charge no longer exists".into()));
    }
    let used: i64 = tx.conn().query_row(
        "SELECT COUNT(*) FROM item_cost_charges l
         JOIN items i ON i.id = l.item_id AND i._deleted = 0
         WHERE l.charge_type_id = ?1 AND l._deleted = 0 AND l.value IS NOT NULL",
        [id],
        |r| r.get(0),
    )?;
    if used > 0 {
        return Err(Error::Invalid(format!(
            "{used} cost sheet(s) use this charge; deactivate it instead"
        )));
    }
    tx.delete(CHARGE_TYPES, id)
}
