//! Workspace-wide settings: company info, VAT rate, cost-sheet charge
//! types and number formats. Anyone signed in may read what forms need;
//! changing anything takes Settings → edit.

use sanco_core::domain::auth::{Action, Module};
use sanco_core::domain::charge_types::{self, ChargeTypeInfo, ChargeTypeInput};
use sanco_core::domain::settings::{self, Company, SeriesFormat};
use sanco_core::sync::numbering::NumberFormat;
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::commands::{blocking, require, signed_in};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn company_info(state: State<'_, AppState>) -> Result<Company, String> {
    let (db, _) = signed_in(&state).await?;
    blocking(move || db.read(settings::company)).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_company_info(state: State<'_, AppState>, company: Company) -> Result<(), String> {
    let (db, user) = require(&state, Module::Settings, Action::Edit).await?;
    blocking(move || db.write(user.actor(), move |tx| settings::set_company(tx, &company))).await
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct PricingSettings {
    /// VAT rate in basis points (12% = 1200).
    pub vat_bp: i64,
}

#[tauri::command]
#[specta::specta]
pub async fn pricing_settings(state: State<'_, AppState>) -> Result<PricingSettings, String> {
    let (db, _) = signed_in(&state).await?;
    let vat_bp = blocking(move || db.read(settings::vat_rate_bp)).await?;
    Ok(PricingSettings { vat_bp })
}

#[tauri::command]
#[specta::specta]
pub async fn set_vat_rate(state: State<'_, AppState>, vat_bp: i64) -> Result<(), String> {
    let (db, user) = require(&state, Module::Settings, Action::Edit).await?;
    blocking(move || db.write(user.actor(), move |tx| settings::set_vat_rate_bp(tx, vat_bp))).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_charge_types(state: State<'_, AppState>) -> Result<Vec<ChargeTypeInfo>, String> {
    let (db, _) = signed_in(&state).await?;
    blocking(move || db.read(charge_types::list)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_charge_type(
    state: State<'_, AppState>,
    charge: ChargeTypeInput,
) -> Result<String, String> {
    let (db, user) = require(&state, Module::Settings, Action::Edit).await?;
    blocking(move || db.write(user.actor(), move |tx| charge_types::save(tx, &charge))).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_charge_type(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Settings, Action::Edit).await?;
    blocking(move || db.write(user.actor(), move |tx| charge_types::delete(tx, &id))).await
}

#[tauri::command]
#[specta::specta]
pub async fn number_formats(state: State<'_, AppState>) -> Result<Vec<SeriesFormat>, String> {
    let (db, _) = require(&state, Module::Settings, Action::View).await?;
    let now = db.now_ms();
    blocking(move || db.read(|c| settings::number_formats(c, now))).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_number_format(
    state: State<'_, AppState>,
    series: String,
    format: NumberFormat,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Settings, Action::Edit).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            settings::set_number_format(tx, &series, &format)
        })
    })
    .await
}

/// Switches a series (DRs) between numbers typed by staff and automatic ones.
#[tauri::command]
#[specta::specta]
pub async fn set_number_mode(
    state: State<'_, AppState>,
    series: String,
    manual: bool,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Settings, Action::Edit).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            settings::set_number_mode(tx, &series, manual)
        })
    })
    .await
}
