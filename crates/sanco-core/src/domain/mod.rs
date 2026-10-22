//! Business modules. Each reads with plain SQL and writes through
//! [`crate::repo::WriteTx`].

pub mod audit;
pub mod auth;
pub mod charge_types;
pub mod clients;
pub mod conflicts;
pub mod items;
pub mod locations;
pub mod pricing;
pub mod roles;
pub mod settings;
pub mod suppliers;
pub mod users;

use crate::db::Db;
use crate::error::{Error, Result};
use crate::repo::{Actor, WriteTx};

/// Trimmed text, or `None` when blank.
pub(crate) fn text_opt(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Trimmed text that must not be blank. `what` reads like "a client name".
pub(crate) fn require_text(s: &str, what: &str) -> Result<String> {
    text_opt(s).ok_or_else(|| Error::Invalid(format!("enter {what}")))
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
