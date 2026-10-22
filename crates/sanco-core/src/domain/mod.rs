//! Business modules. Each reads with plain SQL and writes through
//! [`crate::repo::WriteTx`].

pub mod audit;
pub mod auth;
pub mod bd;
pub mod bookings;
pub mod charge_types;
pub mod clients;
pub mod conflicts;
pub mod delivery;
pub mod inventory;
pub mod items;
pub mod locations;
pub mod payments;
pub mod pricing;
pub mod receivables;
pub mod roles;
pub mod settings;
pub mod suppliers;
pub mod users;

use std::collections::BTreeSet;

use serde_json::Value;

use crate::db::Db;
use crate::error::{Error, Result};
use crate::repo::{Actor, WriteTx};
use crate::time::parse_date;

/// Lines per document.
pub const MAX_LINES: usize = 500;

/// Trimmed text, or `None` when blank.
pub(crate) fn text_opt(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Trimmed text that must not be blank. `what` reads like "a client name".
pub(crate) fn require_text(s: &str, what: &str) -> Result<String> {
    text_opt(s).ok_or_else(|| Error::Invalid(format!("enter {what}")))
}

/// A required `YYYY-MM-DD` date.
pub(crate) fn require_date(s: &str, what: &str) -> Result<String> {
    parse_date(s).ok_or_else(|| Error::Invalid(format!("enter a valid {what}")))
}

/// An optional `YYYY-MM-DD` date; blank is `None`.
pub(crate) fn date_opt(s: Option<&str>, what: &str) -> Result<Option<String>> {
    match s.map(str::trim).filter(|s| !s.is_empty()) {
        Some(d) => require_date(d, what).map(Some),
        None => Ok(None),
    }
}

/// The ids of a document's live lines in `table` (`parent_col` = `parent`).
pub(crate) fn line_ids(
    tx: &WriteTx,
    table: &str,
    parent_col: &str,
    parent: &str,
) -> Result<BTreeSet<String>> {
    let mut stmt = tx.conn().prepare(&format!(
        "SELECT id FROM {table} WHERE {parent_col} = ?1 AND _deleted = 0"
    ))?;
    let rows = stmt.query_map([parent], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Makes a document's lines match `lines` (`(existing id, data)` each, in
/// order): updates lines that are kept, inserts new ones and deletes the
/// rest. `data` must not carry `id`.
pub(crate) fn replace_lines(
    tx: &mut WriteTx,
    table: &str,
    parent_col: &str,
    parent: &str,
    lines: Vec<(Option<String>, Value)>,
) -> Result<()> {
    let mut old = line_ids(tx, table, parent_col, parent)?;
    for (id, mut data) in lines {
        data[parent_col] = Value::from(parent);
        match id.filter(|id| old.contains(id)) {
            Some(id) => {
                old.remove(&id);
                tx.update(table, &id, data)?;
            }
            None => {
                tx.insert(table, data)?;
            }
        }
    }
    for id in old {
        tx.delete(table, &id)?;
    }
    Ok(())
}

/// "GLV-100 ×10, MSK-1 ×2, +3 more" from `(code, qty)` per line.
pub(crate) fn line_summary<'a>(lines: impl IntoIterator<Item = (Option<&'a str>, i64)>) -> String {
    const SHOWN: usize = 3;
    let lines: Vec<_> = lines.into_iter().collect();
    let mut parts: Vec<String> = lines
        .iter()
        .take(SHOWN)
        .map(|(code, qty)| format!("{} ×{qty}", code.unwrap_or("?")))
        .collect();
    if lines.len() > SHOWN {
        parts.push(format!("+{} more", lines.len() - SHOWN));
    }
    parts.join(", ")
}

/// Centavos for messages: 123450 → "₱1,234.50".
pub(crate) fn peso(c: i64) -> String {
    let sign = if c < 0 { "-" } else { "" };
    let c = c.unsigned_abs();
    let digits = (c / 100).to_string();
    let mut whole = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            whole.push(',');
        }
        whole.push(ch);
    }
    format!("{sign}₱{whole}.{:02}", c % 100)
}

/// `qty × price`, saturating.
pub(crate) fn line_amount(qty: i64, price_c: i64) -> i64 {
    qty.saturating_mul(price_c)
}

/// Default rows every workspace starts with (locations, charge types).
/// Seeds use fixed ids and skip rows that exist, so running this on every
/// PC, at every start, is harmless.
pub fn seed_defaults_tx(tx: &mut WriteTx) -> Result<()> {
    locations::seed(tx)?;
    charge_types::seed(tx)
}

pub fn seed_defaults(db: &Db) -> Result<()> {
    db.write(Actor::system(), seed_defaults_tx)
}
