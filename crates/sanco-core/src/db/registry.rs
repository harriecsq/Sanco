//! Registry of replicated tables: names, columns and value kinds.
//!
//! Sync and the write path are generic over these definitions. The SQL in
//! `migrations/` must match; `tests/repo.rs` checks that it does.

use std::collections::BTreeMap;

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde_json::Value;

use crate::error::{Error, Result};

/// A full row as column name → JSON scalar (null, integer or string).
/// `BTreeMap` keeps the key order canonical, so serialized rows compare equal.
pub type Row = BTreeMap<String, Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Int,
}

#[derive(Debug)]
pub struct Col {
    pub name: &'static str,
    pub kind: Kind,
}

#[derive(Debug)]
pub struct TableDef {
    pub name: &'static str,
    pub cols: &'static [Col],
}

const fn text(name: &'static str) -> Col {
    Col {
        name,
        kind: Kind::Text,
    }
}

const fn int(name: &'static str) -> Col {
    Col {
        name,
        kind: Kind::Int,
    }
}

/// Columns every replicated table has, managed by the write path.
pub const META_COLS: &[Col] = &[
    text("id"),
    int("_hlc"),
    text("_site"),
    text("_vv"),
    int("_deleted"),
    int("created_at"),
    text("created_by"),
    text("updated_by"),
];

pub const AUDIT_LOG: &str = "audit_log";
pub const CONFLICTS: &str = "conflicts";

pub static TABLES: &[TableDef] = &[
    TableDef {
        name: "devices",
        cols: &[
            text("name"),
            text("host"),
            int("port"),
            text("app_version"),
            int("schema_version"),
            int("last_seen"),
        ],
    },
    TableDef {
        name: "users",
        cols: &[
            text("username"),
            text("display_name"),
            text("pw_hash"),
            int("active"),
        ],
    },
    TableDef {
        name: "roles",
        cols: &[text("name"), text("perms_json")],
    },
    TableDef {
        name: "user_roles",
        cols: &[text("user_id"), text("role_id")],
    },
    TableDef {
        name: AUDIT_LOG,
        cols: &[
            int("at"),
            text("user_id"),
            text("action"),
            text("tbl"),
            text("row_id"),
            text("changes_json"),
        ],
    },
    TableDef {
        name: "todos",
        cols: &[
            text("title"),
            text("notes"),
            text("due"),
            text("assignee_id"),
            int("done"),
            text("ref_tbl"),
            text("ref_id"),
        ],
    },
    TableDef {
        name: CONFLICTS,
        cols: &[
            text("tbl"),
            text("row_id"),
            text("lost_json"),
            int("resolved"),
            text("resolved_by"),
        ],
    },
    TableDef {
        name: "settings",
        cols: &[text("key"), text("value")],
    },
];

pub fn table(name: &str) -> Result<&'static TableDef> {
    TABLES
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| Error::UnknownTable(name.to_string()))
}

pub fn is_meta(column: &str) -> bool {
    META_COLS.iter().any(|c| c.name == column)
}

impl TableDef {
    /// Meta columns followed by data columns.
    pub fn all_cols(&self) -> impl Iterator<Item = &Col> {
        META_COLS.iter().chain(self.cols.iter())
    }

    pub fn col(&self, name: &str) -> Option<&Col> {
        self.all_cols().find(|c| c.name == name)
    }

    fn column_list(&self) -> String {
        self.all_cols()
            .map(|c| format!("\"{}\"", c.name))
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn select_sql(&self) -> String {
        format!("SELECT {} FROM \"{}\"", self.column_list(), self.name)
    }

    /// Loads a row by id, including deleted ones.
    pub fn load(&self, conn: &Connection, id: &str) -> Result<Option<Row>> {
        let sql = format!("{} WHERE id = ?1", self.select_sql());
        let mut stmt = conn.prepare_cached(&sql)?;
        Ok(stmt.query_row([id], |r| self.row_from_sql(r)).optional()?)
    }

    /// Every row, ordered by id. Used for digests and tests.
    pub fn load_all(&self, conn: &Connection) -> Result<Vec<Row>> {
        let sql = format!("{} ORDER BY id", self.select_sql());
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt.query_map([], |r| self.row_from_sql(r))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn row_from_sql(&self, r: &rusqlite::Row) -> rusqlite::Result<Row> {
        let mut row = Row::new();
        for (i, col) in self.all_cols().enumerate() {
            let v = match r.get_ref(i)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => Value::from(n),
                ValueRef::Text(t) => Value::from(String::from_utf8_lossy(t).into_owned()),
                ValueRef::Real(f) => Value::from(f),
                ValueRef::Blob(_) => Value::Null,
            };
            row.insert(col.name.to_string(), v);
        }
        Ok(row)
    }

    /// Inserts or replaces a complete row. The row must already be validated.
    pub fn store(&self, conn: &Connection, row: &Row) -> Result<()> {
        let cols: Vec<&Col> = self.all_cols().collect();
        let placeholders = (1..=cols.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT OR REPLACE INTO \"{}\" ({}) VALUES ({placeholders})",
            self.name,
            self.column_list()
        );
        let values = cols
            .iter()
            .map(|c| to_sql(self, c, row.get(c.name).unwrap_or(&Value::Null)))
            .collect::<Result<Vec<_>>>()?;
        conn.prepare_cached(&sql)?
            .execute(params_from_iter(values))?;
        Ok(())
    }

    /// Checks one value against its column kind and normalizes it
    /// (booleans become 0/1).
    pub fn check_value(&self, column: &str, value: Value) -> Result<Value> {
        let col = self.col(column).ok_or_else(|| Error::UnknownColumn {
            table: self.name.to_string(),
            column: column.to_string(),
        })?;
        let bad = |reason: &str| Error::BadValue {
            table: self.name.to_string(),
            column: column.to_string(),
            reason: reason.to_string(),
        };
        match (col.kind, value) {
            (_, Value::Null) => Ok(Value::Null),
            (Kind::Text, v @ Value::String(_)) => Ok(v),
            (Kind::Int, Value::Bool(b)) => Ok(Value::from(i64::from(b))),
            (Kind::Int, Value::Number(n)) => n
                .as_i64()
                .map(Value::from)
                .ok_or_else(|| bad("expected an integer")),
            (Kind::Text, _) => Err(bad("expected text")),
            (Kind::Int, _) => Err(bad("expected an integer")),
        }
    }
}

fn to_sql(def: &TableDef, col: &Col, v: &Value) -> Result<SqlValue> {
    Ok(match def.check_value(col.name, v.clone())? {
        Value::Null => SqlValue::Null,
        Value::String(s) => SqlValue::Text(s),
        Value::Number(n) => SqlValue::Integer(n.as_i64().unwrap_or_default()),
        _ => SqlValue::Null,
    })
}
