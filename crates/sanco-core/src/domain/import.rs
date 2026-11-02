//! One-time import of the opening stock (M14).
//!
//! A script turns the cleaned inventory spreadsheet into an [`ImportPlan`]:
//! the items, and one line per item, shipment date and location. This module
//! checks the plan and posts it as ordinary items and stock receipts (one
//! document per date and location, at most [`MAX_LINES`] lines each). All of
//! it is written in one transaction, so a refusal leaves nothing behind. It
//! refuses to run once the database has any item, so it can't run twice.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::domain::inventory::{self, MAX_LINES, StockDocInput, StockKind, StockLineInput};
use crate::domain::items::{self, ItemInput};
use crate::error::{Error, Result};
use crate::repo::WriteTx;
use crate::sync::numbering::Reservation;
use crate::time::parse_date;

/// Note on every imported receipt.
pub const NOTE: &str = "Opening balance";

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImportItem {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImportLine {
    pub code: String,
    /// The shipment date, `YYYY-MM-DD`: it picks the lot.
    pub date: String,
    /// Name of an existing location.
    pub location: String,
    pub qty: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImportPlan {
    pub items: Vec<ImportItem>,
    pub lines: Vec<ImportLine>,
}

/// What an import creates, or created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImportSummary {
    pub items: i64,
    pub documents: i64,
    pub lines: i64,
    /// Quantity per location name.
    pub totals: Vec<(String, i64)>,
}

/// One receipt to post: a date and location with its lines.
struct Doc<'a> {
    date: &'a str,
    location: &'a str,
    lines: Vec<&'a ImportLine>,
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// The receipts of a plan: lines grouped by date and location, oldest date
/// first, and split at [`MAX_LINES`].
fn documents(plan: &ImportPlan) -> Vec<Doc<'_>> {
    let mut groups: BTreeMap<(&str, String), Vec<&ImportLine>> = BTreeMap::new();
    for l in &plan.lines {
        groups
            .entry((l.date.trim(), l.location.trim().to_lowercase()))
            .or_default()
            .push(l);
    }
    let mut docs = Vec::new();
    for ((date, _), lines) in groups {
        for chunk in lines.chunks(MAX_LINES) {
            docs.push(Doc {
                date,
                location: chunk[0].location.trim(),
                lines: chunk.to_vec(),
            });
        }
    }
    docs
}

/// How many receipt numbers the plan needs.
pub fn document_count(plan: &ImportPlan) -> usize {
    documents(plan).len()
}

fn live_locations(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut stmt =
        conn.prepare("SELECT id, name FROM locations WHERE _deleted = 0 AND name IS NOT NULL")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn location_id(locations: &[(String, String)], name: &str) -> Result<String> {
    locations
        .iter()
        .find(|(_, n)| same(n, name))
        .map(|(id, _)| id.clone())
        .ok_or_else(|| Error::Invalid(format!("there is no location named {name}")))
}

fn summary(plan: &ImportPlan, docs: &[Doc]) -> ImportSummary {
    let mut totals: BTreeMap<String, i64> = BTreeMap::new();
    for l in &plan.lines {
        *totals.entry(l.location.trim().to_string()).or_default() += l.qty;
    }
    ImportSummary {
        items: plan.items.len() as i64,
        documents: docs.len() as i64,
        lines: plan.lines.len() as i64,
        totals: totals.into_iter().collect(),
    }
}

/// Checks the plan against the database without writing. Fails if the
/// database already has an item.
pub fn validate(conn: &Connection, plan: &ImportPlan) -> Result<ImportSummary> {
    let existing: i64 =
        conn.query_row("SELECT COUNT(*) FROM items WHERE _deleted = 0", [], |r| {
            r.get(0)
        })?;
    if existing > 0 {
        return Err(Error::Invalid(
            "this database already has items; the import only runs on an empty one".into(),
        ));
    }
    if plan.items.is_empty() || plan.lines.is_empty() {
        return Err(Error::Invalid(
            "the import file has no items or no stock".into(),
        ));
    }
    let mut codes = HashSet::new();
    for i in &plan.items {
        let code = i.code.trim();
        if code.is_empty() || i.name.trim().is_empty() {
            return Err(Error::Invalid("every item needs a code and a name".into()));
        }
        if !codes.insert(code.to_lowercase()) {
            return Err(Error::Invalid(format!("the code {code} is listed twice")));
        }
    }
    let locations = live_locations(conn)?;
    let mut seen = HashSet::new();
    for l in &plan.lines {
        if !codes.contains(&l.code.trim().to_lowercase()) {
            return Err(Error::Invalid(format!(
                "{} is not in the item list",
                l.code
            )));
        }
        if l.qty <= 0 {
            return Err(Error::Invalid(format!(
                "the quantity of {} must be positive",
                l.code
            )));
        }
        if parse_date(&l.date).is_none() {
            return Err(Error::Invalid(format!(
                "{} has an invalid date: {}",
                l.code, l.date
            )));
        }
        location_id(&locations, &l.location)?;
        let key = (
            l.code.trim().to_lowercase(),
            l.date.trim(),
            l.location.trim().to_lowercase(),
        );
        if !seen.insert(key) {
            return Err(Error::Invalid(format!(
                "{} appears twice for {} at {}",
                l.code, l.date, l.location
            )));
        }
    }
    Ok(summary(plan, &documents(plan)))
}

/// Creates the items and posts the receipts. `reservations` holds one
/// receipt number per [`document_count`], in order; each one's `ref_id` is
/// the new document's id.
pub fn apply(
    tx: &mut WriteTx,
    plan: &ImportPlan,
    reservations: &[Reservation],
) -> Result<ImportSummary> {
    let sum = validate(tx.conn(), plan)?;
    let docs = documents(plan);
    if docs.len() != reservations.len() {
        return Err(Error::Invalid(
            "the import needs one receipt number per document".into(),
        ));
    }
    let locations = live_locations(tx.conn())?;
    let mut ids: HashMap<String, String> = HashMap::new();
    for i in &plan.items {
        let id = items::save(
            tx,
            &ItemInput {
                id: None,
                code: i.code.clone(),
                name: i.name.clone(),
                unit: String::new(),
                category: String::new(),
                supplier_id: None,
                tracks_expiry: false,
                reorder_level: None,
                default_margin_bp: None,
                notes: String::new(),
                active: true,
            },
            false,
        )?;
        ids.insert(i.code.trim().to_lowercase(), id);
    }
    for (doc, res) in docs.iter().zip(reservations) {
        let input = StockDocInput {
            kind: StockKind::Receipt,
            date: doc.date.to_string(),
            location_id: location_id(&locations, doc.location)?,
            to_location_id: None,
            reason: String::new(),
            notes: NOTE.into(),
            lines: doc
                .lines
                .iter()
                .map(|l| StockLineInput {
                    item_id: ids[&l.code.trim().to_lowercase()].clone(),
                    lot_id: None,
                    qty: l.qty,
                    expiry: None,
                    serial_from: None,
                    serial_to: None,
                })
                .collect(),
        };
        inventory::create(tx, &input, res)?;
    }
    Ok(sum)
}
