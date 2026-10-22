//! Delivery receipts (DRs). Requesting one is part of Booking or
//! Accounting; approving needs Accounting → approve, delivering and DR
//! numbers Accounting → edit, voiding a delivered DR Accounting → delete.

use std::sync::Arc;

use sanco_core::Db;
use sanco_core::domain::auth::{Action, Module};
use sanco_core::domain::delivery::{self, DeliverInput, DrDetail, DrInfo, DrRequestInput};
use sanco_core::domain::inventory::StockLot;
use sanco_core::sync::numbering::{self, Reservation, series};
use sanco_core::time::manila_date_string;
use tauri::State;

use crate::commands::sync::{reserve_exact_number, reserve_number};
use crate::commands::{blocking, require, require_view_any, signed_in};
use crate::state::AppState;

const VIEWERS: [Module; 2] = [Module::Accounting, Module::Bookings];

#[tauri::command]
#[specta::specta]
pub async fn list_drs(state: State<'_, AppState>) -> Result<Vec<DrInfo>, String> {
    let (db, _) = require_view_any(&state, &VIEWERS).await?;
    blocking(move || db.read(delivery::list)).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_dr(state: State<'_, AppState>, id: String) -> Result<Option<DrDetail>, String> {
    let (db, _) = require_view_any(&state, &VIEWERS).await?;
    blocking(move || db.read(|c| delivery::get(c, &id))).await
}

/// Stock lots of a booking's items, for choosing a DR's lots.
#[tauri::command]
#[specta::specta]
pub async fn dr_lots(
    state: State<'_, AppState>,
    booking_id: String,
) -> Result<Vec<StockLot>, String> {
    let (db, _) = require_view_any(&state, &VIEWERS).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| delivery::lots_for_booking(c, &booking_id, &today))).await
}

/// Requests a DR for a confirmed booking. Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn request_dr(
    state: State<'_, AppState>,
    request: DrRequestInput,
) -> Result<String, String> {
    let (db, user) = signed_in(&state).await?;
    if !user.perms.can(Module::Bookings, Action::Create) {
        user.require(Module::Accounting, Action::Create)
            .map_err(|e| e.to_string())?;
    }
    blocking(move || db.write(user.actor(), move |tx| delivery::request(tx, &request))).await
}

/// Edits a DR that isn't cancelled (header and lines; for a delivered DR
/// also `delivery`). Needs edit in Accounting or Booking.
#[tauri::command]
#[specta::specta]
pub async fn update_dr(
    state: State<'_, AppState>,
    id: String,
    request: DrRequestInput,
    delivery: Option<DeliverInput>,
) -> Result<(), String> {
    let (db, user) = signed_in(&state).await?;
    if !user.perms.can(Module::Bookings, Action::Edit) {
        user.require(Module::Accounting, Action::Edit)
            .map_err(|e| e.to_string())?;
    }
    blocking(move || {
        db.write(user.actor(), move |tx| {
            delivery::update(tx, &id, &request, delivery.as_ref())
        })
    })
    .await
}

/// The number to give a DR now: the typed one when staff type DR numbers,
/// else the next automatic one if it has none yet.
async fn dr_number(
    state: &AppState,
    db: &Arc<Db>,
    id: &str,
    typed: Option<String>,
) -> Result<Option<Reservation>, String> {
    let (read, rid) = (db.clone(), id.to_string());
    let (manual, has) = blocking(move || {
        read.read(|c| {
            Ok((
                numbering::is_manual(c, series::DR)?,
                delivery::number_of(c, &rid)?.is_some(),
            ))
        })
    })
    .await?;
    let typed = typed.filter(|t| !t.trim().is_empty());
    match (manual, typed) {
        (true, Some(no)) => Ok(Some(
            reserve_exact_number(state, series::DR, &no, id).await?,
        )),
        (true, None) => Ok(None),
        (false, _) if has => Ok(None),
        (false, _) => Ok(Some(reserve_number(state, series::DR, id).await?)),
    }
}

/// Approves a requested DR. `no` is the DR (booklet) number, if known yet.
#[tauri::command]
#[specta::specta]
pub async fn approve_dr(
    state: State<'_, AppState>,
    id: String,
    no: Option<String>,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Accounting, Action::Approve).await?;
    let (read, rid) = (db.clone(), id.clone());
    blocking(move || read.read(|c| delivery::check_approve(c, &rid))).await?;
    let res = dr_number(&state, &db, &id, no).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            delivery::approve(tx, &id, res.as_ref())
        })
    })
    .await
}

/// Marks an approved DR delivered (stock goes out). Needs the DR number by
/// now: `no`, or the one it has.
#[tauri::command]
#[specta::specta]
pub async fn deliver_dr(
    state: State<'_, AppState>,
    id: String,
    delivery: DeliverInput,
    no: Option<String>,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Accounting, Action::Edit).await?;
    let (read, rid, input) = (db.clone(), id.clone(), delivery.clone());
    let today = manila_date_string(db.now_ms());
    blocking(move || read.read(|c| delivery::check_deliver(c, &rid, &input, &today))).await?;
    let res = dr_number(&state, &db, &id, no).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            delivery::deliver(tx, &id, &delivery, res.as_ref())
        })
    })
    .await
}

/// Sets or corrects a DR's typed number.
#[tauri::command]
#[specta::specta]
pub async fn set_dr_number(
    state: State<'_, AppState>,
    id: String,
    no: String,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Accounting, Action::Edit).await?;
    let (read, rid) = (db.clone(), id.clone());
    let manual = blocking(move || {
        read.read(|c| {
            delivery::check_set_number(c, &rid)?;
            numbering::is_manual(c, series::DR)
        })
    })
    .await?;
    if !manual {
        return Err("DR numbers are given automatically (Settings → Number formats)".into());
    }
    let res = reserve_exact_number(&state, series::DR, &no, &id).await?;
    blocking(move || db.write(user.actor(), move |tx| delivery::set_number(tx, &id, &res))).await
}

/// Cancels a DR that wasn't delivered.
#[tauri::command]
#[specta::specta]
pub async fn cancel_dr(
    state: State<'_, AppState>,
    id: String,
    reason: String,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Accounting, Action::Edit).await?;
    blocking(move || db.write(user.actor(), move |tx| delivery::cancel(tx, &id, &reason))).await
}

/// Voids a delivered DR: its stock comes back.
#[tauri::command]
#[specta::specta]
pub async fn void_dr(state: State<'_, AppState>, id: String, reason: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Accounting, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| delivery::void(tx, &id, &reason))).await
}
