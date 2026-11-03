//! DRs and invoices: the client's order and its delivery. Booking is where
//! they are entered and delivered; Accounting users with the same action
//! may do the same (receipts used to live there). Create enters one, edit
//! changes it, delivers it and handles its number, delete cancels, voids and
//! restores it. There is no approval step.

use std::sync::Arc;

use sanco_core::Db;
use sanco_core::domain::auth::{Action, Module, SessionUser};
use sanco_core::domain::delivery::{self, DeliverInput, DrDetail, DrInfo, DrInput, ReceiptKind};
use sanco_core::domain::inventory::{self, StockLot};
use sanco_core::sync::numbering::{self, Reservation};
use sanco_core::time::manila_date_string;
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::commands::sync::{reserve_exact_number, reserve_number};
use crate::commands::{blocking, require_view_any, signed_in};
use crate::state::AppState;

const VIEWERS: [Module; 2] = [Module::Bookings, Module::Accounting];

/// Passes if the user may do `action` in Booking or in Accounting.
fn require_sales(user: &SessionUser, action: Action) -> Result<(), String> {
    if user.perms.can(Module::Bookings, action) {
        return Ok(());
    }
    user.require(Module::Accounting, action)
        .map_err(|e| e.to_string())
}

async fn sales(state: &AppState, action: Action) -> Result<(Arc<Db>, SessionUser), String> {
    let (db, user) = signed_in(state).await?;
    require_sales(&user, action)?;
    Ok((db, user))
}

#[tauri::command]
#[specta::specta]
pub async fn list_drs(state: State<'_, AppState>) -> Result<Vec<DrInfo>, String> {
    let (db, _) = require_view_any(&state, &VIEWERS).await?;
    blocking(move || db.read(delivery::list)).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_dr(state: State<'_, AppState>, id: String) -> Result<Option<DrDetail>, String> {
    let (db, user) = require_view_any(&state, &VIEWERS).await?;
    let mut d = blocking(move || db.read(|c| delivery::get(c, &id))).await?;
    if let Some(d) = &mut d {
        d.strip(&user.perms);
    }
    Ok(d)
}

/// Whether staff type the numbers of DRs and of invoices (else they are
/// given automatically).
#[derive(Debug, Clone, Serialize, Type)]
pub struct NumberModes {
    pub dr_manual: bool,
    pub invoice_manual: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn receipt_number_modes(state: State<'_, AppState>) -> Result<NumberModes, String> {
    let (db, _) = require_view_any(&state, &VIEWERS).await?;
    blocking(move || {
        db.read(|c| {
            Ok(NumberModes {
                dr_manual: numbering::is_manual(c, ReceiptKind::Dr.series())?,
                invoice_manual: numbering::is_manual(c, ReceiptKind::Invoice.series())?,
            })
        })
    })
    .await
}

/// Stock by lot and location, for choosing a receipt's lots (oldest first).
#[tauri::command]
#[specta::specta]
pub async fn receipt_lots(state: State<'_, AppState>) -> Result<Vec<StockLot>, String> {
    let (db, _) = require_view_any(&state, &VIEWERS).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| inventory::lots(c, None, &today))).await
}

/// Enters a new DR or invoice (`id` empty) or saves one in any status.
/// `delivery`: for a delivered one, when and to whom. `no`: the booklet
/// number, typed, if it is to be given or corrected. Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn save_receipt(
    state: State<'_, AppState>,
    id: Option<String>,
    receipt: DrInput,
    delivery: Option<DeliverInput>,
    no: Option<String>,
) -> Result<String, String> {
    let (db, user) = sales(
        &state,
        if id.is_none() {
            Action::Create
        } else {
            Action::Edit
        },
    )
    .await?;
    let typed = no.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    let today = manila_date_string(db.now_ms());

    // Check everything first, so a refused receipt doesn't use up a number.
    let (read, rid, input, done, t) = (
        db.clone(),
        id.clone(),
        receipt.clone(),
        delivery.clone(),
        typed.clone(),
    );
    let plan = blocking(move || {
        read.read(|c| {
            let plan = delivery::number_plan(c, rid.as_deref(), input.kind)?;
            let assign = match (plan.manual, t.as_deref()) {
                (true, Some(t)) => plan.current.as_deref() != Some(t),
                (true, None) => false,
                (false, _) => plan.current.is_none(),
            };
            match &rid {
                Some(rid) => delivery::check_update(c, rid, &input, done.as_ref(), assign, &today)?,
                None => delivery::validate(c, &input)?,
            }
            Ok((plan, assign))
        })
    })
    .await?;
    let (plan, assign) = plan;

    let doc_id = match &id {
        Some(id) => id.clone(),
        None => db.new_id().map_err(|e| e.to_string())?,
    };
    let res: Option<Reservation> = match (assign, plan.manual, typed.as_deref()) {
        (true, true, Some(no)) => {
            Some(reserve_exact_number(&state, plan.series, no, &doc_id).await?)
        }
        (true, false, _) => Some(reserve_number(&state, plan.series, &doc_id).await?),
        _ => None,
    };
    let out = doc_id.clone();
    blocking(move || {
        db.write(user.actor(), move |tx| match id {
            Some(id) => {
                delivery::update(tx, &id, &receipt, delivery.as_ref(), res.as_ref()).map(|()| id)
            }
            None => delivery::create(tx, &doc_id, &receipt, res.as_ref()),
        })
    })
    .await?;
    Ok(out)
}

