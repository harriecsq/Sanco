//! Signing in and out.

use std::time::Duration;

use sanco_core::domain::auth::{self, Action, Field, Module, SessionUser};
use sanco_core::domain::users::{self, NewUser};
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::commands::{blocking, signed_in, sync::db};
use crate::state::AppState;

/// The signed-in user, or `None` (show the sign-in screen).
#[tauri::command]
#[specta::specta]
pub async fn session_info(state: State<'_, AppState>) -> Result<Option<SessionUser>, String> {
    if state.session().is_none() || state.node.lock().await.is_none() {
        return Ok(None);
    }
    Ok(signed_in(&state).await.ok().map(|(_, u)| u))
}

#[tauri::command]
#[specta::specta]
pub async fn login(
    state: State<'_, AppState>,
    username: String,
    password: String,
) -> Result<SessionUser, String> {
    let db = db(&state).await?;
    match blocking(move || auth::login(&db, &username, &password)).await {
        Ok(user) => {
            state.set_session(Some(user.id.clone()));
            Ok(user)
        }
        Err(e) => {
            // Slows down guessing.
            tokio::time::sleep(Duration::from_millis(500)).await;
            Err(e)
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn logout(state: State<'_, AppState>) -> Result<(), String> {
    if let (Some(id), Ok(db)) = (state.session(), db(&state).await) {
        let _ = blocking(move || auth::logout(&db, &id)).await;
    }
    state.set_session(None);
    Ok(())
}

/// Creates the first admin of a new workspace and signs them in. Only
/// allowed while the workspace has no users.
#[tauri::command]
#[specta::specta]
pub async fn create_first_admin(
    state: State<'_, AppState>,
    user: NewUser,
) -> Result<SessionUser, String> {
    let db = db(&state).await?;
    let admin = blocking(move || users::create_first_admin(&db, user)).await?;
    state.set_session(Some(admin.id.clone()));
    Ok(admin)
}

#[tauri::command]
#[specta::specta]
pub async fn change_password(
    state: State<'_, AppState>,
    current: String,
    password: String,
) -> Result<(), String> {
    let (db, user) = signed_in(&state).await?;
    blocking(move || users::change_own_password(&db, &user.id, &current, &password)).await
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct Labeled<T> {
    pub id: T,
    pub label: String,
}

/// Modules, actions and sensitive fields, for the role editor.
#[derive(Debug, Clone, Serialize, Type)]
pub struct PermCatalog {
    pub modules: Vec<Labeled<Module>>,
    pub actions: Vec<Labeled<Action>>,
    pub fields: Vec<Labeled<Field>>,
}

#[tauri::command]
#[specta::specta]
pub fn perm_catalog() -> PermCatalog {
    let cap = |s: &str| {
        let mut c = s.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default()
    };
    PermCatalog {
        modules: Module::ALL
            .into_iter()
            .map(|id| Labeled {
                id,
                label: id.label().into(),
            })
            .collect(),
        actions: Action::ALL
            .into_iter()
            .map(|id| Labeled {
                id,
                label: cap(id.verb().split(' ').next().unwrap_or_default()),
            })
            .collect(),
        fields: Field::ALL
            .into_iter()
            .map(|id| Labeled {
                id,
                label: id.label().into(),
            })
            .collect(),
    }
}
