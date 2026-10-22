//! Dashboard (KPIs and data issues, Dashboard → view) and to-dos (anyone
//! signed in; see `sanco_core::domain::todos` for who sees which).

use sanco_core::domain::auth::{Action, Module};
use sanco_core::domain::dashboard::{self, Dashboard};
use sanco_core::domain::todos::{self, TodoInfo, TodoInput};
use sanco_core::time::manila_date_string;
use tauri::State;

use crate::commands::{blocking, require, signed_in};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn dashboard(state: State<'_, AppState>) -> Result<Dashboard, String> {
    let (db, user) = require(&state, Module::Dashboard, Action::View).await?;
    let today = manila_date_string(db.now_ms());
    blocking(move || db.read(|c| dashboard::summary(c, &user.perms, &today))).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_todos(
    state: State<'_, AppState>,
    include_done: bool,
) -> Result<Vec<TodoInfo>, String> {
    let (db, user) = signed_in(&state).await?;
    blocking(move || db.read(|c| todos::list(c, &user.id, include_done))).await
}

/// Creates or edits a to-do. Returns its id.
#[tauri::command]
#[specta::specta]
pub async fn save_todo(state: State<'_, AppState>, todo: TodoInput) -> Result<String, String> {
    let (db, user) = signed_in(&state).await?;
    blocking(move || db.write(user.actor(), move |tx| todos::save(tx, &todo))).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_todo_done(
    state: State<'_, AppState>,
    id: String,
    done: bool,
) -> Result<(), String> {
    let (db, user) = signed_in(&state).await?;
    blocking(move || db.write(user.actor(), move |tx| todos::set_done(tx, &id, done))).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_todo(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, user) = signed_in(&state).await?;
    let (actor, uid, admin) = (user.actor(), user.id.clone(), user.perms.admin);
    blocking(move || db.write(actor, move |tx| todos::delete(tx, &id, &uid, admin))).await
}
