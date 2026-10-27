//! Landed cost, margins, VAT and price resolution.
//!
//! Money is integer centavos; percentages are basis points (20% = 2000).
//! Margin is margin on price: `price = cost ÷ (1 − margin)`, so ₱100 at 20%
//! is ₱125. Every division rounds half away from zero.

use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::auth::{Field, Perms};
use crate::domain::{clients, items, settings, text_opt};
use crate::error::{Error, Result};
use crate::ids;
use crate::repo::WriteTx;

/// 100% in basis points.
pub const BP: i64 = 10_000;

pub const CLIENT_ITEM_RATES: &str = "client_item_rates";

/// Currencies a supplier price can be in. All have two minor digits.
pub const CURRENCIES: [&str; 4] = ["PHP", "USD", "CNY", "TWD"];

pub fn check_currency(code: &str) -> Result<String> {
    let c = code.trim().to_ascii_uppercase();
    if CURRENCIES.contains(&c.as_str()) {
        Ok(c)
    } else {
        Err(Error::Invalid(format!(
            "currency must be one of {}",
            CURRENCIES.join(", ")
        )))
    }
}

/// `n ÷ d`, rounded half away from zero. `d` must not be 0.
fn div_round(n: i128, d: i128) -> i128 {
    let q = n / d;
    let r = n % d;
    if 2 * r.abs() >= d.abs() {
        q + if (n < 0) == (d < 0) { 1 } else { -1 }
    } else {
        q
    }
}

fn to_i64(v: i128) -> i64 {
    v.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// `bp` basis points of `amount_c`.
pub fn pct_of(amount_c: i64, bp: i64) -> i64 {
    to_i64(div_round(amount_c as i128 * bp as i128, BP as i128))
}

/// Selling price for a cost at a margin on price. `None` if the margin is
/// 100% or more.
pub fn price_from_cost(cost_c: i64, margin_bp: i64) -> Option<i64> {
    if !(0..BP).contains(&margin_bp) {
        return None;
    }
    Some(to_i64(div_round(
        cost_c as i128 * BP as i128,
        (BP - margin_bp) as i128,
    )))
}

/// Margin on price of selling at `price_c` what costs `cost_c`. `None` for a
/// zero or negative price.
pub fn margin_of(price_c: i64, cost_c: i64) -> Option<i64> {
    (price_c > 0).then(|| {
        to_i64(div_round(
            (price_c as i128 - cost_c as i128) * BP as i128,
            price_c as i128,
        ))
    })
}

pub fn check_margin(bp: Option<i64>) -> Result<Option<i64>> {
    match bp {
        Some(b) if !(0..BP).contains(&b) => Err(Error::Invalid(
            "a margin must be at least 0% and below 100%".into(),
        )),
        other => Ok(other),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct VatAmounts {
    pub net_c: i64,
    pub vat_c: i64,
    pub gross_c: i64,
}

/// `gross = net × (1 + VAT)`.
pub fn with_vat(net_c: i64, vat_bp: i64) -> VatAmounts {
    let vat_c = pct_of(net_c, vat_bp);
    VatAmounts {
        net_c,
        vat_c,
        gross_c: net_c + vat_c,
    }
}

/// Parses an exchange rate (PHP per unit of the foreign currency).
pub fn parse_fx(text: &str) -> Result<Decimal> {
    let bad = || Error::Invalid(format!("“{}” is not a valid exchange rate", text.trim()));
    let fx = Decimal::from_str(text.trim().replace(',', "").as_str()).map_err(|_| bad())?;
    if fx <= Decimal::ZERO || fx.scale() > 6 {
        return Err(bad());
    }
    Ok(fx.normalize())
}

/// Supplier price in PHP centavos: minor units × FX.
pub fn base_cost_c(price_minor: i64, fx: Decimal) -> Option<i64> {
    (Decimal::from(price_minor) * fx)
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ChargeMode {
    /// A PHP amount per unit, in centavos.
    PerUnit,
    /// Basis points of the PHP base cost (supplier price × FX).
    Pct,
    /// A PHP amount for the whole shipment, in centavos, divided by the cost
    /// sheet's shipment quantity.
    Shipment,
}

impl ChargeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ChargeMode::PerUnit => "per_unit",
            ChargeMode::Pct => "pct",
            ChargeMode::Shipment => "shipment",
        }
    }

    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("pct") => ChargeMode::Pct,
            Some("shipment") => ChargeMode::Shipment,
            _ => ChargeMode::PerUnit,
        }
    }

    /// The charge in centavos per unit. `None` for a shipment charge without
    /// a positive shipment quantity.
    pub fn amount_c(self, value: i64, base_c: i64, shipment_qty: Option<i64>) -> Option<i64> {
        match self {
            ChargeMode::PerUnit => Some(value),
            ChargeMode::Pct => Some(pct_of(base_c, value)),
            ChargeMode::Shipment => shipment_qty
                .filter(|&q| q > 0)
                .map(|q| to_i64(div_round(value as i128, q as i128))),
        }
    }
}

