//! Inventory: stock on hand, stock documents (receipts, adjustments,
//! transfers) and the expiry and low-stock lists. Voiding a document needs
//! the delete permission.

use sanco_core::domain::auth::{Action, Module};
use sanco_core::domain::boxes::{self, BoxStock};
use sanco_core::domain::inventory::{
    self, ItemStock, StockDocDetail, StockDocInfo, StockDocInput, StockLot, StockMove,
};
use sanco_core::time::manila_date_string;
use tauri::State;

use crate::commands::sync::{reserve_number, reserve_serials};
use crate::commands::{blocking, require};
use crate::state::AppState;

/// Every live item with its stock per location.
#[tauri::command]
#[specta::specta]
pub async fn stock_overview(state: State<'_, AppState>) -> Result<Vec<ItemStock>, String> {
    let (db, _) = require(&state, Module::Inventory, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| inventory::item_stock(c, &today))).await
}

/// Non-zero lots (item, location, expiry), optionally of one item.
#[tauri::command]
#[specta::specta]
pub async fn stock_lots(
    state: State<'_, AppState>,
    item_id: Option<String>,
) -> Result<Vec<StockLot>, String> {
    let (db, _) = require(&state, Module::Inventory, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| inventory::lots(c, item_id.as_deref(), &today))).await
}

/// Lots in stock expiring within `within_days` days, or already expired.
#[tauri::command]
#[specta::specta]
pub async fn expiring_stock(
    state: State<'_, AppState>,
    within_days: i64,
) -> Result<Vec<StockLot>, String> {
    let (db, _) = require(&state, Module::Inventory, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    let days = within_days.clamp(0, 3650);
    blocking(move || db.read(|c| inventory::expiring(c, &today, days))).await
}

#[tauri::command]
#[specta::specta]
pub async fn item_stock_moves(
    state: State<'_, AppState>,
    item_id: String,
) -> Result<Vec<StockMove>, String> {
    let (db, _) = require(&state, Module::Inventory, Action::View).await?;
    blocking(move || db.read(|c| inventory::item_moves(c, &item_id))).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_stock_docs(state: State<'_, AppState>) -> Result<Vec<StockDocInfo>, String> {
    let (db, _) = require(&state, Module::Inventory, Action::View).await?;
    blocking(move || db.read(inventory::list_docs)).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_stock_doc(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<StockDocDetail>, String> {
    let (db, _) = require(&state, Module::Inventory, Action::View).await?;
    blocking(move || db.read(|c| inventory::get_doc(c, &id))).await
}

/// Posts a receipt, adjustment or transfer with a new shared number.
/// Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn post_stock_doc(
    state: State<'_, AppState>,
    doc: StockDocInput,
) -> Result<String, String> {
    let (db, user) = require(&state, Module::Inventory, Action::Create).await?;
    // Check first, so a refused document doesn't use up a number.
    let (read, input) = (db.clone(), doc.clone());
    let (now, today) = (db.now_ms(), manila_date_string(db.now_ms()));
    let needs =
        blocking(move || read.read(|c| inventory::serials_for_doc(c, &input, &today, now))).await?;
    let id = db.new_id().map_err(|e| e.to_string())?;
    let mut serials = reserve_serials(&state, needs, &id).await?;
    let res = reserve_number(&state, doc.kind.series(), &id).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            inventory::create(tx, &doc, &res, &mut serials)
        })
    })
    .await
}

/// Boxes in stock with what is left in each, optionally of one item.
#[tauri::command]
#[specta::specta]
pub async fn box_stock(
    state: State<'_, AppState>,
    item_id: Option<String>,
) -> Result<Vec<BoxStock>, String> {
    let (db, _) = require(&state, Module::Inventory, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| boxes::box_stock(c, item_id.as_deref(), None, &today))).await
}

#[tauri::command]
#[specta::specta]
pub async fn void_stock_doc(
    state: State<'_, AppState>,
    id: String,
    reason: String,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Inventory, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| inventory::void(tx, &id, &reason))).await
}
