//! Bookings. Confirming one takes the approve permission; line costs and
//! margins are stripped for users who can't see them.

use sanco_core::domain::auth::{Action, Module};
use sanco_core::domain::bookings::{self, BookingAction, BookingDetail, BookingInfo, BookingInput};
use sanco_core::sync::numbering::series;
use tauri::State;

use crate::commands::sync::reserve_number;
use crate::commands::{blocking, require, require_save, require_view_any};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_bookings(state: State<'_, AppState>) -> Result<Vec<BookingInfo>, String> {
    let (db, _) = require(&state, Module::Bookings, Action::View).await?;
    blocking(move || db.read(bookings::list)).await
}

/// A booking with its lines and DRs. Accounting may read it too (editing
/// a DR needs its booking).
#[tauri::command]
#[specta::specta]
pub async fn get_booking(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<BookingDetail>, String> {
    let (db, user) = require_view_any(&state, &[Module::Bookings, Module::Accounting]).await?;
    let mut b = blocking(move || db.read(|c| bookings::get(c, &id))).await?;
    if let Some(b) = &mut b {
        b.strip(&user.perms);
    }
    Ok(b)
}

/// Creates a draft (with a new shared number) or saves a draft. Returns its
/// id.
#[tauri::command]
#[specta::specta]
pub async fn save_booking(
    state: State<'_, AppState>,
    booking: BookingInput,
) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Bookings, booking.id.is_none()).await?;
    if let Some(id) = booking.id.clone() {
        blocking(move || db.write(user.actor(), move |tx| bookings::update(tx, &booking))).await?;
        return Ok(id);
    }
    // Check first, so a refused booking doesn't use up a number.
    let (read, input) = (db.clone(), booking.clone());
    blocking(move || read.read(|c| bookings::validate(c, &input))).await?;
    let id = db.new_id().map_err(|e| e.to_string())?;
    let res = reserve_number(&state, series::BOOKING, &id).await?;
    blocking(move || db.write(user.actor(), move |tx| bookings::create(tx, &booking, &res))).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_booking_status(
    state: State<'_, AppState>,
    id: String,
    action: BookingAction,
    reason: String,
) -> Result<(), String> {
    let needed = match action {
        BookingAction::Confirm => Action::Approve,
        _ => Action::Edit,
    };
    let (db, user) = require(&state, Module::Bookings, needed).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            bookings::set_status(tx, &id, action, &reason)
        })
    })
    .await
}

/// Deletes a draft booking.
#[tauri::command]
#[specta::specta]
pub async fn delete_booking(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Bookings, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| bookings::delete(tx, &id))).await
}
