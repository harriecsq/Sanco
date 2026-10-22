//! Accounting: payments (cash, checks, bank transfers) with their
//! allocations to DRs, check status, and receivables. Recording a payment
//! takes Accounting → create, editing it or moving a check along
//! Accounting → edit, voiding it Accounting → delete.

use sanco_core::domain::auth::{Action, Module};
use sanco_core::domain::payments::{
    self, CheckStepInput, OpenDr, PaymentDetail, PaymentInfo, PaymentInput,
};
use sanco_core::domain::receivables::{self, ClientBalance, ReceivableDr, ReceivablesReport};
use sanco_core::sync::numbering::series;
use sanco_core::time::manila_date_string;
use tauri::State;

use crate::commands::sync::reserve_number;
use crate::commands::{blocking, require, require_save, require_view_any};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_payments(state: State<'_, AppState>) -> Result<Vec<PaymentInfo>, String> {
    let (db, _) = require(&state, Module::Accounting, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| payments::list(c, &today))).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_payment(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<PaymentDetail>, String> {
    let (db, _) = require(&state, Module::Accounting, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| payments::get(c, &id, &today))).await
}

/// The client's delivered DRs a payment can be allocated to. `payment_id`
/// is the payment being edited, `replaces_id` a check being replaced.
#[tauri::command]
#[specta::specta]
pub async fn payment_open_drs(
    state: State<'_, AppState>,
    client_id: String,
    payment_id: Option<String>,
    replaces_id: Option<String>,
) -> Result<Vec<OpenDr>, String> {
    let (db, _) = require(&state, Module::Accounting, Action::View).await?;
    blocking(move || {
        db.read(|c| {
            payments::open_drs(c, &client_id, payment_id.as_deref(), replaces_id.as_deref())
        })
    })
    .await
}

/// Records a payment (with a new shared number) or edits one. Returns its
/// id.
#[tauri::command]
#[specta::specta]
pub async fn save_payment(
    state: State<'_, AppState>,
    payment: PaymentInput,
) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Accounting, payment.id.is_none()).await?;
    let today = manila_date_string(db.now_ms());
    // Check first, so a refused payment doesn't use up a number.
    let (read, input) = (db.clone(), payment.clone());
    blocking(move || read.read(|c| payments::validate(c, &input, &today))).await?;
    if let Some(id) = payment.id.clone() {
        blocking(move || db.write(user.actor(), move |tx| payments::update(tx, &payment))).await?;
        return Ok(id);
    }
    let id = db.new_id().map_err(|e| e.to_string())?;
    let res = reserve_number(&state, series::PAYMENT, &id).await?;
    blocking(move || db.write(user.actor(), move |tx| payments::create(tx, &payment, &res))).await
}

/// Voids a payment: the DRs it paid owe again.
#[tauri::command]
#[specta::specta]
pub async fn void_payment(
    state: State<'_, AppState>,
    id: String,
    reason: String,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Accounting, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| payments::void(tx, &id, &reason))).await
}

/// Deposits, clears or bounces a check, or takes it back a step.
#[tauri::command]
#[specta::specta]
pub async fn check_step(
    state: State<'_, AppState>,
    id: String,
    step: CheckStepInput,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Accounting, Action::Edit).await?;
    blocking(move || db.write(user.actor(), move |tx| payments::check_step(tx, &id, &step))).await
}

/// Every client's balance and aging today, with the unpaid DRs.
#[tauri::command]
#[specta::specta]
pub async fn receivables_report(state: State<'_, AppState>) -> Result<ReceivablesReport, String> {
    let (db, _) = require(&state, Module::Accounting, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| receivables::report(c, &today))).await
}

/// One client's balance today, for client pages and booking credit checks.
#[tauri::command]
#[specta::specta]
pub async fn client_balance(
    state: State<'_, AppState>,
    client_id: String,
) -> Result<ClientBalance, String> {
    let (db, _) = require_view_any(
        &state,
        &[Module::Accounting, Module::Bookings, Module::Clients],
    )
    .await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| receivables::client_balance(c, &client_id, &today))).await
}

/// A client's unpaid delivered DRs, oldest first.
#[tauri::command]
#[specta::specta]
pub async fn client_receivables(
    state: State<'_, AppState>,
    client_id: String,
) -> Result<Vec<ReceivableDr>, String> {
    let (db, _) = require_view_any(&state, &[Module::Accounting, Module::Clients]).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| receivables::client_drs(c, &client_id, &today))).await
}
