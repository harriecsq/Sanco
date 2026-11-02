//! One-time import of the client and supplier lists.
//!
//! A script turns the two spreadsheets into a [`PartiesPlan`]: the code
//! groups, then clients and suppliers in the order they get their codes
//! (each group counts 1, 2, 3 … in that order). Records are created through
//! [`clients::create`] and [`suppliers::create`] in one transaction. It
//! refuses to run once the database has any client or supplier, so it can't
//! run twice.
//!
//! The groups are saved first ([`apply_groups`]), because a code is rendered
//! from its group when it is reserved.

use std::collections::{BTreeMap, HashSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::domain::clients::{self, ClientInput};
use crate::domain::code_groups::{self, CodeGroup, Kind};
use crate::domain::pricing::check_currency;
use crate::domain::suppliers::{self, COUNTRIES, SupplierInput};
use crate::error::{Error, Result};
use crate::repo::WriteTx;
use crate::sync::numbering::Reservation;

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImportClient {
    /// Key of its code group.
    pub group: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImportSupplier {
    pub group: String,
    pub name: String,
    /// TW | CN | OTHER
    pub country: String,
    pub currency: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub phone: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PartiesPlan {
    pub client_groups: Vec<CodeGroup>,
    pub supplier_groups: Vec<CodeGroup>,
    pub clients: Vec<ImportClient>,
    pub suppliers: Vec<ImportSupplier>,
}

/// What an import creates, or created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PartiesSummary {
    pub clients: i64,
    pub suppliers: i64,
    /// Records per code group, e.g. `NCR` → 86.
    pub groups: Vec<(String, i64)>,
}

fn key(g: &str) -> String {
    g.trim().to_ascii_uppercase()
}

/// The series of every code the plan needs, in the order of its records:
/// clients first, then suppliers. [`apply`] takes one reservation for each.
pub fn series_list(plan: &PartiesPlan) -> Vec<String> {
    let clients = plan
        .clients
        .iter()
        .map(|c| Kind::Client.series(&key(&c.group)));
    let suppliers = plan
        .suppliers
        .iter()
        .map(|s| Kind::Supplier.series(&key(&s.group)));
    clients.chain(suppliers).collect()
}

fn summary(plan: &PartiesPlan) -> PartiesSummary {
    let mut groups: BTreeMap<String, i64> = BTreeMap::new();
    for c in &plan.clients {
        *groups
            .entry(Kind::Client.series(&key(&c.group)))
            .or_default() += 1;
    }
    for s in &plan.suppliers {
        *groups
            .entry(Kind::Supplier.series(&key(&s.group)))
            .or_default() += 1;
    }
    PartiesSummary {
        clients: plan.clients.len() as i64,
        suppliers: plan.suppliers.len() as i64,
        groups: groups.into_iter().collect(),
    }
}

fn count(conn: &Connection, table: &str) -> Result<i64> {
    Ok(conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE _deleted = 0"),
        [],
        |r| r.get(0),
    )?)
}

fn check_groups(
    what: &str,
    groups: &[CodeGroup],
    used: impl Iterator<Item = String>,
) -> Result<()> {
    let mut keys = HashSet::new();
    let mut prefixes = HashSet::new();
    for g in groups {
        if !keys.insert(key(&g.key)) || !prefixes.insert(g.prefix.trim().to_ascii_uppercase()) {
            return Err(Error::Invalid(format!(
                "the {what} code groups repeat a key or prefix"
            )));
        }
    }
    for k in used {
        if !keys.contains(&k) {
            return Err(Error::Invalid(format!(
                "a {what} is in the group {k}, which isn't in the group list"
            )));
        }
    }
    Ok(())
}

/// Checks the plan against the database without writing. Fails if the
/// database already has a client or a supplier.
pub fn validate(conn: &Connection, plan: &PartiesPlan) -> Result<PartiesSummary> {
    if count(conn, "clients")? > 0 || count(conn, "suppliers")? > 0 {
        return Err(Error::Invalid(
            "this database already has clients or suppliers; the import only runs on an empty one"
                .into(),
        ));
    }
    if plan.clients.is_empty() && plan.suppliers.is_empty() {
        return Err(Error::Invalid(
            "the import file has no clients or suppliers".into(),
        ));
    }
    check_groups(
        "client",
        &plan.client_groups,
        plan.clients.iter().map(|c| key(&c.group)),
    )?;
    check_groups(
        "supplier",
        &plan.supplier_groups,
        plan.suppliers.iter().map(|s| key(&s.group)),
    )?;
    if plan.clients.iter().any(|c| c.name.trim().is_empty())
        || plan.suppliers.iter().any(|s| s.name.trim().is_empty())
    {
        return Err(Error::Invalid("every record needs a name".into()));
    }
    for s in &plan.suppliers {
        let country = s.country.trim().to_ascii_uppercase();
        if !COUNTRIES.contains(&country.as_str()) {
            return Err(Error::Invalid(format!(
                "{} has an unknown country: {}",
                s.name, s.country
            )));
        }
        check_currency(&s.currency)?;
    }
    Ok(summary(plan))
}

/// Saves the plan's code groups (they must exist before numbers are
/// reserved). Safe to repeat.
pub fn apply_groups(tx: &mut WriteTx, plan: &PartiesPlan) -> Result<()> {
    for g in &plan.client_groups {
        code_groups::save(tx, Kind::Client, g)?;
    }
    for g in &plan.supplier_groups {
        code_groups::save(tx, Kind::Supplier, g)?;
    }
    Ok(())
}

/// Creates the clients and suppliers. `reservations` holds one code per
/// [`series_list`] entry, in order; each one's `ref_id` is the new record's
/// id and its series must be the record's group.
pub fn apply(
    tx: &mut WriteTx,
    plan: &PartiesPlan,
    reservations: &[Reservation],
) -> Result<PartiesSummary> {
    let sum = validate(tx.conn(), plan)?;
    let series = series_list(plan);
    if series.len() != reservations.len()
        || series.iter().zip(reservations).any(|(s, r)| *s != r.series)
    {
        return Err(Error::Invalid(
            "the import needs one code per record, in its group".into(),
        ));
    }
    let (client_res, supplier_res) = reservations.split_at(plan.clients.len());
    for (c, res) in plan.clients.iter().zip(client_res) {
        let input = ClientInput {
            id: None,
            name: c.name.clone(),
            contact_person: String::new(),
            phone: String::new(),
            email: String::new(),
            tin: String::new(),
            address: String::new(),
            terms_days: None,
            margin_tier_bp: None,
            credit_limit_c: None,
            notes: String::new(),
            active: true,
        };
        clients::create(tx, &input, res, false)?;
    }
    for (s, res) in plan.suppliers.iter().zip(supplier_res) {
        let input = SupplierInput {
            id: None,
            name: s.name.clone(),
            country: s.country.clone(),
            currency: s.currency.clone(),
            contact_person: String::new(),
            phone: s.phone.clone(),
            email: String::new(),
            address: s.address.clone(),
            notes: String::new(),
            active: true,
        };
        suppliers::create(tx, &input, res)?;
    }
    Ok(sum)
}
