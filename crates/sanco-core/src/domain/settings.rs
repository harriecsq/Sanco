//! Workspace settings: one replicated `settings` row per key, id
//! `derived_id("setting", key)`. Rows are never deleted, only overwritten.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::text_opt;
use crate::error::{Error, Result};
use crate::ids;
use crate::repo::WriteTx;
use crate::sync::numbering::{self, NumberFormat, series};

pub const SETTINGS: &str = "settings";

pub const VAT_RATE_KEY: &str = "vat_rate_bp";
pub const DEFAULT_VAT_BP: i64 = 1_200;

pub fn setting_id(key: &str) -> String {
    ids::derived_id("setting", key)
}

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached(
            "SELECT value FROM settings WHERE key = ?1 AND _deleted = 0 ORDER BY id LIMIT 1",
        )?
        .query_row([key], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten())
}

pub fn set(tx: &mut WriteTx, key: &str, value: Option<&str>) -> Result<()> {
    tx.upsert(
        SETTINGS,
        &setting_id(key),
        json!({ "key": key, "value": value }),
    )
}

/// VAT rate in basis points (12% = 1200).
pub fn vat_rate_bp(conn: &Connection) -> Result<i64> {
    Ok(get(conn, VAT_RATE_KEY)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_VAT_BP))
}

pub fn set_vat_rate_bp(tx: &mut WriteTx, bp: i64) -> Result<()> {
    if !(0..=5_000).contains(&bp) {
        return Err(Error::Invalid(
            "the VAT rate must be between 0% and 50%".into(),
        ));
    }
    set(tx, VAT_RATE_KEY, Some(&bp.to_string()))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Company {
    pub name: String,
    pub address: String,
    pub tin: String,
    pub phone: String,
    pub email: String,
}

const COMPANY_KEYS: [&str; 5] = ["name", "address", "tin", "phone", "email"];

pub fn company(conn: &Connection) -> Result<Company> {
    let mut v = COMPANY_KEYS
        .iter()
        .map(|k| get(conn, &format!("company.{k}")).map(Option::unwrap_or_default))
        .collect::<Result<Vec<_>>>()?
        .into_iter();
    let mut next = || v.next().unwrap_or_default();
    Ok(Company {
        name: next(),
        address: next(),
        tin: next(),
        phone: next(),
        email: next(),
    })
}

pub fn set_company(tx: &mut WriteTx, c: &Company) -> Result<()> {
    let values = [&c.name, &c.address, &c.tin, &c.phone, &c.email];
    for (k, v) in COMPANY_KEYS.iter().zip(values) {
        set(tx, &format!("company.{k}"), text_opt(v).as_deref())?;
    }
    Ok(())
}

// Number formats ------------------------------------------------------------

/// Series shown in Settings → Number formats, with their labels.
pub const SERIES: [(&str, &str); 11] = [
    (series::BD, "Business Development requests"),
    (series::BOOKING, "Bookings"),
    (series::DR, "Delivery receipts"),
    (series::PAYMENT, "Payments"),
    (series::PO, "Purchase orders"),
    (series::RECEIVING, "Receiving reports"),
    (series::STOCK_RECEIPT, "Stock receipts"),
    (series::ADJUSTMENT, "Stock adjustments"),
    (series::TRANSFER, "Stock transfers"),
    (series::CLIENT, "Client codes"),
    (series::SUPPLIER, "Supplier codes"),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SeriesFormat {
    pub series: String,
    pub label: String,
    pub format: NumberFormat,
    /// What the next number would look like (sequence 1).
    pub example: String,
    /// The series can take numbers typed by staff instead.
    pub manual_allowed: bool,
    /// Staff type the numbers (the format then only applies after
    /// switching to automatic).
    pub manual: bool,
}

pub fn number_formats(conn: &Connection, now_ms: i64) -> Result<Vec<SeriesFormat>> {
    SERIES
        .iter()
        .map(|(s, label)| {
            let format = numbering::number_format(conn, s)?;
            Ok(SeriesFormat {
                series: s.to_string(),
                label: label.to_string(),
                example: format.render(&format.period(now_ms), now_ms, 1),
                format,
                manual_allowed: numbering::MANUAL_SERIES.contains(s),
                manual: numbering::is_manual(conn, s)?,
            })
        })
        .collect()
}

/// Switches a series between typed and automatic numbers. Numbers already
/// given keep their text.
pub fn set_number_mode(tx: &mut WriteTx, series: &str, manual: bool) -> Result<()> {
    if !numbering::MANUAL_SERIES.contains(&series) {
        return Err(Error::Invalid(format!(
            "{series} numbers are always given automatically"
        )));
    }
    set(
        tx,
        &numbering::mode_key(series),
        Some(if manual { "manual" } else { "auto" }),
    )
}

/// Changes how new numbers of a series look. Numbers already given keep
/// their text.
pub fn set_number_format(tx: &mut WriteTx, series: &str, fmt: &NumberFormat) -> Result<()> {
    if !SERIES.iter().any(|(s, _)| *s == series) {
        return Err(Error::Invalid(format!("unknown number series {series}")));
    }
    let prefix = fmt.prefix.trim();
    if prefix.len() > 12 || !prefix.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Error::Invalid(
            "a prefix is up to 12 letters or digits, without spaces or dashes".into(),
        ));
    }
    if !(1..=8).contains(&fmt.pad) {
        return Err(Error::Invalid("digits must be between 1 and 8".into()));
    }
    if fmt.reset_yearly && !fmt.year {
        return Err(Error::Invalid(
            "numbers that restart each year must show the year, or they would repeat".into(),
        ));
    }
    // Switching the yearly reset moves numbering to another period, which
    // would start again at 1 and could repeat numbers already given.
    let current = numbering::number_format(tx.conn(), series)?;
    let used: bool = tx.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM doc_numbers WHERE series = ?1 AND _deleted = 0)",
        [series],
        |r| r.get(0),
    )?;
    if used && current.reset_yearly != fmt.reset_yearly {
        return Err(Error::Invalid(
            "the yearly reset can't change once numbers have been given".into(),
        ));
    }
    let fmt = NumberFormat {
        prefix: prefix.to_ascii_uppercase(),
        ..fmt.clone()
    };
    set(
        tx,
        &numbering::format_key(series),
        Some(&serde_json::to_string(&fmt)?),
    )
}
