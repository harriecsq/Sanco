//! Order: purchase orders, TT payments and receiving. Marking a PO ordered
//! takes the approve permission. Prices, totals, FX and TT payments are cost
//! data: stripped for users without the costs permission, whose saves keep
//! the stored prices; TT payments need it to open at all.

use sanco_core::domain::auth::{Action, Field, Module};
use sanco_core::domain::purchasing::{
    self, PoAction, PoDetail, PoInfo, PoInput, PoStatusInput, ReceiptDetail, ReceiptInfo,
    ReceiptInput, TtInfo, TtInput,
};
use sanco_core::sync::numbering::series;
use sanco_core::time::manila_date_string;
use tauri::State;

use crate::commands::items::require_costs;
use crate::commands::sync::reserve_number;
use crate::commands::{blocking, require, require_save, require_view_any};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_pos(state: State<'_, AppState>) -> Result<Vec<PoInfo>, String> {
    let (db, user) = require(&state, Module::Orders, Action::View).await?;
    let mut list = blocking(move || db.read(purchasing::list)).await?;
    list.iter_mut().for_each(|p| p.strip(&user.perms));
    Ok(list)
}

/// A PO with its lines, TT payments and receiving reports.
#[tauri::command]
#[specta::specta]
pub async fn get_po(state: State<'_, AppState>, id: String) -> Result<Option<PoDetail>, String> {
    let (db, user) = require(&state, Module::Orders, Action::View).await?;
    let mut po = blocking(move || db.read(|c| purchasing::get(c, &id))).await?;
    if let Some(p) = &mut po {
        p.strip(&user.perms);
    }
    Ok(po)
}

/// Creates a draft (with a new shared number) or saves a PO. Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn save_po(state: State<'_, AppState>, po: PoInput) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Orders, po.id.is_none()).await?;
    let costs = user.perms.sees(Field::Costs);
    if let Some(id) = po.id.clone() {
        blocking(move || db.write(user.actor(), move |tx| purchasing::update(tx, &po, costs)))
            .await?;
        return Ok(id);
    }
    // Check first, so a refused PO doesn't use up a number.
    let (read, input) = (db.clone(), po.clone());
    blocking(move || read.read(|c| purchasing::validate(c, &input, costs))).await?;
    let id = db.new_id().map_err(|e| e.to_string())?;
    let res = reserve_number(&state, series::PO, &id).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            purchasing::create(tx, &po, &res, costs)
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_po_status(
    state: State<'_, AppState>,
    id: String,
    change: PoStatusInput,
) -> Result<(), String> {
    let needed = match change.action {
        PoAction::Order => Action::Approve,
        _ => Action::Edit,
    };
    let (db, user) = require(&state, Module::Orders, needed).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            purchasing::set_status(tx, &id, &change)
        })
    })
    .await
}

/// Deletes a draft PO without payments.
#[tauri::command]
#[specta::specta]
pub async fn delete_po(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Orders, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| purchasing::delete(tx, &id))).await
}

/// Copies the PO's prices, currency and FX into its items' cost sheets.
/// Returns the item codes updated.
#[tauri::command]
#[specta::specta]
pub async fn po_to_cost_sheets(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<String>, String> {
    require(&state, Module::Orders, Action::View).await?;
    let (db, user) = require(&state, Module::Items, Action::Edit).await?;
    require_costs(&user)?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            purchasing::apply_to_cost_sheets(tx, &id)
        })
    })
    .await
}

/// Every TT payment. Needs the costs permission.
#[tauri::command]
#[specta::specta]
pub async fn list_po_payments(state: State<'_, AppState>) -> Result<Vec<TtInfo>, String> {
    let (db, user) = require(&state, Module::Orders, Action::View).await?;
    require_costs(&user)?;
    blocking(move || db.read(purchasing::list_payments)).await
}

/// Adds or saves a TT payment. Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn save_po_payment(
    state: State<'_, AppState>,
    payment: TtInput,
) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Orders, payment.id.is_none()).await?;
    require_costs(&user)?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            purchasing::save_payment(tx, &payment)
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_po_payment(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Orders, Action::Delete).await?;
    require_costs(&user)?;
    blocking(move || db.write(user.actor(), move |tx| purchasing::delete_payment(tx, &id))).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_po_receipts(state: State<'_, AppState>) -> Result<Vec<ReceiptInfo>, String> {
    let (db, _) = require(&state, Module::Orders, Action::View).await?;
    blocking(move || db.read(purchasing::list_receipts)).await
}

/// A receiving report with its lines. Inventory may read it too (an item's
/// stock history links to it).
#[tauri::command]
#[specta::specta]
pub async fn get_po_receipt(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<ReceiptDetail>, String> {
    let (db, _) = require_view_any(&state, &[Module::Orders, Module::Inventory]).await?;
    blocking(move || db.read(|c| purchasing::get_receipt(c, &id))).await
}

/// Posts a receiving report with a new shared number: the goods are in
/// stock. Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn receive_po(
    state: State<'_, AppState>,
    receipt: ReceiptInput,
) -> Result<String, String> {
    let (db, user) = require(&state, Module::Orders, Action::Create).await?;
    // Check first, so a refused report doesn't use up a number.
    let (read, input) = (db.clone(), receipt.clone());
    let today = manila_date_string(db.now_ms());
    blocking(move || read.read(|c| purchasing::validate_receipt(c, &input, &today))).await?;
    let id = db.new_id().map_err(|e| e.to_string())?;
    let res = reserve_number(&state, series::RECEIVING, &id).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            purchasing::receive(tx, &receipt, &res)
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn void_po_receipt(
    state: State<'_, AppState>,
    id: String,
    reason: String,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Orders, Action::Delete).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            purchasing::void_receipt(tx, &id, &reason)
        })
    })
    .await
}
