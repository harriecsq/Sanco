//! Code groups: a client or supplier is numbered in one of several groups,
//! e.g. clients `NCR-0001` / `REG-0001`, suppliers `SUP-TWN-001`. Each group
//! is its own number series (`CL.<KEY>`, `SUP.<KEY>`) with its own counter.
//! The groups of a kind are one replicated setting (a JSON list); a group is
//! retired, never removed, so its counter and codes stay.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::settings;
use crate::error::{Error, Result};
use crate::repo::WriteTx;
use crate::sync::numbering::{self, NumberFormat, series};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Client,
    Supplier,
}

impl Kind {
    fn setting(self) -> &'static str {
        match self {
            Kind::Client => "code_groups.client",
            Kind::Supplier => "code_groups.supplier",
        }
    }

    pub fn series(self, key: &str) -> String {
        match self {
            Kind::Client => series::client(key),
            Kind::Supplier => series::supplier(key),
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Kind::Client => "client",
            Kind::Supplier => "supplier",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CodeGroup {
    /// Short id of the group (`NCR`); makes the series. Can't change.
    pub key: String,
    /// Printed before the number: `NCR`, `SUP-TWN`.
    pub prefix: String,
    /// Minimum digits of the number.
    pub pad: u32,
    /// Retired groups give out no new codes.
    pub active: bool,
}

/// A group and how many live records hold a code in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CodeGroupInfo {
    pub group: CodeGroup,
    pub used: i64,
    /// What the first code looks like.
    pub example: String,
}

pub fn list(conn: &Connection, kind: Kind) -> Result<Vec<CodeGroup>> {
    match settings::get(conn, kind.setting())? {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(Vec::new()),
    }
}

pub fn infos(conn: &Connection, kind: Kind) -> Result<Vec<CodeGroupInfo>> {
    list(conn, kind)?
        .into_iter()
        .map(|group| {
            let s = kind.series(&group.key);
            let used: i64 = conn.query_row(
                "SELECT COUNT(*) FROM doc_numbers
                 WHERE series = ?1 AND _deleted = 0 AND seq IS NOT NULL",
                [&s],
                |r| r.get(0),
            )?;
            let example = format(&group).render("", 0, 1);
            Ok(CodeGroupInfo {
                group,
                used,
                example,
            })
        })
        .collect()
}

fn format(g: &CodeGroup) -> NumberFormat {
    NumberFormat {
        prefix: g.prefix.clone(),
        year: false,
        reset_yearly: false,
        pad: g.pad,
    }
}

/// The series new codes of group `key` come from. Refuses unknown and
/// retired groups.
pub fn series_for(conn: &Connection, kind: Kind, key: &str) -> Result<String> {
    let key = key.trim().to_ascii_uppercase();
    match list(conn, kind)?.iter().find(|g| g.key == key) {
        None => Err(Error::Invalid(format!(
            "choose a {} code group",
            kind.noun()
        ))),
        Some(g) if !g.active => Err(Error::Invalid(format!(
            "the {} code group {} is retired; choose another",
            kind.noun(),
            g.prefix
        ))),
        Some(_) => Ok(kind.series(&key)),
    }
}

fn check(g: &CodeGroup) -> Result<CodeGroup> {
    let key = g.key.trim().to_ascii_uppercase();
    if key.is_empty() || key.len() > 8 || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Error::Invalid(
            "a group key is 1–8 letters or digits".into(),
        ));
    }
    let prefix = g.prefix.trim().to_ascii_uppercase();
    if prefix.is_empty()
        || prefix.len() > 16
        || prefix.starts_with('-')
        || prefix.ends_with('-')
        || !prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(Error::Invalid(
            "a prefix is up to 16 letters, digits or dashes (e.g. SUP-TWN)".into(),
        ));
    }
    if !(1..=8).contains(&g.pad) {
        return Err(Error::Invalid("digits must be between 1 and 8".into()));
    }
    Ok(CodeGroup {
        key,
        prefix,
        pad: g.pad,
        active: g.active,
    })
}

/// Adds a group, or edits the prefix, digits and active flag of the one with
/// the same key. Codes already given keep their text. Another group of the
/// kind may not print the same prefix.
pub fn save(tx: &mut WriteTx, kind: Kind, group: &CodeGroup) -> Result<()> {
    let g = check(group)?;
    let mut groups = list(tx.conn(), kind)?;
    if groups
        .iter()
        .any(|o| o.key != g.key && o.prefix == g.prefix)
    {
        return Err(Error::Invalid(format!(
            "another group already uses the prefix {}",
            g.prefix
        )));
    }
    match groups.iter_mut().find(|o| o.key == g.key) {
        Some(o) => *o = g.clone(),
        None => groups.push(g.clone()),
    }
    settings::set(tx, kind.setting(), Some(&serde_json::to_string(&groups)?))?;
    // The series' display format, read by `numbering::number_format`.
    settings::set(
        tx,
        &numbering::format_key(&kind.series(&g.key)),
        Some(&json!(format(&g)).to_string()),
    )
}

/// The group a code's series belongs to (`CL.NCR` → `NCR`).
pub fn group_of(series: &str) -> Option<&str> {
    series
        .strip_prefix("CL.")
        .or_else(|| series.strip_prefix("SUP."))
}
