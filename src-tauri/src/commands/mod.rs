//! Tauri commands. Every command that reads or writes business data first
//! checks the signed-in user's permissions here, in Rust; the UI only hides
//! what a user can't use.

pub mod auth;
pub mod settings;
pub mod sync;
pub mod users;

use std::sync::Arc;

use sanco_core::Db;
use sanco_core::domain::auth::{Action, Module, SessionUser, load_user};

use crate::state::AppState;

/// Runs blocking database work off the async threads.
pub async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> sanco_core::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// The database and the signed-in user, with permissions read just now.
/// A deactivated or deleted user is signed out.
pub async fn signed_in(state: &AppState) -> Result<(Arc<Db>, SessionUser), String> {
    let db = sync::db(state).await?;
    let id = state
        .session()
        .ok_or_else(|| sanco_core::Error::NotSignedIn.to_string())?;
    let read = db.clone();
    match blocking(move || read.read(|c| load_user(c, &id))).await? {
        Some(user) => Ok((db, user)),
        None => {
            state.set_session(None);
            Err("your account is no longer active; sign in again".into())
        }
    }
}

pub async fn require(
    state: &AppState,
    module: Module,
    action: Action,
) -> Result<(Arc<Db>, SessionUser), String> {
    let (db, user) = signed_in(state).await?;
    user.require(module, action).map_err(|e| e.to_string())?;
    Ok((db, user))
}

pub async fn require_admin(state: &AppState) -> Result<(Arc<Db>, SessionUser), String> {
    let (db, user) = signed_in(state).await?;
    user.require_admin().map_err(|e| e.to_string())?;
    Ok((db, user))
}
