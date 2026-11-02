//! Settings → Import stock: the one-time opening stock (M14). Admin only.

use sanco_core::backup;
use sanco_core::domain::import::{self, ImportPlan, ImportSummary};
use sanco_core::sync::numbering::series;
use tauri::State;

use crate::commands::sync::reserve_number;
use crate::commands::{blocking, require_admin};
use crate::state::AppState;

fn parse(json: &str) -> Result<ImportPlan, String> {
    serde_json::from_str(json).map_err(|e| format!("that isn't a valid import file: {e}"))
}

/// Checks an import file without writing anything.
#[tauri::command]
#[specta::specta]
pub async fn check_stock_import(
    state: State<'_, AppState>,
    json: String,
) -> Result<ImportSummary, String> {
    let (db, _) = require_admin(&state).await?;
    let plan = parse(&json)?;
    blocking(move || db.read(|c| import::validate(c, &plan))).await
}

/// Creates the items and posts the opening receipts, after a backup. Only
/// runs while there are no items.
#[tauri::command]
#[specta::specta]
pub async fn run_stock_import(
    state: State<'_, AppState>,
    json: String,
) -> Result<ImportSummary, String> {
    let (db, user) = require_admin(&state).await?;
    let plan = parse(&json)?;
    // Check first, so a refused import doesn't use up numbers.
    let (read, checked) = (db.clone(), plan.clone());
    blocking(move || read.read(|c| import::validate(c, &checked))).await?;
    let dir = state.backup_dir();
    let saved = db.clone();
    blocking(move || backup::manual(&saved, &dir)).await?;
    let mut reservations = Vec::new();
    for _ in 0..import::document_count(&plan) {
        let id = db.new_id().map_err(|e| e.to_string())?;
        reservations.push(reserve_number(&state, series::STOCK_RECEIPT, &id).await?);
    }
    blocking(move || {
        db.write(user.actor(), move |tx| {
            import::apply(tx, &plan, &reservations)
        })
    })
    .await
}
