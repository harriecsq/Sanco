//! Settings → Import clients and suppliers: the one-time lists. Admin only.

use sanco_core::backup;
use sanco_core::domain::import_parties::{self, PartiesPlan, PartiesSummary};
use tauri::State;

use crate::commands::sync::reserve_number;
use crate::commands::{blocking, require_admin};
use crate::state::AppState;

fn parse(json: &str) -> Result<PartiesPlan, String> {
    serde_json::from_str(json).map_err(|e| format!("that isn't a valid import file: {e}"))
}

/// Checks an import file without writing anything.
#[tauri::command]
#[specta::specta]
pub async fn check_parties_import(
    state: State<'_, AppState>,
    json: String,
) -> Result<PartiesSummary, String> {
    let (db, _) = require_admin(&state).await?;
    let plan = parse(&json)?;
    blocking(move || db.read(|c| import_parties::validate(c, &plan))).await
}

/// Creates the code groups, clients and suppliers, after a backup. Only runs
/// while there are no clients or suppliers.
#[tauri::command]
#[specta::specta]
pub async fn run_parties_import(
    state: State<'_, AppState>,
    json: String,
) -> Result<PartiesSummary, String> {
    let (db, user) = require_admin(&state).await?;
    let plan = parse(&json)?;
    // Check first, so a refused import doesn't use up numbers.
    let (read, checked) = (db.clone(), plan.clone());
    blocking(move || read.read(|c| import_parties::validate(c, &checked))).await?;
    let dir = state.backup_dir();
    let saved = db.clone();
    blocking(move || backup::manual(&saved, &dir)).await?;
    // Codes are rendered from their group, so the groups come first.
    let (groups_db, groups_plan, actor) = (db.clone(), plan.clone(), user.actor());
    blocking(move || {
        groups_db.write(actor, move |tx| {
            import_parties::apply_groups(tx, &groups_plan)
        })
    })
    .await?;
    let mut reservations = Vec::new();
    for series in import_parties::series_list(&plan) {
        let id = db.new_id().map_err(|e| e.to_string())?;
        reservations.push(reserve_number(&state, &series, &id).await?);
    }
    blocking(move || {
        db.write(user.actor(), move |tx| {
            import_parties::apply(tx, &plan, &reservations)
        })
    })
    .await
}
