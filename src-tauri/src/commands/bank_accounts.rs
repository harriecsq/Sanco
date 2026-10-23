//! The company's bank accounts (Accounting module). Payments and TT payments
//! pick from them, so Order users can list them too. TT payments in an
//! account's totals and activity are cost data.

use sanco_core::domain::auth::{Action, Field, Module};
use sanco_core::domain::bank_accounts::{self, BankAccountInfo, BankAccountInput, BankMove};
use tauri::State;

use crate::commands::{blocking, require, require_save, require_view_any};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_bank_accounts(
    state: State<'_, AppState>,
) -> Result<Vec<BankAccountInfo>, String> {
    let (db, user) = require_view_any(&state, &[Module::Accounting, Module::Orders]).await?;
    let mut list = blocking(move || db.read(bank_accounts::list)).await?;
    list.iter_mut().for_each(|a| a.strip(&user.perms));
    Ok(list)
}

/// What went into and out of an account, newest first.
#[tauri::command]
#[specta::specta]
pub async fn bank_account_moves(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<BankMove>, String> {
    let (db, user) = require(&state, Module::Accounting, Action::View).await?;
    let costs = user.perms.sees(Field::Costs);
    blocking(move || db.read(|c| bank_accounts::moves(c, &id, costs))).await
}

/// Adds or saves an account. Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn save_bank_account(
    state: State<'_, AppState>,
    account: BankAccountInput,
) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Accounting, account.id.is_none()).await?;
    blocking(move || db.write(user.actor(), move |tx| bank_accounts::save(tx, &account))).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_bank_account(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Accounting, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| bank_accounts::delete(tx, &id))).await
}
