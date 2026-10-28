//! Reports (Reports → view). Cost reports also need the costs permission,
//! item margin the margins permission too. CSV files go to Downloads.

use std::path::PathBuf;

use sanco_core::domain::auth::{Action, Field, Module};
use sanco_core::domain::inventory::{self, StockLot};
use sanco_core::domain::payments::{self, PaymentInfo};
use sanco_core::domain::receivables::{self, ReceivablesReport};
use sanco_core::domain::reports::{self, DateRange, MarginRow, SalesGroup, SalesRow, SpendRow};
use sanco_core::time::{manila_date_string, manila_stamp};
use tauri::{Manager, State};

use crate::commands::items::require_costs;
use crate::commands::{blocking, require};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn report_sales(
    state: State<'_, AppState>,
    range: DateRange,
    group: SalesGroup,
) -> Result<Vec<SalesRow>, String> {
    let (db, _) = require(&state, Module::Reports, Action::View).await?;
    blocking(move || db.read(|c| reports::sales(c, &range, group))).await
}

#[tauri::command]
#[specta::specta]
pub async fn report_margin(
    state: State<'_, AppState>,
    range: DateRange,
) -> Result<Vec<MarginRow>, String> {
    let (db, user) = require(&state, Module::Reports, Action::View).await?;
    require_costs(&user)?;
    if !user.perms.sees(Field::Margins) {
        return Err(sanco_core::Error::Forbidden("see margins".into()).to_string());
    }
    blocking(move || db.read(|c| reports::item_margin(c, &range))).await
}

#[tauri::command]
#[specta::specta]
pub async fn report_po_spend(
    state: State<'_, AppState>,
    range: DateRange,
) -> Result<Vec<SpendRow>, String> {
    let (db, user) = require(&state, Module::Reports, Action::View).await?;
    require_costs(&user)?;
    blocking(move || db.read(|c| reports::po_spend(c, &range))).await
}

#[tauri::command]
#[specta::specta]
pub async fn report_receivables(state: State<'_, AppState>) -> Result<ReceivablesReport, String> {
    let (db, _) = require(&state, Module::Reports, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| receivables::report(c, &today))).await
}

/// Every payment and check: collections, PDC calendar, bounced checks.
#[tauri::command]
#[specta::specta]
pub async fn report_payments(state: State<'_, AppState>) -> Result<Vec<PaymentInfo>, String> {
    let (db, _) = require(&state, Module::Reports, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| payments::list(c, &today))).await
}

/// Every lot in stock (expiring ones carry `days_left`).
#[tauri::command]
#[specta::specta]
pub async fn report_stock(state: State<'_, AppState>) -> Result<Vec<StockLot>, String> {
    let (db, _) = require(&state, Module::Reports, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| inventory::lots(c, None, &today))).await
}

/// Writes a CSV to the Downloads folder (UTF-8 with BOM, so Excel shows ₱)
/// and returns its path. `name` is like "sales-by-client".
#[tauri::command]
#[specta::specta]
pub async fn save_csv(
    state: State<'_, AppState>,
    name: String,
    content: String,
) -> Result<String, String> {
    let (db, _) = require(&state, Module::Reports, Action::View).await?;
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(60)
        .collect();
    let dir = state
        .app
        .path()
        .download_dir()
        .or_else(|_| state.app.path().document_dir())
        .map_err(|e| e.to_string())?;
    let path: PathBuf = dir.join(format!("sanco-{stem}-{}.csv", manila_stamp(db.now_ms())));
    let mut bytes = "\u{feff}".as_bytes().to_vec();
    bytes.extend_from_slice(content.as_bytes());
    std::fs::write(&path, bytes).map_err(|e| format!("could not save {}: {e}", path.display()))?;
    Ok(path.display().to_string())
}

/// Opens Explorer (Finder on macOS) with the file selected.
#[tauri::command]
#[specta::specta]
pub async fn show_in_folder(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("open");
        cmd.arg("-R").arg(&path);
        cmd
    };
    #[cfg(not(target_os = "macos"))]
    let mut cmd = {
        let mut cmd = std::process::Command::new("explorer.exe");
        cmd.arg(format!("/select,{path}"));
        cmd
    };
    cmd.spawn().map(drop).map_err(|e| e.to_string())
}