/// The numbers a cost sheet produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LandedCost {
    /// Supplier price × FX.
    pub base_c: i64,
    /// Sum of the charge lines.
    pub charges_c: i64,
    pub total_c: i64,
}

/// Landed cost from a supplier price, its currency and FX, and charge lines
/// `(mode, value)`. Shipment charges are divided by `shipment_qty`. `None`
/// while the price or rate is missing, or while a shipment charge has no
/// quantity to divide by. PHP prices ignore the FX rate.
pub fn landed_cost(
    currency: &str,
    price_minor: Option<i64>,
    fx_rate: Option<&str>,
    shipment_qty: Option<i64>,
    charges: impl IntoIterator<Item = (ChargeMode, i64)>,
) -> Option<LandedCost> {
    let price = price_minor?;
    let fx = if currency == "PHP" {
        Decimal::ONE
    } else {
        parse_fx(fx_rate?).ok()?
    };
    let base_c = base_cost_c(price, fx)?;
    let charges_c = charges
        .into_iter()
        .map(|(mode, v)| mode.amount_c(v, base_c, shipment_qty))
        .sum::<Option<i64>>()?;
    Some(LandedCost {
        base_c,
        charges_c,
        total_c: base_c + charges_c,
    })
}

/// Which rule produced a price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum PriceRule {
    /// The manual client-item rate.
    Manual,
    /// Landed cost at the client's margin tier.
    ClientTier,
    /// Landed cost at the item's default margin.
    ItemDefault,
    /// No rate, and no landed cost or margin to work from.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PriceQuote {
    pub rule: PriceRule,
    /// Unit price before VAT.
    pub net_c: Option<i64>,
    /// The manual rate's VAT checkbox. `None` when no rate applied: the
    /// booking or DR decides.
    pub vat: Option<bool>,
    /// VAT and gross at the current rate, if VAT applies.
    pub vat_amounts: Option<VatAmounts>,
    pub vat_bp: i64,
    /// Stripped without the costs permission.
    pub cost_c: Option<i64>,
    /// Margin on price. Stripped without the margins permission.
    pub margin_bp: Option<i64>,
    pub below_cost: bool,
}

impl PriceQuote {
    pub fn strip(&mut self, perms: &Perms) {
        if !perms.sees(Field::Costs) {
            self.cost_c = None;
        }
        if !perms.sees(Field::Margins) {
            self.margin_bp = None;
        }
    }
}

/// Price resolution for a client and item:
/// 1. the manual client-item rate;
/// 2. otherwise landed cost at the client's margin tier;
/// 3. otherwise landed cost at the item's default margin.
pub fn quote(conn: &Connection, client_id: &str, item_id: &str) -> Result<PriceQuote> {
    let vat_bp = settings::vat_rate_bp(conn)?;
    let cost = items::landed_cost_of(conn, item_id)?.map(|l| l.total_c);
    let rate = rate_of(conn, client_id, item_id)?;
    let tier = clients::margin_tier_of(conn, client_id)?;
    let default = items::default_margin_of(conn, item_id)?;

    let (rule, net, vat) = if let Some((price, vat)) = rate {
        (PriceRule::Manual, Some(price), Some(vat))
    } else if let (Some(c), Some(m)) = (cost, tier) {
        (PriceRule::ClientTier, price_from_cost(c, m), None)
    } else if let (Some(c), Some(m)) = (cost, default) {
        (PriceRule::ItemDefault, price_from_cost(c, m), None)
    } else {
        (PriceRule::None, None, None)
    };
    Ok(PriceQuote {
        rule,
        net_c: net,
        vat,
        vat_amounts: net
            .filter(|_| vat != Some(false))
            .map(|n| with_vat(n, vat_bp)),
        vat_bp,
        cost_c: cost,
        margin_bp: net.zip(cost).and_then(|(p, c)| margin_of(p, c)),
        below_cost: net.zip(cost).is_some_and(|(p, c)| p < c),
    })
}

// Client-item rates ---------------------------------------------------------

pub fn rate_id(client_id: &str, item_id: &str) -> String {
    ids::derived_id(client_id, item_id)
}

