//! Suppliers (Order module).

use sanco_core::domain::auth::{Action, Module};
use sanco_core::domain::code_groups::{self, Kind};
use sanco_core::domain::suppliers::{self, SupplierInfo, SupplierInput};
use tauri::State;

use crate::commands::sync::reserve_number;
use crate::commands::{blocking, require, require_save, signed_in};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_suppliers(state: State<'_, AppState>) -> Result<Vec<SupplierInfo>, String> {
    let (db, _) = require(&state, Module::Orders, Action::View).await?;
    blocking(move || db.read(suppliers::list)).await
}

/// Creates (with a new shared code) or updates a supplier. Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn save_supplier(
    state: State<'_, AppState>,
    supplier: SupplierInput,
    code_group: String,
) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Orders, supplier.id.is_none()).await?;
    if let Some(id) = supplier.id.clone() {
        // A different code group gives the supplier a new code.
        let regroup = {
            let (db, id, group) = (db.clone(), id.clone(), code_group.clone());
            blocking(move || {
                db.read(|c| {
                    let current = suppliers::get(c, &id)?.and_then(|s| s.code_group);
                    let wanted = group.trim().to_ascii_uppercase();
                    if wanted.is_empty() || current.as_deref() == Some(wanted.as_str()) {
                        return Ok(None);
                    }
                    code_groups::series_for(c, Kind::Supplier, &wanted).map(Some)
                })
            })
            .await?
        };
        let res = match regroup {
            Some(series) => Some(reserve_number(&state, &series, &id).await?),
            None => None,
        };
        blocking(move || {
            db.write(user.actor(), move |tx| {
                suppliers::update(tx, &supplier)?;
                match &res {
                    Some(res) => suppliers::change_group(tx, res),
                    None => Ok(()),
                }
            })
        })
        .await?;
        return Ok(id);
    }
    let series = {
        let db = db.clone();
        blocking(move || db.read(|c| code_groups::series_for(c, Kind::Supplier, &code_group)))
            .await?
    };
    let id = db.new_id().map_err(|e| e.to_string())?;
    let res = reserve_number(&state, &series, &id).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            suppliers::create(tx, &supplier, &res)
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_supplier(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Orders, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| suppliers::delete(tx, &id))).await
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct SupplierChoice {
    pub id: String,
    pub label: String,
    pub active: bool,
    pub currency: String,
}

/// Every supplier with its default currency, for pickers.
#[tauri::command]
#[specta::specta]
pub async fn supplier_options(state: State<'_, AppState>) -> Result<Vec<SupplierChoice>, String> {
    let (db, _) = signed_in(&state).await?;
    let list = blocking(move || db.read(suppliers::list)).await?;
    Ok(list
        .into_iter()
        .map(|s| SupplierChoice {
            label: match &s.code {
                Some(code) => format!("{code} · {}", s.name),
                None => s.name.clone(),
            },
            id: s.id,
            active: s.active,
            currency: s.currency,
        })
        .collect())
}
