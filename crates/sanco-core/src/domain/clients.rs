//! Clients. A client's code (`CL-0001`) is a shared number in series `CL`,
//! stored in `doc_numbers`, not on the client row.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::auth::{Field, Perms};
use crate::domain::pricing::{self, CLIENT_ITEM_RATES};
use crate::domain::{require_text, text_opt};
use crate::error::{Error, Result};
use crate::repo::WriteTx;
use crate::sync::numbering::{self, Reservation, series};

pub const CLIENTS: &str = "clients";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ClientInfo {
    pub id: String,
    pub code: Option<String>,
    /// False while the code was given offline: it may still change.
    pub code_confirmed: bool,
    pub name: String,
    pub contact_person: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub tin: Option<String>,
    pub address: Option<String>,
    pub terms_days: Option<i64>,
    /// Stripped without the margins permission.
    pub margin_tier_bp: Option<i64>,
    pub credit_limit_c: Option<i64>,
    pub notes: Option<String>,
    pub active: bool,
    /// Another live client has the same name (e.g. both added offline).
    pub duplicate_name: bool,
    pub rate_count: i64,
    pub created_at: i64,
}

impl ClientInfo {
    pub fn strip(&mut self, perms: &Perms) {
        if !perms.sees(Field::Margins) {
            self.margin_tier_bp = None;
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ClientInput {
    /// `None` creates a client.
    pub id: Option<String>,
    pub name: String,
    pub contact_person: String,
    pub phone: String,
    pub email: String,
    pub tin: String,
    pub address: String,
    pub terms_days: Option<i64>,
    /// Ignored for users who can't see margins (the stored tier stays).
    pub margin_tier_bp: Option<i64>,
    pub credit_limit_c: Option<i64>,
    pub notes: String,
    pub active: bool,
}

const SELECT: &str = "
    SELECT c.id, n.no, COALESCE(n.confirmed, 1), c.name, c.contact_person, c.phone, c.email,
           c.tin, c.address, c.terms_days, c.margin_tier_bp, c.credit_limit_c, c.notes,
           COALESCE(c.active, 1),
           EXISTS (SELECT 1 FROM clients d WHERE d._deleted = 0 AND d.id <> c.id
                   AND d.name = c.name COLLATE NOCASE),
           (SELECT COUNT(*) FROM client_item_rates r
            JOIN items i ON i.id = r.item_id AND i._deleted = 0
            WHERE r.client_id = c.id AND r._deleted = 0 AND r.price_c IS NOT NULL),
           c.created_at
    FROM clients c
    LEFT JOIN doc_numbers n ON n.ref_id = c.id AND n.series = 'CL' AND n._deleted = 0
    WHERE c._deleted = 0";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<ClientInfo> {
    Ok(ClientInfo {
        id: r.get(0)?,
        code: r.get(1)?,
        code_confirmed: r.get::<_, i64>(2)? != 0,
        name: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        contact_person: r.get(4)?,
        phone: r.get(5)?,
        email: r.get(6)?,
        tin: r.get(7)?,
        address: r.get(8)?,
        terms_days: r.get(9)?,
        margin_tier_bp: r.get(10)?,
        credit_limit_c: r.get(11)?,
        notes: r.get(12)?,
        active: r.get::<_, i64>(13)? != 0,
        duplicate_name: r.get(14)?,
        rate_count: r.get(15)?,
        created_at: r.get(16)?,
    })
}

pub fn list(conn: &Connection) -> Result<Vec<ClientInfo>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY c.name COLLATE NOCASE, c.id"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<ClientInfo>> {
    Ok(conn
        .query_row(&format!("{SELECT} AND c.id = ?1"), [id], from_row)
        .optional()?)
}

/// The client's margin tier, if it has one.
pub fn margin_tier_of(conn: &Connection, id: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT margin_tier_bp FROM clients WHERE id = ?1 AND _deleted = 0",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .flatten())
}

fn data(input: &ClientInput, margins: bool) -> Result<Value> {
    let name = require_text(&input.name, "a client name")?;
    if input.terms_days.is_some_and(|d| !(0..=365).contains(&d)) {
        return Err(Error::Invalid("payment terms must be 0–365 days".into()));
    }
    if input.credit_limit_c.is_some_and(|c| c < 0) {
        return Err(Error::Invalid("a credit limit can't be negative".into()));
    }
    let mut v = json!({
        "name": name,
        "contact_person": text_opt(&input.contact_person),
        "phone": text_opt(&input.phone),
        "email": text_opt(&input.email),
        "tin": text_opt(&input.tin),
        "address": text_opt(&input.address),
        "terms_days": input.terms_days,
        "credit_limit_c": input.credit_limit_c,
        "notes": text_opt(&input.notes),
        "active": input.active,
    });
    if margins {
        v["margin_tier_bp"] = json!(pricing::check_margin(input.margin_tier_bp)?);
    }
    Ok(v)
}

/// Creates a client with a code reserved for it (`res.ref_id` is the new
/// client's id). `margins`: whether the user may set the margin tier.
pub fn create(
    tx: &mut WriteTx,
    input: &ClientInput,
    res: &Reservation,
    margins: bool,
) -> Result<String> {
    if res.series != series::CLIENT {
        return Err(Error::Invalid("wrong number series for a client".into()));
    }
    let mut v = data(input, margins)?;
    v["id"] = json!(res.ref_id);
    let id = tx.insert(CLIENTS, v)?;
    numbering::assign(tx, res, CLIENTS)?;
    Ok(id)
}

pub fn update(tx: &mut WriteTx, input: &ClientInput, margins: bool) -> Result<()> {
    let id = input.id.as_deref().unwrap_or_default();
    if tx.get(CLIENTS, id)?.is_none() {
        return Err(Error::Invalid("that client no longer exists".into()));
    }
    tx.update(CLIENTS, id, data(input, margins)?).map(drop)
}

/// Deletes a client and its rates. Its code is not given out again.
pub fn delete(tx: &mut WriteTx, id: &str) -> Result<()> {
    if tx.get(CLIENTS, id)?.is_none() {
        return Err(Error::Invalid("that client no longer exists".into()));
    }
    let rates: Vec<String> = {
        let mut stmt = tx
            .conn()
            .prepare("SELECT id FROM client_item_rates WHERE client_id = ?1 AND _deleted = 0")?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for rate in rates {
        tx.delete(CLIENT_ITEM_RATES, &rate)?;
    }
    tx.delete(CLIENTS, id)
}