/// The manual rate `(net price, VAT)` for a client and item, if set.
pub fn rate_of(conn: &Connection, client_id: &str, item_id: &str) -> Result<Option<(i64, bool)>> {
    Ok(conn
        .query_row(
            "SELECT price_c, COALESCE(vat, 0) FROM client_item_rates
             WHERE id = ?1 AND _deleted = 0 AND price_c IS NOT NULL",
            [rate_id(client_id, item_id)],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
        )
        .optional()?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RateInfo {
    pub id: String,
    pub client_id: String,
    pub client_code: Option<String>,
    pub client_name: String,
    pub item_id: String,
    pub item_code: String,
    pub item_name: String,
    pub unit: Option<String>,
    pub price_c: i64,
    pub vat: bool,
    pub vat_amounts: VatAmounts,
    pub notes: Option<String>,
    /// When the rate was last changed (UTC ms).
    pub changed_at: i64,
    /// Stripped without the costs permission.
    pub cost_c: Option<i64>,
    /// Stripped without the margins permission.
    pub margin_bp: Option<i64>,
    /// Shown to everyone, so nobody sells below cost by accident.
    pub below_cost: bool,
}

impl RateInfo {
    pub fn strip(&mut self, perms: &Perms) {
        if !perms.sees(Field::Costs) {
            self.cost_c = None;
        }
        if !perms.sees(Field::Margins) {
            self.margin_bp = None;
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RateFilter {
    pub client_id: Option<String>,
    pub item_id: Option<String>,
}

/// Set rates of live clients and items.
pub fn list_rates(conn: &Connection, filter: &RateFilter) -> Result<Vec<RateInfo>> {
    let vat_bp = settings::vat_rate_bp(conn)?;
    let costs = items::landed_costs(conn)?;
    let mut stmt = conn.prepare(
        "SELECT r.id, r.client_id, n.no, c.name, r.item_id, i.code, i.name, i.unit,
                r.price_c, COALESCE(r.vat, 0), r.notes, r._hlc / 1000
         FROM client_item_rates r
         JOIN clients c ON c.id = r.client_id AND c._deleted = 0
         JOIN items i ON i.id = r.item_id AND i._deleted = 0
         LEFT JOIN doc_numbers n ON n.ref_id = c.id AND n.series = 'CL' AND n._deleted = 0
         WHERE r._deleted = 0 AND r.price_c IS NOT NULL
           AND (?1 IS NULL OR r.client_id = ?1) AND (?2 IS NULL OR r.item_id = ?2)
         ORDER BY c.name COLLATE NOCASE, i.code COLLATE NOCASE, r.id",
    )?;
    let rows = stmt.query_map((&filter.client_id, &filter.item_id), |r| {
        let item_id: String = r.get(4)?;
        let price: i64 = r.get(8)?;
        let vat = r.get::<_, i64>(9)? != 0;
        let cost = costs.get(&item_id).map(|l| l.total_c);
        Ok(RateInfo {
            id: r.get(0)?,
            client_id: r.get(1)?,
            client_code: r.get(2)?,
            client_name: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            item_code: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            item_name: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            unit: r.get(7)?,
            price_c: price,
            vat,
            vat_amounts: with_vat(price, if vat { vat_bp } else { 0 }),
            notes: r.get(10)?,
            changed_at: r.get(11)?,
            cost_c: cost,
            margin_bp: cost.and_then(|c| margin_of(price, c)),
            below_cost: cost.is_some_and(|c| price < c),
            item_id,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RateInput {
    pub client_id: String,
    pub item_id: String,
    /// Net unit price. `None` clears the rate.
    pub price_c: Option<i64>,
    pub vat: bool,
    pub notes: String,
}

/// Sets or clears a client-item rate. A cleared rate keeps its row with a
/// null price, because a deleted derived-id row could never come back.
pub fn save_rate(tx: &mut WriteTx, input: &RateInput) -> Result<()> {
    if tx.get(clients::CLIENTS, &input.client_id)?.is_none() {
        return Err(Error::Invalid("that client no longer exists".into()));
    }
    if tx.get(items::ITEMS, &input.item_id)?.is_none() {
        return Err(Error::Invalid("that item no longer exists".into()));
    }
    if input.price_c.is_some_and(|p| p < 0) {
        return Err(Error::Invalid("a price can't be negative".into()));
    }
    let id = rate_id(&input.client_id, &input.item_id);
    if input.price_c.is_none() && tx.get(CLIENT_ITEM_RATES, &id)?.is_none() {
        return Ok(());
    }
    tx.upsert(
        CLIENT_ITEM_RATES,
        &id,
        json!({
            "client_id": input.client_id,
            "item_id": input.item_id,
            "price_c": input.price_c,
            "vat": input.vat,
            "notes": text_opt(&input.notes),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_is_half_away_from_zero() {
        assert_eq!(div_round(5, 2), 3);
        assert_eq!(div_round(-5, 2), -3);
        assert_eq!(div_round(4, 3), 1);
        assert_eq!(div_round(-4, 3), -1);
        assert_eq!(div_round(7, -2), -4);
    }

    #[test]
    fn margin_on_price() {
        // ₱100 at 20% → ₱125.
        assert_eq!(price_from_cost(10_000, 2_000), Some(12_500));
        assert_eq!(price_from_cost(10_000, 0), Some(10_000));
        assert_eq!(price_from_cost(10_000, BP), None);
        assert_eq!(price_from_cost(10_000, -1), None);
        // ₱100 at 30% → ₱142.857… → ₱142.86.
        assert_eq!(price_from_cost(10_000, 3_000), Some(14_286));
        assert_eq!(margin_of(12_500, 10_000), Some(2_000));
        assert_eq!(margin_of(9_000, 10_000), Some(-1_111));
        assert_eq!(margin_of(0, 10_000), None);
    }

    #[test]
    fn vat_on_net() {
        let v = with_vat(10_000, 1_200);
        assert_eq!((v.net_c, v.vat_c, v.gross_c), (10_000, 1_200, 11_200));
        // 12% of ₱0.05 = 0.6 centavo → 1.
        assert_eq!(with_vat(5, 1_200).vat_c, 1);
        assert_eq!(with_vat(4, 1_200).vat_c, 0);
    }

    #[test]
    fn fx_parsing() {
        assert_eq!(parse_fx("56.25").unwrap(), Decimal::new(5625, 2));
        assert_eq!(parse_fx(" 1,056.5 ").unwrap(), Decimal::new(10565, 1));
        assert!(parse_fx("0").is_err());
        assert!(parse_fx("-1").is_err());
        assert!(parse_fx("abc").is_err());
        assert!(parse_fx("1.1234567").is_err());
    }

    #[test]
    fn landed_cost_adds_charges() {
        // US$10.00 at 56.50 = ₱565.00; freight ₱20/unit; duty 10% = ₱56.50.
        let l = landed_cost(
            "USD",
            Some(1_000),
            Some("56.50"),
            None,
            [(ChargeMode::PerUnit, 2_000), (ChargeMode::Pct, 1_000)],
        )
        .unwrap();
        assert_eq!(l.base_c, 56_500);
        assert_eq!(l.charges_c, 2_000 + 5_650);
        assert_eq!(l.total_c, 64_150);
        // PHP ignores FX; missing FX for a foreign price → no cost.
        assert_eq!(
            landed_cost("PHP", Some(100), None, None, [])
                .unwrap()
                .total_c,
            100
        );
        assert_eq!(landed_cost("USD", Some(100), None, None, []), None);
        assert_eq!(landed_cost("USD", None, Some("56"), None, []), None);
        // ¥1.23 at 7.8125 = ₱9.609375 → 961 centavos.
        assert_eq!(
            landed_cost("CNY", Some(123), Some("7.8125"), None, [])
                .unwrap()
                .base_c,
            961
        );
    }

    #[test]
    fn shipment_charges_divide_by_qty() {
        // ₱100 base; freight ₱10,000 for 300 units = ₱33.333… → ₱33.33.
        let freight = [(ChargeMode::Shipment, 1_000_000)];
        let l = landed_cost("PHP", Some(10_000), None, Some(300), freight).unwrap();
        assert_eq!(l.charges_c, 3_333);
        assert_eq!(l.total_c, 13_333);
        // ₱10,000 for 600 units = ₱16.666… → ₱16.67.
        let l = landed_cost("PHP", Some(10_000), None, Some(600), freight).unwrap();
        assert_eq!(l.charges_c, 1_667);
        // No quantity to divide by → no cost.
        assert_eq!(landed_cost("PHP", Some(10_000), None, None, freight), None);
        assert_eq!(
            landed_cost("PHP", Some(10_000), None, Some(0), freight),
            None
        );
        // Without shipment charges the quantity doesn't matter.
        let per_unit = [(ChargeMode::PerUnit, 5)];
        assert_eq!(
            landed_cost("PHP", Some(10_000), None, None, per_unit)
                .unwrap()
                .total_c,
            10_005
        );
    }
}
