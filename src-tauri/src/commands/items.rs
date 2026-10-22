//! Items: catalog, cost sheets, client rates and price quotes. Costs and
//! margins are stripped here for users without those permissions; cost
//! sheets need the costs permission to open at all.

use sanco_core::domain::auth::{Action, Field, Module, SessionUser};
use sanco_core::domain::items::{self, CostSheetInfo, CostSheetInput, ItemInfo, ItemInput};
use sanco_core::domain::pricing::{self, PriceQuote, RateFilter, RateInfo, RateInput};
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::commands::{Choice, blocking, require, require_save, require_view_any, signed_in};
use crate::state::AppState;

pub(crate) fn require_costs(user: &SessionUser) -> Result<(), String> {
    if user.perms.sees(Field::Costs) {
        Ok(())
    } else {
        Err(sanco_core::Error::Forbidden("see item costs".into()).to_string())
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_items(state: State<'_, AppState>) -> Result<Vec<ItemInfo>, String> {
    let (db, user) = require(&state, Module::Items, Action::View).await?;
    let mut list = blocking(move || db.read(items::list)).await?;
    list.iter_mut().for_each(|i| i.strip(&user.perms));
    Ok(list)
}

#[tauri::command]
#[specta::specta]
pub async fn get_item(state: State<'_, AppState>, id: String) -> Result<Option<ItemInfo>, String> {
    let (db, user) = require(&state, Module::Items, Action::View).await?;
    let mut item = blocking(move || db.read(|c| items::get(c, &id))).await?;
    if let Some(i) = &mut item {
        i.strip(&user.perms);
    }
    Ok(item)
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ItemSuggestions {
    pub categories: Vec<String>,
    pub units: Vec<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn item_suggestions(state: State<'_, AppState>) -> Result<ItemSuggestions, String> {
    let (db, _) = require(&state, Module::Items, Action::View).await?;
    let (categories, units) = blocking(move || db.read(items::suggestions)).await?;
    Ok(ItemSuggestions { categories, units })
}

#[tauri::command]
#[specta::specta]
pub async fn save_item(state: State<'_, AppState>, item: ItemInput) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Items, item.id.is_none()).await?;
    let margins = user.perms.sees(Field::Margins);
    blocking(move || db.write(user.actor(), move |tx| items::save(tx, &item, margins))).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Items, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| items::delete(tx, &id))).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_cost_sheet(
    state: State<'_, AppState>,
    item_id: String,
) -> Result<Option<CostSheetInfo>, String> {
    let (db, user) = require(&state, Module::Items, Action::View).await?;
    require_costs(&user)?;
    blocking(move || db.read(|c| items::cost_sheet(c, &item_id))).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_cost_sheet(state: State<'_, AppState>, sheet: CostSheetInput) -> Result<(), String> {
    let (db, user) = require(&state, Module::Items, Action::Edit).await?;
    require_costs(&user)?;
    blocking(move || db.write(user.actor(), move |tx| items::save_cost_sheet(tx, &sheet))).await
}

/// Client rates, by client and/or item. Clients screens show them too.
#[tauri::command]
#[specta::specta]
pub async fn list_rates(state: State<'_, AppState>, filter: RateFilter) -> Result<Vec<RateInfo>, String> {
    let (db, user) = require_view_any(&state, &[Module::Items, Module::Clients]).await?;
    let mut list = blocking(move || db.read(|c| pricing::list_rates(c, &filter))).await?;
    list.iter_mut().for_each(|r| r.strip(&user.perms));
    Ok(list)
}

/// Sets or clears (price `None`) a client's rate for an item.
#[tauri::command]
#[specta::specta]
pub async fn save_rate(state: State<'_, AppState>, rate: RateInput) -> Result<(), String> {
    let (db, user) = require(&state, Module::Items, Action::Edit).await?;
    blocking(move || db.write(user.actor(), move |tx| pricing::save_rate(tx, &rate))).await
}

/// The price a client pays for an item, and which rule produced it.
#[tauri::command]
#[specta::specta]
pub async fn price_quote(
    state: State<'_, AppState>,
    client_id: String,
    item_id: String,
) -> Result<PriceQuote, String> {
    let (db, user) = require_view_any(
        &state,
        &[Module::Items, Module::Clients, Module::Bookings, Module::Bd],
    )
    .await?;
    let mut q = blocking(move || db.read(|c| pricing::quote(c, &client_id, &item_id))).await?;
    q.strip(&user.perms);
    Ok(q)
}

/// Every item, for pickers in other modules.
#[tauri::command]
#[specta::specta]
pub async fn item_options(state: State<'_, AppState>) -> Result<Vec<Choice>, String> {
    let (db, _) = signed_in(&state).await?;
    let list = blocking(move || db.read(items::list)).await?;
    Ok(list
        .into_iter()
        .map(|i| Choice {
            label: format!("{} · {}", i.code, i.name),
            id: i.id,
            active: i.active,
        })
        .collect())
}
