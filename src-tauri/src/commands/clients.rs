//! Clients. Margin tiers are stripped for users who can't see margins.

use sanco_core::domain::auth::{Action, Field, Module};
use sanco_core::domain::clients::{self, ClientInfo, ClientInput};
use sanco_core::domain::code_groups::{self, Kind};
use tauri::State;

use crate::commands::sync::reserve_number;
use crate::commands::{Choice, blocking, require, require_save, signed_in};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_clients(state: State<'_, AppState>) -> Result<Vec<ClientInfo>, String> {
    let (db, user) = require(&state, Module::Clients, Action::View).await?;
    let mut list = blocking(move || db.read(clients::list)).await?;
    list.iter_mut().for_each(|c| c.strip(&user.perms));
    Ok(list)
}

#[tauri::command]
#[specta::specta]
pub async fn get_client(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<ClientInfo>, String> {
    let (db, user) = require(&state, Module::Clients, Action::View).await?;
    let mut c = blocking(move || db.read(|c| clients::get(c, &id))).await?;
    if let Some(c) = &mut c {
        c.strip(&user.perms);
    }
    Ok(c)
}

/// Creates (with a new shared code) or updates a client. Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn save_client(
    state: State<'_, AppState>,
    client: ClientInput,
    code_group: String,
) -> Result<String, String> {
    let (db, user) = require_save(&state, Module::Clients, client.id.is_none()).await?;
    let margins = user.perms.sees(Field::Margins);
    if let Some(id) = client.id.clone() {
        // A different code group gives the client a new code.
        let regroup = {
            let (db, id, group) = (db.clone(), id.clone(), code_group.clone());
            blocking(move || {
                db.read(|c| {
                    let current = clients::get(c, &id)?.and_then(|c| c.code_group);
                    let wanted = group.trim().to_ascii_uppercase();
                    if wanted.is_empty() || current.as_deref() == Some(wanted.as_str()) {
                        return Ok(None);
                    }
                    code_groups::series_for(c, Kind::Client, &wanted).map(Some)
                })
            })
            .await?
        };
        let res = match regroup {
            Some(series) => Some(reserve_number(&state, &series, &id).await?),
            None => None,
        };
        blocking(move || {
            db.write(user.actor(), move |tx| {
                clients::update(tx, &client, margins)?;
                match &res {
                    Some(res) => clients::change_group(tx, res),
                    None => Ok(()),
                }
            })
        })
        .await?;
        return Ok(id);
    }
    let series = {
        let db = db.clone();
        blocking(move || db.read(|c| code_groups::series_for(c, Kind::Client, &code_group))).await?
    };
    let id = db.new_id().map_err(|e| e.to_string())?;
    let res = reserve_number(&state, &series, &id).await?;
    blocking(move || {
        db.write(user.actor(), move |tx| {
            clients::create(tx, &client, &res, margins)
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_client(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = require(&state, Module::Clients, Action::Delete).await?;
    blocking(move || db.write(user.actor(), move |tx| clients::delete(tx, &id))).await
}

/// Every client, for pickers in other modules.
#[tauri::command]
#[specta::specta]
pub async fn client_options(state: State<'_, AppState>) -> Result<Vec<Choice>, String> {
    let (db, _) = signed_in(&state).await?;
    let list = blocking(move || db.read(clients::list)).await?;
    Ok(list
        .into_iter()
        .map(|c| Choice {
            label: match c.code {
                Some(code) => format!("{code} · {}", c.name),
                None => c.name,
            },
            id: c.id,
            active: c.active,
        })
        .collect())
}
