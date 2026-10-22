//! Stock locations (Inventory module).

use sanco_core::domain::auth::{Action, Module};
use sanco_core::domain::locations::{self, LocationInfo, LocationInput};
use tauri::State;

use crate::commands::{blocking, require, require_save, signed_in};
use crate::state::AppState;

/// Every location. Readable by anyone signed in: other modules pick from it.
#[tauri::command]
#[specta::specta]
pub async fn list_locations(state: State<'_, AppState>) -> Result<Vec<LocationInfo>, String> {
    let (db, _) = signed_in(&state).await?;
    blocking(move || db.read(locations::list)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_location(
    state: State<'_, AppState>,
    location: LocationInput,
) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Inventory, location.id.is_none()).await?;
    blocking(move || db.write(user.actor(), move |tx| locations::save(tx, &location))).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_location(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Inventory, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| locations::delete(tx, &id))).await
}
