//! Reading the audit log. Entries are written by the write path only.

use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::time::manila_day_start_ms;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
pub const MAX_ENTRIES: u32 = 500;

#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AuditFilter {
    /// Matches the row id, action or changed values.
    pub q: Option<String>,
    pub user_id: Option<String>,
    pub tbl: Option<String>,
    /// Manila dates, inclusive.
    pub from: Option<String>,
    pub to: Option<String>,
    /// Defaults to and is capped at [`MAX_ENTRIES`].
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AuditEntry {
    pub id: String,
    pub at: i64,
    pub user_id: Option<String>,
    /// Display name, or `None` for system work (sync, numbering).
    pub user_name: Option<String>,
    /// insert | update | delete | login | logout
    pub action: String,
    pub tbl: String,
    pub row_id: String,
    /// JSON: new values for inserts, `{col: [old, new]}` for updates.
    pub changes_json: String,
}

pub fn list(conn: &Connection, f: &AuditFilter) -> Result<Vec<AuditEntry>> {
    let mut sql = String::from(
        "SELECT a.id, a.at, a.user_id, COALESCE(u.display_name, u.username),
                a.action, a.tbl, a.row_id, a.changes_json
         FROM audit_log a LEFT JOIN users u ON u.id = a.user_id
         WHERE a._deleted = 0",
    );
    let mut args: Vec<SqlValue> = Vec::new();
    let mut add = |cond: &str, v: SqlValue| {
        args.push(v);
        sql.push_str(&format!(
            " AND {}",
            cond.replace('?', &format!("?{}", args.len()))
        ));
    };
    if let Some(q) = f.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        let like = format!(
            "%{}%",
            q.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        add(
            "(a.row_id LIKE ? ESCAPE '\\' OR a.action LIKE ? ESCAPE '\\' OR a.changes_json LIKE ? ESCAPE '\\')",
            SqlValue::Text(like),
        );
    }
    if let Some(u) = f.user_id.as_deref().filter(|s| !s.is_empty()) {
        add("a.user_id = ?", SqlValue::Text(u.into()));
    }
    if let Some(t) = f.tbl.as_deref().filter(|s| !s.is_empty()) {
        add("a.tbl = ?", SqlValue::Text(t.into()));
    }
    if let Some(ms) = f.from.as_deref().and_then(manila_day_start_ms) {
        add("a.at >= ?", SqlValue::Integer(ms));
    }
    if let Some(ms) = f.to.as_deref().and_then(manila_day_start_ms) {
        add("a.at < ?", SqlValue::Integer(ms + DAY_MS));
    }
    let limit = f.limit.unwrap_or(MAX_ENTRIES).clamp(1, MAX_ENTRIES);
    sql.push_str(&format!(" ORDER BY a.at DESC, a.id DESC LIMIT {limit}"));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
        Ok(AuditEntry {
            id: r.get(0)?,
            at: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
            user_id: r.get(2)?,
            user_name: r.get(3)?,
            action: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            tbl: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            row_id: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            changes_json: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
