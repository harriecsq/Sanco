//! Reports that need their own aggregation. Receivables aging, collections,
//! checks and stock reuse the module functions (`receivables::report`,
//! `payments::list`, `inventory::lots` / `expiring`); the screen groups
//! those.
//!
//! Date ranges are inclusive `YYYY-MM-DD`; a missing end is open.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::domain::bd::{self, BdStatus};
use crate::domain::purchasing::{self, PoStatus};
use crate::domain::{items, pricing};
use crate::error::Result;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DateRange {
    pub from: Option<String>,
    pub to: Option<String>,
}

impl DateRange {
    pub fn new(from: &str, to: &str) -> Self {
        let opt = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
        Self {
            from: opt(from),
            to: opt(to),
        }
    }

    pub fn contains(&self, date: &str) -> bool {
        self.from.as_deref().is_none_or(|f| date >= f) && self.to.as_deref().is_none_or(|t| date <= t)
    }
}

// Sales ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum SalesGroup {
    Client,
    Item,
    Month,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SalesRow {
    /// Client id, item id or `YYYY-MM`.
    pub key: String,
    pub code: Option<String>,
    pub label: String,
    /// Delivered DRs counted.
    pub drs: i64,
    /// Units (item grouping only).
    pub qty: Option<i64>,
    pub net_c: i64,
    pub vat_c: i64,
    pub gross_c: i64,
}

/// Delivered DRs (voided ones left out) by delivery date. Per item, VAT is
/// worked out per line, so item totals can differ from DR totals by a
/// centavo.
pub fn sales(conn: &Connection, range: &DateRange, group: SalesGroup) -> Result<Vec<SalesRow>> {
    let mut rows: BTreeMap<String, SalesRow> = BTreeMap::new();
    let mut drs_of: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    let mut add = |key: String, code: Option<String>, label: String, dr: &str, qty: Option<i64>, net, vat| {
        let row = rows.entry(key.clone()).or_insert_with(|| SalesRow {
            key: key.clone(),
            code,
            label,
            drs: 0,
            qty: qty.map(|_| 0),
            net_c: 0,
            vat_c: 0,
            gross_c: 0,
        });
        if let (Some(total), Some(q)) = (row.qty.as_mut(), qty) {
            *total += q;
        }
        row.net_c += net;
        row.vat_c += vat;
        row.gross_c += net + vat;
        drs_of.entry(key).or_default().insert(dr.to_string());
    };

    if group == SalesGroup::Item {
        let mut stmt = conn.prepare(
            "SELECT r.id, r.delivered_date, l.item_id, i.code, i.name,
                    COALESCE(l.qty, 0), COALESCE(l.price_c, 0),
                    COALESCE(r.vat, 0), COALESCE(r.vat_bp, 0)
             FROM dr_lines l
             JOIN delivery_receipts r ON r.id = l.dr_id AND r._deleted = 0
             LEFT JOIN items i ON i.id = l.item_id
             WHERE l._deleted = 0 AND r.status = 'delivered'",
        )?;
        let lines = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)? != 0,
                r.get::<_, i64>(8)?,
            ))
        })?;
        for line in lines {
            let (dr, date, item, code, name, qty, price, vat, vat_bp) = line?;
            if !range.contains(&date) {
                continue;
            }
            let net = qty.saturating_mul(price);
            let vat_c = if vat { pricing::pct_of(net, vat_bp) } else { 0 };
            add(item, code, name.unwrap_or_default(), &dr, Some(qty), net, vat_c);
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT t.dr_id, t.delivered_date, t.client_id, n.no, c.name, t.net_c, t.vat_c
             FROM v_dr_totals t
             LEFT JOIN clients c ON c.id = t.client_id
             LEFT JOIN doc_numbers n ON n.ref_id = t.client_id AND n._deleted = 0
             WHERE t.status = 'delivered'",
        )?;
        let drs = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })?;
        for dr in drs {
            let (dr, date, client, code, name, net, vat) = dr?;
            if !range.contains(&date) {
                continue;
            }
            if group == SalesGroup::Client {
                add(client, code, name.unwrap_or_default(), &dr, None, net, vat);
            } else {
                let month = date.get(..7).unwrap_or_default().to_string();
                add(month.clone(), None, month, &dr, None, net, vat);
            }
        }
    }

    let mut out: Vec<SalesRow> = rows
        .into_values()
        .map(|mut r| {
            r.drs = drs_of.get(&r.key).map_or(0, |s| s.len() as i64);
            r
        })
        .collect();
    if group != SalesGroup::Month {
        out.sort_by(|a, b| b.gross_c.cmp(&a.gross_c).then_with(|| a.label.cmp(&b.label)));
    }
    Ok(out)
}

// Item margin -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MarginRow {
    pub item_id: String,
    pub code: Option<String>,
    pub name: String,
    pub unit: Option<String>,
    pub qty: i64,
    /// Sales before VAT.
    pub net_c: i64,
    /// Today's landed cost per unit, if the cost sheet is complete.
    pub unit_cost_c: Option<i64>,
    pub cost_c: Option<i64>,
    pub margin_c: Option<i64>,
    /// Margin on price.
    pub margin_bp: Option<i64>,
}

