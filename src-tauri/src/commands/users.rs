//! Settings → Users and Roles. Admins only.

use sanco_core::domain::roles::{self, RoleInfo, RoleInput};
use sanco_core::domain::users::{self, NewUser, UserInfo, UserUpdate};
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::commands::{blocking, require_admin, signed_in};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_users(state: State<'_, AppState>) -> Result<Vec<UserInfo>, String> {
    let (db, _) = require_admin(&state).await?;
    blocking(move || db.read(users::list)).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_user(state: State<'_, AppState>, user: NewUser) -> Result<String, String> {
    let (db, admin) = require_admin(&state).await?;
    blocking(move || users::create_user(&db, admin.actor(), user)).await
}

#[tauri::command]
#[specta::specta]
pub async fn update_user(state: State<'_, AppState>, user: UserUpdate) -> Result<(), String> {
    let (db, admin) = require_admin(&state).await?;
    blocking(move || db.write(admin.actor(), move |tx| users::update(tx, &user))).await
}

/// Sets a new password for someone who forgot theirs.
#[tauri::command]
#[specta::specta]
pub async fn reset_password(
    state: State<'_, AppState>,
    user_id: String,
    password: String,
) -> Result<(), String> {
    let (db, admin) = require_admin(&state).await?;
    blocking(move || users::set_password(&db, admin.actor(), &user_id, &password)).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_roles(state: State<'_, AppState>) -> Result<Vec<RoleInfo>, String> {
    let (db, _) = require_admin(&state).await?;
    blocking(move || db.read(roles::list)).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_role(state: State<'_, AppState>, role: RoleInput) -> Result<String, String> {
    let (db, admin) = require_admin(&state).await?;
    blocking(move || db.write(admin.actor(), move |tx| roles::save(tx, role))).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_role(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (db, admin) = require_admin(&state).await?;
    blocking(move || db.write(admin.actor(), move |tx| roles::delete(tx, &id))).await
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct UserOption {
    pub id: String,
    pub name: String,
}

/// Every user's name, active or not, for pickers and filters.
#[tauri::command]
#[specta::specta]
pub async fn user_options(state: State<'_, AppState>) -> Result<Vec<UserOption>, String> {
    let (db, _) = signed_in(&state).await?;
    blocking(move || {
        db.read(|c| {
            Ok(users::list(c)?
                .into_iter()
                .map(|u| UserOption {
                    id: u.id,
                    name: u.display_name,
                })
                .collect())
        })
    })
    .await
}
