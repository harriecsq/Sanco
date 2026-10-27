//! The item catalog and each item's cost sheet.
//!
//! Item codes are typed by staff. Two PCs can add the same code while
//! offline; such items are flagged (`duplicate_code`) rather than blocked.
//! Landed cost is computed from the cost sheet when read, never stored.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::auth::{Field, Perms};
use crate::domain::charge_types::CHARGE_TYPES;
use crate::domain::pricing::{
    self, CLIENT_ITEM_RATES, ChargeMode, LandedCost, check_currency, price_from_cost,
};
use crate::domain::purchasing::{self, SupplierPrice};
use crate::domain::suppliers::SUPPLIERS;
use crate::domain::{inventory, require_text, text_opt};
use crate::error::{Error, Result};
use crate::ids;
use crate::repo::WriteTx;

pub const ITEMS: &str = "items";
pub const ITEM_COSTS: &str = "item_costs";
pub const ITEM_COST_CHARGES: &str = "item_cost_charges";

pub fn cost_id(item_id: &str) -> String {
    ids::derived_id(item_id, "cost")
}

pub fn charge_line_id(item_id: &str, charge_type_id: &str) -> String {
    ids::derived_id(item_id, &format!("charge/{charge_type_id}"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ItemInfo {
    pub id: String,
    pub code: String,
    pub name: String,
    pub unit: Option<String>,
    pub category: Option<String>,
    pub supplier_id: Option<String>,
    pub supplier_name: Option<String>,
    pub tracks_expiry: bool,
    pub reorder_level: Option<i64>,
    /// Stripped without the margins permission.
    pub default_margin_bp: Option<i64>,
    pub notes: Option<String>,
    pub active: bool,
    /// Another live item has the same code.
    pub duplicate_code: bool,
    /// Stripped without the costs permission.
    pub landed_cost_c: Option<i64>,
    /// Landed cost at the default margin, before VAT.
    pub base_price_c: Option<i64>,
    pub created_at: i64,
}

impl ItemInfo {
    pub fn strip(&mut self, perms: &Perms) {
        if !perms.sees(Field::Costs) {
            self.landed_cost_c = None;
        }
        if !perms.sees(Field::Margins) {
            self.default_margin_bp = None;
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ItemInput {
    pub id: Option<String>,
    pub code: String,
    pub name: String,
    pub unit: String,
    pub category: String,
    pub supplier_id: Option<String>,
    pub tracks_expiry: bool,
    pub reorder_level: Option<i64>,
    /// Ignored for users who can't see margins.
    pub default_margin_bp: Option<i64>,
    pub notes: String,
    pub active: bool,
}

const SELECT: &str = "
    SELECT i.id, i.code, i.name, i.unit, i.category, i.supplier_id, s.name,
           COALESCE(i.tracks_expiry, 0), i.reorder_level, i.default_margin_bp, i.notes,
           COALESCE(i.active, 1),
           EXISTS (SELECT 1 FROM items d WHERE d._deleted = 0 AND d.id <> i.id
                   AND d.code = i.code COLLATE NOCASE),
           i.created_at
    FROM items i
    LEFT JOIN suppliers s ON s.id = i.supplier_id AND s._deleted = 0
    WHERE i._deleted = 0";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<ItemInfo> {
    Ok(ItemInfo {
        id: r.get(0)?,
        code: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
        name: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
        unit: r.get(3)?,
        category: r.get(4)?,
        supplier_id: r.get(5)?,
        supplier_name: r.get(6)?,
        tracks_expiry: r.get::<_, i64>(7)? != 0,
        reorder_level: r.get(8)?,
        default_margin_bp: r.get(9)?,
        notes: r.get(10)?,
        active: r.get::<_, i64>(11)? != 0,
        duplicate_code: r.get(12)?,
        created_at: r.get(13)?,
        landed_cost_c: None,
        base_price_c: None,
    })
}

fn with_costs(mut item: ItemInfo, costs: &HashMap<String, LandedCost>) -> ItemInfo {
    item.landed_cost_c = costs.get(&item.id).map(|l| l.total_c);
    item.base_price_c = item
        .landed_cost_c
        .zip(item.default_margin_bp)
        .and_then(|(c, m)| price_from_cost(c, m));
    item
}

pub fn list(conn: &Connection) -> Result<Vec<ItemInfo>> {
    let costs = landed_costs(conn)?;
    let mut stmt = conn.prepare(&format!(
        "{SELECT} ORDER BY i.code COLLATE NOCASE, i.name COLLATE NOCASE, i.id"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows
        .map(|r| r.map(|i| with_costs(i, &costs)))
        .collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<ItemInfo>> {
    let costs = costs_for(conn, Some(id))?;
    Ok(conn
        .query_row(&format!("{SELECT} AND i.id = ?1"), [id], from_row)
        .optional()?
        .map(|i| with_costs(i, &costs)))
}

pub fn default_margin_of(conn: &Connection, id: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT default_margin_bp FROM items WHERE id = ?1 AND _deleted = 0",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .flatten())
}

/// Distinct categories and units in use, for suggestions.
pub fn suggestions(conn: &Connection) -> Result<(Vec<String>, Vec<String>)> {
    let distinct = |col: &str| -> Result<Vec<String>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT DISTINCT {col} FROM items WHERE _deleted = 0 AND {col} IS NOT NULL
             ORDER BY {col} COLLATE NOCASE"
        ))?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    };
    Ok((distinct("category")?, distinct("unit")?))
}

fn data(tx: &WriteTx, input: &ItemInput, margins: bool) -> Result<Value> {
    let code = require_text(&input.code, "an item code")?;
    let taken: bool = tx.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM items WHERE _deleted = 0
                        AND code = ?1 COLLATE NOCASE AND id IS NOT ?2)",
        (&code, &input.id),
        |r| r.get(0),
    )?;
    if taken {
        return Err(Error::Invalid(format!(
            "another item already has the code {code}"
        )));
    }
    let supplier = input.supplier_id.as_deref().filter(|s| !s.is_empty());
    if let Some(s) = supplier
        && tx.get(SUPPLIERS, s)?.is_none()
    {
        return Err(Error::Invalid("that supplier no longer exists".into()));
    }
    if input.reorder_level.is_some_and(|q| q < 0) {
        return Err(Error::Invalid("a reorder level can't be negative".into()));
    }
    let mut v = json!({
        "code": code,
        "name": require_text(&input.name, "an item name")?,
        "unit": text_opt(&input.unit),
        "category": text_opt(&input.category),
        "supplier_id": supplier,
        "tracks_expiry": input.tracks_expiry,
        "reorder_level": input.reorder_level,
        "notes": text_opt(&input.notes),
        "active": input.active,
    });
    if margins {
        v["default_margin_bp"] = json!(pricing::check_margin(input.default_margin_bp)?);
    }
    Ok(v)
}

/// Creates or updates an item. `margins`: whether the user may set the
/// default margin.
pub fn save(tx: &mut WriteTx, input: &ItemInput, margins: bool) -> Result<String> {
    let v = data(tx, input, margins)?;
    match &input.id {
        Some(id) => {
            if tx.get(ITEMS, id)?.is_none() {
                return Err(Error::Invalid("that item no longer exists".into()));
            }
            tx.update(ITEMS, id, v)?;
            Ok(id.clone())
        }
        None => tx.insert(ITEMS, v),
    }
}

/// Deletes an item with its cost sheet and client rates. Items on stock
/// movements or sales documents can't be deleted.
pub fn delete(tx: &mut WriteTx, id: &str) -> Result<()> {
    if tx.get(ITEMS, id)?.is_none() {
        return Err(Error::Invalid("that item no longer exists".into()));
    }
    if inventory::item_in_use(tx.conn(), id)? {
        return Err(Error::Invalid(
            "this item is on stock movements or sales documents; mark it inactive instead".into(),
        ));
    }
    for table in [ITEM_COSTS, ITEM_COST_CHARGES, CLIENT_ITEM_RATES] {
        let rows: Vec<String> = {
            let mut stmt = tx.conn().prepare(&format!(
                "SELECT id FROM {table} WHERE item_id = ?1 AND _deleted = 0"
            ))?;
            let rows = stmt.query_map([id], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        for row in rows {
            tx.delete(table, &row)?;
        }
    }
    tx.delete(ITEMS, id)
}

// Cost sheet ----------------------------------------------------------------

/// Landed costs of every live item that has a complete cost sheet.
pub fn landed_costs(conn: &Connection) -> Result<HashMap<String, LandedCost>> {
    costs_for(conn, None)
}

pub fn landed_cost_of(conn: &Connection, item_id: &str) -> Result<Option<LandedCost>> {
    Ok(costs_for(conn, Some(item_id))?.remove(item_id))
}

fn costs_for(conn: &Connection, item: Option<&str>) -> Result<HashMap<String, LandedCost>> {
    let mut lines: HashMap<String, Vec<(ChargeMode, i64)>> = HashMap::new();
    {
        let mut stmt = conn.prepare_cached(
            "SELECT l.item_id, l.mode, l.value FROM item_cost_charges l
             JOIN charge_types t ON t.id = l.charge_type_id AND t._deleted = 0
             WHERE l._deleted = 0 AND l.value IS NOT NULL AND (?1 IS NULL OR l.item_id = ?1)",
        )?;
        let rows = stmt.query_map([item], |r| {
            Ok((
                r.get::<_, String>(0)?,
                ChargeMode::parse(r.get::<_, Option<String>>(1)?.as_deref()),
                r.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (id, mode, value) = row?;
            lines.entry(id).or_default().push((mode, value));
        }
    }
    let mut stmt = conn.prepare_cached(
        "SELECT c.item_id, c.currency, c.supplier_price_minor, c.fx_rate, c.shipment_qty
         FROM item_costs c
         JOIN items i ON i.id = c.item_id AND i._deleted = 0
         WHERE c._deleted = 0 AND (?1 IS NULL OR c.item_id = ?1)",
    )?;
    let rows = stmt.query_map([item], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<i64>>(4)?,
        ))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, currency, price, fx, qty) = row?;
        let charges = lines.remove(&id).unwrap_or_default();
        let currency = currency.unwrap_or_else(|| "PHP".into());
        if let Some(l) = pricing::landed_cost(&currency, price, fx.as_deref(), qty, charges) {
            out.insert(id, l);
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CostLine {
    pub charge_type_id: String,
    pub name: String,
    pub mode: ChargeMode,
    /// Centavos per unit, basis points of the base cost, or centavos for the
    /// whole shipment.
    pub value: Option<i64>,
    /// What the line adds per unit, once the base cost is known.
    pub amount_c: Option<i64>,
    /// Inactive charge types only appear while they have a value.
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CostSheetInfo {
    pub item_id: String,
    pub currency: String,
    pub supplier_price_minor: Option<i64>,
    pub fx_rate: Option<String>,
    /// Units the shipment charges are spread over.
    pub shipment_qty: Option<i64>,
    pub notes: Option<String>,
    pub lines: Vec<CostLine>,
    pub landed: Option<LandedCost>,
    /// When the sheet or one of its charge lines last changed, if ever
    /// (UTC ms), and by whom.
    pub changed_at: Option<i64>,
    pub changed_by: Option<String>,
    /// The price on the item's latest placed PO.
    pub supplier_price: Option<SupplierPrice>,
    /// The latest PO's currency, price or FX differs from the sheet's.
    pub price_pending: bool,
}

/// The cost sheet of a live item: its saved values plus a line for every
/// active charge type. New sheets default to the supplier's currency.
pub fn cost_sheet(conn: &Connection, item_id: &str) -> Result<Option<CostSheetInfo>> {
    let item: Option<Option<String>> = conn
        .query_row(
            "SELECT s.currency FROM items i
             LEFT JOIN suppliers s ON s.id = i.supplier_id AND s._deleted = 0
             WHERE i.id = ?1 AND i._deleted = 0",
            [item_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(supplier_currency) = item else {
        return Ok(None);
    };
    let saved = conn
        .query_row(
            "SELECT currency, supplier_price_minor, fx_rate, notes, shipment_qty
             FROM item_costs WHERE id = ?1 AND _deleted = 0",
            [cost_id(item_id)],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                ))
            },
        )
        .optional()?;
    let (currency, price, fx, notes, shipment_qty) = saved.unwrap_or_default();
    // Saves skip unchanged rows, so a charge-only edit doesn't touch
    // item_costs: take the newest of the sheet and its lines.
    let (changed_at, changed_by) = conn
        .query_row(
            "SELECT c._hlc / 1000, u.display_name FROM (
                 SELECT _hlc, updated_by FROM item_costs WHERE id = ?1 AND _deleted = 0
                 UNION ALL
                 SELECT _hlc, updated_by FROM item_cost_charges
                 WHERE item_id = ?2 AND _deleted = 0
             ) c LEFT JOIN users u ON u.id = c.updated_by
             ORDER BY c._hlc DESC LIMIT 1",
            [cost_id(item_id).as_str(), item_id],
            |r| Ok((Some(r.get::<_, i64>(0)?), r.get::<_, Option<String>>(1)?)),
        )
        .optional()?
        .unwrap_or_default();
    let currency = currency
        .or(supplier_currency)
        .unwrap_or_else(|| "PHP".into());

    let types: Vec<(String, String, ChargeMode, bool)> = {
        let mut stmt = conn.prepare(
            "SELECT id, name, mode, COALESCE(active, 1) FROM charge_types WHERE _deleted = 0
             ORDER BY sort, name COLLATE NOCASE, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ChargeMode::parse(r.get::<_, Option<String>>(2)?.as_deref()),
                r.get::<_, i64>(3)? != 0,
            ))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let mut lines = Vec::new();
    for (type_id, name, default_mode, active) in types {
        let line: Option<(Option<String>, Option<i64>)> = conn
            .query_row(
                "SELECT mode, value FROM item_cost_charges WHERE id = ?1 AND _deleted = 0",
                [charge_line_id(item_id, &type_id)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (mode, value) = match line {
            Some((m, v)) => (m.map_or(default_mode, |m| ChargeMode::parse(Some(&m))), v),
            None => (default_mode, None),
        };
        if active || value.is_some() {
            lines.push(CostLine {
                charge_type_id: type_id,
                name,
                mode,
                value,
                amount_c: None,
                active,
            });
        }
    }
    let landed = pricing::landed_cost(
        &currency,
        price,
        fx.as_deref(),
        shipment_qty,
        lines.iter().filter_map(|l| l.value.map(|v| (l.mode, v))),
    );
    if let Some(l) = landed {
        for line in &mut lines {
            line.amount_c = line
                .value
                .and_then(|v| line.mode.amount_c(v, l.base_c, shipment_qty));
        }
    }
    let supplier_price = purchasing::latest_price(conn, item_id)?;
    let price_pending = supplier_price.as_ref().is_some_and(|sp| {
        let same_fx = sp.currency == "PHP"
            || match (&sp.fx_rate, &fx) {
                (Some(a), Some(b)) => pricing::parse_fx(a).ok() == pricing::parse_fx(b).ok(),
                (a, b) => a == b,
            };
        sp.currency != currency || Some(sp.price_minor) != price || !same_fx
    });
    Ok(Some(CostSheetInfo {
        item_id: item_id.to_string(),
        currency,
        supplier_price_minor: price,
        fx_rate: fx,
        shipment_qty,
        notes,
        lines,
        landed,
        changed_at,
        changed_by,
        supplier_price,
        price_pending,
    }))
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CostLineInput {
    pub charge_type_id: String,
    pub mode: ChargeMode,
    /// `None` clears the line.
    pub value: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CostSheetInput {
    pub item_id: String,
    pub currency: String,
    pub supplier_price_minor: Option<i64>,
    /// PHP per unit of `currency`. Ignored for PHP.
    pub fx_rate: String,
    /// Units the shipment charges are spread over. Required while a shipment
    /// charge has a value.
    pub shipment_qty: Option<i64>,
    pub notes: String,
    pub lines: Vec<CostLineInput>,
}

pub fn save_cost_sheet(tx: &mut WriteTx, input: &CostSheetInput) -> Result<()> {
    let item = &input.item_id;
    if tx.get(ITEMS, item)?.is_none() {
        return Err(Error::Invalid("that item no longer exists".into()));
    }
    let currency = check_currency(&input.currency)?;
    if input.supplier_price_minor.is_some_and(|p| p < 0) {
        return Err(Error::Invalid("a supplier price can't be negative".into()));
    }
    let fx = match (currency.as_str(), input.fx_rate.trim()) {
        ("PHP", _) | (_, "") => None,
        (_, text) => Some(pricing::parse_fx(text)?.to_string()),
    };
    if input.shipment_qty.is_some_and(|q| q <= 0) {
        return Err(Error::Invalid(
            "the shipment quantity must be more than zero".into(),
        ));
    }
    let per_shipment = input
        .lines
        .iter()
        .any(|l| l.mode == ChargeMode::Shipment && l.value.is_some());
    if per_shipment && input.shipment_qty.is_none() {
        return Err(Error::Invalid(
            "enter the shipment quantity to divide the shipment charges by".into(),
        ));
    }
    tx.upsert(
        ITEM_COSTS,
        &cost_id(item),
        json!({
            "item_id": item,
            "currency": currency,
            "supplier_price_minor": input.supplier_price_minor,
            "fx_rate": fx,
            "shipment_qty": input.shipment_qty,
            "notes": text_opt(&input.notes),
        }),
    )?;
    for line in &input.lines {
        if tx.get(CHARGE_TYPES, &line.charge_type_id)?.is_none() {
            return Err(Error::Invalid("one of the charges no longer exists".into()));
        }
        if line.value.is_some_and(|v| v < 0) {
            return Err(Error::Invalid("a charge can't be negative".into()));
        }
        let id = charge_line_id(item, &line.charge_type_id);
        if line.value.is_none() && tx.get(ITEM_COST_CHARGES, &id)?.is_none() {
            continue;
        }
        tx.upsert(
            ITEM_COST_CHARGES,
            &id,
            json!({
                "item_id": item,
                "charge_type_id": line.charge_type_id,
                "mode": line.mode.as_str(),
                "value": line.value,
            }),
        )?;
    }
    Ok(())
}
