//! Sync conflicts: versions that lost to a concurrent edit on another PC.
//! Nothing is lost silently; someone reviews each one and either keeps the
//! current version or puts the lost values back.

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::db::registry::{self, CONFLICTS, Row};
use crate::error::{Error, Result};
use crate::repo::WriteTx;

/// Columns never shown or restored.
const HIDDEN: &[&str] = &["pw_hash"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ConflictInfo {
    pub id: String,
    pub tbl: String,
    pub row_id: String,
    /// When the lost edit was made (UTC ms).
    pub lost_at: i64,
    /// Who made the lost edit.
    pub lost_by: Option<String>,
    pub resolved: bool,
    pub resolved_by: Option<String>,
    /// The row was deleted, so the lost values can't be put back.
    pub row_deleted: bool,
    /// Columns where the lost version differs from the current row.
    pub fields: Vec<FieldDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FieldDiff {
    pub column: String,
    pub lost: String,
    pub current: String,
}

pub fn list(conn: &Connection, include_resolved: bool) -> Result<Vec<ConflictInfo>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.tbl, c.row_id, c.lost_json, c.created_at, c.resolved,
                COALESCE(u.display_name, u.username, c.created_by),
                COALESCE(r.display_name, r.username, c.resolved_by)
         FROM conflicts c
         LEFT JOIN users u ON u.id = c.created_by
         LEFT JOIN users r ON r.id = c.resolved_by
         WHERE c._deleted = 0 AND (?1 OR COALESCE(c.resolved, 0) = 0)
         ORDER BY c.created_at DESC
         LIMIT 500",
    )?;
    let raw = stmt.query_map([include_resolved], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            r.get::<_, i64>(4)?,
            r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?,
        ))
    })?;
    let mut out = Vec::new();
    for item in raw {
        let (id, tbl, row_id, lost_json, lost_at, resolved, lost_by, resolved_by) = item?;
        let lost: Row = serde_json::from_str(&lost_json).unwrap_or_default();
        let current = registry::table(&tbl)
            .ok()
            .map(|def| def.load(conn, &row_id))
            .transpose()?
            .flatten();
        let row_deleted = current.as_ref().is_none_or(is_deleted);
        out.push(ConflictInfo {
            fields: diff(&tbl, &lost, current.as_ref()),
            id,
            tbl,
            row_id,
            lost_at,
            lost_by,
            resolved,
            resolved_by,
            row_deleted,
        });
    }
    Ok(out)
}

/// Marks a conflict reviewed. With `restore`, first writes the lost values
/// back onto the row, as a normal edit by the current user.
pub fn resolve(tx: &mut WriteTx, id: &str, restore: bool) -> Result<()> {
    let Some(conflict) = tx.get(CONFLICTS, id)? else {
        return Err(Error::Invalid("that conflict no longer exists".into()));
    };
    let text = |c: &str| {
        conflict
            .get(c)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    if restore {
        let (tbl, row_id) = (text("tbl"), text("row_id"));
        let def = registry::table(&tbl)?;
        let lost: Row = serde_json::from_str(&text("lost_json"))?;
        if tx.get(&tbl, &row_id)?.is_none() {
            return Err(Error::Invalid(
                "the row was deleted since, so the lost values can't be put back".into(),
            ));
        }
        let patch: Map<String, Value> = def
            .cols
            .iter()
            .filter(|c| !HIDDEN.contains(&c.name))
            .map(|c| (c.name.to_string(), lost.get(c.name).cloned().unwrap_or(Value::Null)))
            .collect();
        tx.update(&tbl, &row_id, Value::Object(patch))?;
    }
    let by = tx.actor().user_id.clone();
    tx.update(CONFLICTS, id, json!({ "resolved": 1, "resolved_by": by }))?;
    Ok(())
}

fn diff(tbl: &str, lost: &Row, current: Option<&Row>) -> Vec<FieldDiff> {
    let Ok(def) = registry::table(tbl) else {
        return Vec::new();
    };
    def.cols
        .iter()
        .filter(|c| !HIDDEN.contains(&c.name))
        .filter_map(|c| {
            let l = lost.get(c.name).unwrap_or(&Value::Null);
            let cur = current.and_then(|r| r.get(c.name)).unwrap_or(&Value::Null);
            (l != cur).then(|| FieldDiff {
                column: c.name.to_string(),
                lost: show(l),
                current: show(cur),
            })
        })
        .collect()
}

fn show(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn is_deleted(row: &Row) -> bool {
    row.get("_deleted").and_then(Value::as_i64).unwrap_or(0) != 0
}
