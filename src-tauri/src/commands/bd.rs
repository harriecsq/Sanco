//! Business Development requests. Converting one into a booking also needs
//! permission to create bookings.

use sanco_core::domain::auth::{Action, Module};
use sanco_core::domain::bd::{self, BdDetail, BdInfo, BdInput};
use sanco_core::sync::numbering::series;
use sanco_core::time::manila_date_string;
use tauri::State;

use crate::commands::sync::reserve_number;
use crate::commands::{blocking, require, require_save};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_bd_requests(state: State<'_, AppState>) -> Result<Vec<BdInfo>, String> {
    let (db, _) = require(&state, Module::Bd, Action::View).await?;
    blocking(move || db.read(bd::list)).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_bd_request(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<BdDetail>, String> {
    let (db, _) = require(&state, Module::Bd, Action::View).await?;
    blocking(move || db.read(|c| bd::get(c, &id))).await
}

/// Creates (with a new shared number) or updates a request. Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn save_bd_request(
    state: State<'_, AppState>,
    request: BdInput,
) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Bd, request.id.is_none()).await?;
    if let Some(id) = request.id.clone() {
        blocking(move || db.write(user.actor(), move |tx| bd::update(tx, &request))).await?;
        return Ok(id);
    }
    // Check first, so a refused request doesn't use up a number.
    let (read, input) = (db.clone(), request.clone());
    blocking(move || read.read(|c| bd::validate(c, &input))).await?;
    let id = db.new_id().map_err(|e| e.to_string())?;
    let res = reserve_number(&state, series::BD, &id).await?;
    blocking(move || db.write(user.actor(), move |tx| bd::create(tx, &request, &res))).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_bd_request(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Bd, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| bd::delete(tx, &id))).await
}

/// Turns a request into a draft booking for `client_id` (or the request's
/// own client) and marks it won. Returns the booking's id.
#[tauri::command]
#[specta::specta]
pub async fn convert_bd_request(
    state: State<'_, AppState>,
    id: String,
    client_id: Option<String>,
) -> Result<String, String> {
    let (db, user) = require(&state, Module::Bd, Action::Edit).await?;
    user.require(Module::Bookings, Action::Create)
        .map_err(|e| e.to_string())?;
    let (read, rid, client) = (db.clone(), id.clone(), client_id.clone());
    let today = manila_date_string(db.now_ms());
    blocking(move || read.read(|c| bd::check_convert(c, &rid, client.as_deref(), &today))).await?;
    let booking = db.new_id().map_err(|e| e.to_string())?;
    let res = reserve_number(&state, series::BOOKING, &booking).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            bd::convert(tx, &id, client_id.as_deref(), &res)
        })
    })
    .await
}