/// Sales per item against today's landed cost (costs aren't kept per
/// delivery). Needs the costs and margins permissions.
pub fn item_margin(conn: &Connection, range: &DateRange) -> Result<Vec<MarginRow>> {
    let costs = items::landed_costs(conn)?;
    let mut stmt = conn.prepare(
        "SELECT i.id, i.unit FROM items i WHERE i._deleted = 0",
    )?;
    let units: HashMap<String, Option<String>> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut out: Vec<MarginRow> = sales(conn, range, SalesGroup::Item)?
        .into_iter()
        .map(|s| {
            let qty = s.qty.unwrap_or_default();
            let unit_cost_c = costs.get(&s.key).map(|l| l.total_c);
            let cost_c = unit_cost_c.map(|c| c.saturating_mul(qty));
            let margin_c = cost_c.map(|c| s.net_c - c);
            MarginRow {
                unit: units.get(&s.key).cloned().flatten(),
                code: s.code,
                name: s.label,
                qty,
                net_c: s.net_c,
                unit_cost_c,
                cost_c,
                margin_c,
                margin_bp: cost_c.and_then(|c| pricing::margin_of(s.net_c, c)),
                item_id: s.key,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.margin_c
            .unwrap_or(i64::MIN)
            .cmp(&a.margin_c.unwrap_or(i64::MIN))
            .then_with(|| a.code.cmp(&b.code))
    });
    Ok(out)
}

// PO spend --------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SpendRow {
    pub supplier_id: String,
    pub supplier_code: Option<String>,
    pub supplier_name: String,
    /// POs dated in the range, ordered or later (not drafts or cancelled).
    pub po_count: i64,
    /// Their totals in PHP at each PO's FX.
    pub ordered_php_c: i64,
    /// POs whose PHP total is unknown (unpriced lines or no FX).
    pub incomplete_pos: i64,
    /// TT payments dated in the range.
    pub tt_count: i64,
    pub paid_php_c: i64,
    /// TT payments without an FX rate (left out of `paid_php_c`).
    pub tt_without_fx: i64,
}

/// Purchases and TT payments per supplier, in PHP. Cost data.
pub fn po_spend(conn: &Connection, range: &DateRange) -> Result<Vec<SpendRow>> {
    let mut rows: BTreeMap<String, SpendRow> = BTreeMap::new();
    for po in purchasing::list(conn)? {
        if matches!(po.status, PoStatus::Draft | PoStatus::Cancelled) || !range.contains(&po.date) {
            continue;
        }
        let r = rows
            .entry(po.supplier_id.clone().unwrap_or_default())
            .or_insert_with(|| blank_spend(&po.supplier_id, &po.supplier_name));
        r.supplier_code = po.supplier_code.clone().or(r.supplier_code.take());
        r.po_count += 1;
        match po.total_php_c {
            Some(c) if po.unpriced_lines == 0 => r.ordered_php_c += c,
            Some(c) => {
                r.ordered_php_c += c;
                r.incomplete_pos += 1;
            }
            None => r.incomplete_pos += 1,
        }
    }
    for tt in purchasing::list_payments(conn)? {
        if !range.contains(&tt.date) {
            continue;
        }
        let r = rows
            .entry(tt.supplier_id.clone().unwrap_or_default())
            .or_insert_with(|| blank_spend(&tt.supplier_id, &tt.supplier_name));
        r.tt_count += 1;
        match tt.php_c {
            Some(c) => r.paid_php_c += c,
            None => r.tt_without_fx += 1,
        }
    }
    let mut out: Vec<SpendRow> = rows.into_values().collect();
    out.sort_by(|a, b| {
        b.ordered_php_c
            .cmp(&a.ordered_php_c)
            .then_with(|| a.supplier_name.cmp(&b.supplier_name))
    });
    Ok(out)
}

fn blank_spend(id: &Option<String>, name: &Option<String>) -> SpendRow {
    SpendRow {
        supplier_id: id.clone().unwrap_or_default(),
        supplier_code: None,
        supplier_name: name.clone().unwrap_or_default(),
        po_count: 0,
        ordered_php_c: 0,
        incomplete_pos: 0,
        tt_count: 0,
        paid_php_c: 0,
        tt_without_fx: 0,
    }
}

// BD conversion ---------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BdRow {
    /// `YYYY-MM` of the request date.
    pub month: String,
    pub total: i64,
    pub open: i64,
    pub quoted: i64,
    pub won: i64,
    pub lost: i64,
    /// Won ÷ (won + lost), in basis points.
    pub win_rate_bp: Option<i64>,
    /// Quoted prices of won requests, before VAT.
    pub won_net_c: i64,
    /// Quoted prices of lost requests, before VAT.
    pub lost_net_c: i64,
}

/// Requests by month of their date.
pub fn bd_conversion(conn: &Connection, range: &DateRange) -> Result<Vec<BdRow>> {
    let mut rows: BTreeMap<String, BdRow> = BTreeMap::new();
    for r in bd::list(conn)? {
        if !range.contains(&r.date) {
            continue;
        }
        let month = r.date.get(..7).unwrap_or_default().to_string();
        let row = rows.entry(month.clone()).or_insert_with(|| BdRow {
            month,
            ..BdRow::default()
        });
        row.total += 1;
        match r.status {
            BdStatus::Open => row.open += 1,
            BdStatus::Quoted => row.quoted += 1,
            BdStatus::Won => {
                row.won += 1;
                row.won_net_c += r.quoted_net_c;
            }
            BdStatus::Lost => {
                row.lost += 1;
                row.lost_net_c += r.quoted_net_c;
            }
        }
    }
    Ok(rows
        .into_values()
        .map(|mut r| {
            r.win_rate_bp = win_rate(r.won, r.lost);
            r
        })
        .collect())
}

pub fn win_rate(won: i64, lost: i64) -> Option<i64> {
    (won + lost > 0).then(|| (won * 10_000 + (won + lost) / 2) / (won + lost))
}