/// The number to give a receipt now, in its series: the typed one when staff
/// type that series, else the next automatic one if it has none yet.
async fn dr_number(
    state: &AppState,
    db: &Arc<Db>,
    id: &str,
    typed: Option<String>,
) -> Result<Option<Reservation>, String> {
    let (read, rid) = (db.clone(), id.to_string());
    let (series, manual, has) = blocking(move || {
        read.read(|c| {
            let series = delivery::number_series(c, &rid)?;
            Ok((
                series,
                numbering::is_manual(c, series)?,
                delivery::number_of(c, &rid)?.is_some(),
            ))
        })
    })
    .await?;
    let typed = typed.filter(|t| !t.trim().is_empty());
    match (manual, typed) {
        (true, Some(no)) => Ok(Some(reserve_exact_number(state, series, &no, id).await?)),
        (true, None) => Ok(None),
        (false, _) if has => Ok(None),
        (false, _) => Ok(Some(reserve_number(state, series, id).await?)),
    }
}

/// Marks a new DR or invoice delivered (stock goes out). Needs its number by
/// now: `no`, or the one it has.
#[tauri::command]
#[specta::specta]
pub async fn deliver_dr(
    state: State<'_, AppState>,
    id: String,
    delivery: DeliverInput,
    no: Option<String>,
) -> Result<(), String> {
    let (db, user) = sales(&state, Action::Edit).await?;
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

/// Sets or corrects a receipt's typed number.
#[tauri::command]
#[specta::specta]
pub async fn set_dr_number(
    state: State<'_, AppState>,
    id: String,
    no: String,
) -> Result<(), String> {
    let (db, user) = sales(&state, Action::Edit).await?;
    let (read, rid) = (db.clone(), id.clone());
    let (series, manual) = blocking(move || {
        read.read(|c| {
            delivery::check_set_number(c, &rid)?;
            let series = delivery::number_series(c, &rid)?;
            Ok((series, numbering::is_manual(c, series)?))
        })
    })
    .await?;
    if !manual {
        return Err("these numbers are given automatically (Settings → Number formats)".into());
    }
    let res = reserve_exact_number(&state, series, &no, &id).await?;
    blocking(move || db.write(user.actor(), move |tx| delivery::set_number(tx, &id, &res))).await
}

/// Cancels a DR or invoice that wasn't delivered.
#[tauri::command]
#[specta::specta]
pub async fn cancel_dr(
    state: State<'_, AppState>,
    id: String,
    reason: String,
) -> Result<(), String> {
    let (db, user) = sales(&state, Action::Edit).await?;
    blocking(move || db.write(user.actor(), move |tx| delivery::cancel(tx, &id, &reason))).await
}

/// Voids a delivered DR or invoice: its stock comes back.
#[tauri::command]
#[specta::specta]
pub async fn void_dr(state: State<'_, AppState>, id: String, reason: String) -> Result<(), String> {
    let (db, user) = sales(&state, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| delivery::void(tx, &id, &reason))).await
}

/// Restores a cancelled one (back to new) or a void one (delivered again).
#[tauri::command]
#[specta::specta]
pub async fn restore_dr(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = sales(&state, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| delivery::restore(tx, &id))).await
}
