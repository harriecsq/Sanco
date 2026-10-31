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
pub const DOC_NUMBERS: &str = "doc_numbers";

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
    TableDef {
        name: DOC_NUMBERS,
        cols: &[
            text("series"),
            text("period"),
            int("seq"),
            text("no"),
            text("ref_tbl"),
            text("ref_id"),
            text("site"),
            int("assigned_at"),
            text("prev_no"),
            int("confirmed"),
            int("manual"),
        ],
    },
    TableDef {
        name: "clients",
        cols: &[
            text("name"),
            text("contact_person"),
            text("phone"),
            text("email"),
            text("tin"),
            text("address"),
            int("terms_days"),
            int("margin_tier_bp"),
            int("credit_limit_c"),
            text("notes"),
            int("active"),
        ],
    },
    TableDef {
        name: "suppliers",
        cols: &[
            text("name"),
            text("country"),
            text("currency"),
            text("contact_person"),
            text("phone"),
            text("email"),
            text("address"),
            text("notes"),
            int("active"),
        ],
    },
    TableDef {
        name: "locations",
        cols: &[text("name"), text("notes"), int("sort"), int("active")],
    },
    TableDef {
        name: "charge_types",
        cols: &[text("name"), text("mode"), int("sort"), int("active")],
    },
    TableDef {
        name: "items",
        cols: &[
            text("code"),
            text("name"),
            text("unit"),
            text("category"),
            text("supplier_id"),
            int("tracks_expiry"),
            int("reorder_level"),
            int("default_margin_bp"),
            text("notes"),
            int("active"),
            // Added in 0011: boxes and serials.
            text("serial_code"),
            int("serial_digits"),
            int("pcs_per_box"),
        ],
    },
    TableDef {
        name: "item_costs",
        cols: &[
            text("item_id"),
            text("currency"),
            int("supplier_price_minor"),
            text("fx_rate"),
            text("notes"),
            int("shipment_qty"),
        ],
    },
    TableDef {
        name: "item_cost_charges",
        cols: &[
            text("item_id"),
            text("charge_type_id"),
            text("mode"),
            int("value"),
        ],
    },
    TableDef {
        name: "client_item_rates",
        cols: &[
            text("client_id"),
            text("item_id"),
            int("price_c"),
            int("vat"),
            text("notes"),
        ],
    },
    TableDef {
        name: "stock_docs",
        cols: &[
            text("kind"),
            text("date"),
            text("location_id"),
            text("to_location_id"),
            text("reason"),
            text("notes"),
            text("status"),
            int("voided_at"),
            text("voided_by"),
            text("void_reason"),
        ],
    },
    TableDef {
        name: "stock_doc_lines",
        cols: &[
            text("doc_id"),
            int("line_no"),
            text("item_id"),
            text("expiry"),
            int("qty"),
            // Added in 0011.
            text("box_id"),
        ],
    },
    // Business Development was removed from the app (2026-10-27). Its tables
    // stay replicated (migrations are additive only, and older versions
    // still sync them) but nothing reads or writes them.
    TableDef {
        name: "bd_requests",
        cols: &[
            text("client_id"),
            text("prospect_name"),
            text("contact"),
            text("date"),
            text("status"),
            text("follow_up"),
            int("vat"),
            text("notes"),
            text("lost_reason"),
        ],
    },
    TableDef {
        name: "bd_lines",
        cols: &[
            text("request_id"),
            int("line_no"),
            text("item_id"),
            text("description"),
            int("qty"),
            int("price_c"),
        ],
    },
    TableDef {
        name: "bookings",
        cols: &[
            text("client_id"),
            // Unused since Business Development was removed.
            text("bd_request_id"),
            text("date"),
            text("status"),
            int("vat"),
            int("vat_bp"),
            text("client_ref"),
            text("deliver_to"),
            text("notes"),
            int("confirmed_at"),
            text("confirmed_by"),
            int("closed_at"),
            text("closed_by"),
            text("cancel_reason"),
        ],
    },
    TableDef {
        name: "booking_lines",
        cols: &[
            text("booking_id"),
            int("line_no"),
            text("item_id"),
            int("qty"),
            int("price_c"),
            text("price_rule"),
            text("notes"),
        ],
    },
    TableDef {
        name: "delivery_receipts",
        cols: &[
            text("booking_id"),
            text("client_id"),
            text("date"),
            text("from_location_id"),
            text("status"),
            int("vat"),
            int("vat_bp"),
            text("deliver_to"),
            text("notes"),
            int("approved_at"),
            text("approved_by"),
            text("delivered_date"),
            int("delivered_at"),
            text("delivered_by"),
            text("received_by"),
            int("cancelled_at"),
            text("cancelled_by"),
            text("cancel_reason"),
        ],
    },
    TableDef {
        name: "dr_lines",
        cols: &[
            text("dr_id"),
            int("line_no"),
            text("booking_line_id"),
            text("item_id"),
            text("expiry"),
            int("qty"),
            int("price_c"),
        ],
    },
    TableDef {
        name: "payments",
        cols: &[
            text("client_id"),
            text("date"),
            text("method"),
            int("amount_c"),
            text("reference"),
            text("or_no"),
            text("notes"),
            text("check_bank"),
            text("check_no"),
            text("check_date"),
            text("check_status"),
            text("deposited_date"),
            text("deposit_account"),
            text("cleared_date"),
            text("bounced_date"),
            text("bounce_reason"),
            text("replaces_id"),
            text("status"),
            int("voided_at"),
            text("voided_by"),
            text("void_reason"),
            // Added in 0009: deposited to (checks), received into (transfers).
            text("bank_account_id"),
        ],
    },
    TableDef {
        name: "payment_allocations",
        cols: &[text("payment_id"), text("dr_id"), int("amount_c")],
    },
    TableDef {
        name: "purchase_orders",
        cols: &[
            text("supplier_id"),
            text("date"),
            text("currency"),
            text("fx_rate"),
            text("expected_date"),
            text("status"),
            text("supplier_ref"),
            text("notes"),
            int("ordered_at"),
            text("ordered_by"),
            text("shipped_date"),
            text("arrived_date"),
            int("closed_at"),
            text("closed_by"),
            text("cancel_reason"),
            // Added in 0011.
            text("serial_year"),
        ],
    },
    TableDef {
        name: "po_lines",
        cols: &[
            text("po_id"),
            int("line_no"),
            text("item_id"),
            int("qty"),
            int("price_minor"),
            text("notes"),
            // Added in 0011.
            int("pcs_per_box"),
        ],
    },
    TableDef {
        name: "supplier_payments",
        cols: &[
            text("po_id"),
            text("date"),
            text("currency"),
            int("amount_minor"),
            text("fx_rate"),
            text("reference"),
            text("notes"),
            // Added in 0009: paid from.
            text("bank_account_id"),
        ],
    },
    TableDef {
        name: "po_receipts",
        cols: &[
            text("po_id"),
            text("date"),
            text("location_id"),
            text("notes"),
            text("status"),
            int("voided_at"),
            text("voided_by"),
            text("void_reason"),
        ],
    },
    TableDef {
        name: "po_receipt_lines",
        cols: &[
            text("receipt_id"),
            int("line_no"),
            text("po_line_id"),
            text("item_id"),
            text("expiry"),
            int("qty"),
            // Added in 0011.
            text("box_id"),
        ],
    },
    TableDef {
        name: "boxes",
        cols: &[
            text("item_id"),
            text("po_id"),
            text("po_line_id"),
            text("doc_id"),
            int("pcs"),
        ],
    },
    TableDef {
        name: "dr_line_boxes",
        cols: &[
            text("dr_id"),
            text("dr_line_id"),
            text("box_id"),
            text("expiry"),
            int("qty"),
        ],
    },
    TableDef {
        name: "bank_accounts",
        cols: &[
            text("name"),
            text("bank"),
            text("account_no"),
            text("currency"),
            text("notes"),
            int("active"),
        ],
    },
    TableDef {
        name: "employees",
        cols: &[
            text("name"),
            text("position"),
            int("daily_rate_c"),
            int("sss_c"),
            int("philhealth_c"),
            int("pagibig_c"),
            text("start_date"),
            int("active"),
            text("notes"),
        ],
    },
    TableDef {
        name: "attendance",
        cols: &[
            text("employee_id"),
            text("date"),
            text("status"),
            text("note"),
        ],
    },
    TableDef {
        name: "employee_debts",
        cols: &[
            text("employee_id"),
            text("kind"),
            int("amount_c"),
            text("start_date"),
            int("weeks"),
            int("weekly_c"),
            text("remit_date"),
            text("notes"),
        ],
    },
    TableDef {
        name: "debt_overrides",
        cols: &[text("debt_id"), text("week"), int("amount_c")],
    },
    TableDef {
        name: "payrolls",
        cols: &[
            text("week"),
            text("status"),
            int("posted_at"),
            text("posted_by"),
        ],
    },
    TableDef {
        name: "payroll_lines",
        cols: &[
            text("payroll_id"),
            text("employee_id"),
            text("name"),
            int("daily_rate_c"),
            int("days"),
            int("gross_c"),
            int("sss_c"),
            int("philhealth_c"),
            int("pagibig_c"),
            int("net_c"),
            int("included"),
        ],
    },
    TableDef {
        name: "payroll_debt_lines",
        cols: &[
            text("payroll_id"),
            text("payroll_line_id"),
            text("debt_id"),
            text("kind"),
            int("amount_c"),
        ],
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
