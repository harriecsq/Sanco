//! Workspace setup and sync status.

use std::sync::Arc;

use sanco_core::Db;
use sanco_core::domain::auth::{self, SessionUser};
use sanco_core::domain::users::{self, NewUser};
use sanco_core::sync::numbering::Reservation;
use sanco_net::{LocalChanges, Node, PeerState};
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::commands::{blocking, require_admin, signed_in};
use crate::state::{AppState, JOIN_TIMEOUT, set_aside_db};

#[derive(Debug, Clone, Serialize, Type)]
pub struct WorkspaceStatus {
    /// This PC belongs to a workspace and sync is running.
    pub ready: bool,
    pub workspace: Option<String>,
    pub pc_name: String,
    pub site_id: Option<String>,
    /// `host:port` peers use for this PC.
    pub addr: Option<String>,
    pub port: u16,
    pub data_dir: String,
    pub app_version: String,
    /// Why sync isn't running, if it failed.
    pub error: Option<String>,
    /// The workspace has no users yet: create the first admin.
    pub needs_admin: bool,
    /// An admin removed this PC from the workspace.
    pub removed: bool,
    /// Still opening the database at startup.
    pub starting: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct PeerInfo {
    pub addr: String,
    pub name: Option<String>,
    pub site: Option<String>,
    pub app_version: Option<String>,
    /// unknown | online | offline | blocked
    pub state: String,
    pub error: Option<String>,
    pub last_pull_ms: Option<i64>,
    pub last_accepted: u32,
    pub last_conflicts: u32,
}

/// Runs `f` with the sync node, or fails if this PC isn't set up.
pub async fn with_node<T>(state: &AppState, f: impl FnOnce(&Node) -> T) -> Result<T, String> {
    let node = state.node.lock().await;
    let node = node.as_ref().ok_or_else(|| {
        state
            .error()
            .unwrap_or_else(|| "this PC is not set up yet".into())
    })?;
    Ok(f(node))
}

/// Reserves the next shared number of `series` for the new row `ref_id`,
/// claiming it with every reachable peer.
pub async fn reserve_number(
    state: &AppState,
    series: &str,
    ref_id: &str,
) -> Result<Reservation, String> {
    let node = state.node.lock().await;
    let node = node
        .as_ref()
        .ok_or_else(|| "this PC is not set up yet".to_string())?;
    node.reserve_number(series, ref_id)
        .await
        .map_err(|e| e.to_string())
}

/// Shared database handle, for commands that read or write data.
pub async fn db(state: &AppState) -> Result<Arc<Db>, String> {
    with_node(state, |n| n.db().clone()).await
}

async fn status(state: &AppState) -> WorkspaceStatus {
    let (ws, site_id, addr, port, db) = {
        let node = state.node.lock().await;
        match node.as_ref() {
            Some(n) => (
                n.db().workspace().ok().flatten(),
                Some(n.db().site_id().to_string()),
                n.addr(),
                n.port(),
                Some(n.db().clone()),
            ),
            None => (None, None, None, state.port, None),
        }
    };
    let needs_admin = match db {
        Some(db) => !blocking(move || db.read(users::any_user))
            .await
            .unwrap_or(true),
        None => false,
    };
    WorkspaceStatus {
        ready: site_id.is_some(),
        workspace: ws.map(|w| w.name),
        pc_name: state.name.clone(),
        site_id,
        addr,
        port,
        data_dir: state.data_dir.display().to_string(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        error: state.error(),
        needs_admin,
        removed: state.is_removed(),
        starting: !state.started.load(std::sync::atomic::Ordering::SeqCst),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn workspace_status(state: State<'_, AppState>) -> Result<WorkspaceStatus, String> {
    Ok(status(&state).await)
}

/// Makes this PC the first one of a new workspace, with its first admin,
/// who is then signed in.
#[tauri::command]
#[specta::specta]
pub async fn create_workspace(
    state: State<'_, AppState>,
    name: String,
    admin: NewUser,
) -> Result<SessionUser, String> {
    if name.trim().is_empty() {
        return Err("enter a workspace name".into());
    }
    auth::check_password(&admin.password).map_err(|e| e.to_string())?;
    users::check_username(&admin.username).map_err(|e| e.to_string())?;
    ensure_not_set_up(&state).await?;
    let changes = LocalChanges::new();
    let opts = state.open_options(&changes);
    let (db, user) = blocking(move || {
        let db = Db::open(opts)?;
        db.create_workspace(&name)?;
        let user = users::create_first_admin(&db, admin)?;
        Ok((db, user))
    })
    .await?;
    state.start(db, changes).await?;
    state.set_session(Some(user.id.clone()));
    enable_autostart(&state);
    Ok(user)
}

/// Joins an existing workspace with a code from a PC that is already in it.
#[tauri::command]
#[specta::specta]
pub async fn join_workspace(
    state: State<'_, AppState>,
    code: String,
) -> Result<WorkspaceStatus, String> {
    ensure_not_set_up(&state).await?;
    set_aside_db(&state.db_path(), "unused")?;
    let changes = LocalChanges::new();
    let opts = state.open_options(&changes);
    let db = sanco_net::join(&code, opts, JOIN_TIMEOUT)
        .await
        .map_err(|e| e.to_string())?;
    state.start(db, changes).await?;
    enable_autostart(&state);
    Ok(status(&state).await)
}

/// Code another PC enters to join. It contains the workspace sync key, so
/// only admins see it.
#[tauri::command]
#[specta::specta]
pub async fn invite_code(state: State<'_, AppState>) -> Result<String, String> {
    require_admin(&state).await?;
    with_node(&state, |n| n.invite_code())
        .await?
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn sync_status(state: State<'_, AppState>) -> Result<Vec<PeerInfo>, String> {
    signed_in(&state).await?;
    with_node(&state, |n| n.peers().into_iter().map(peer_info).collect()).await
}

/// Pulls from every peer now.
#[tauri::command]
#[specta::specta]
pub async fn sync_now(state: State<'_, AppState>) -> Result<Vec<PeerInfo>, String> {
    signed_in(&state).await?;
    let node = state.node.lock().await;
    let node = node.as_ref().ok_or("this PC is not set up yet")?;
    node.sync_now().await;
    Ok(node.peers().into_iter().map(peer_info).collect())
}

/// Tries to start sync again after it failed (e.g. the port was in use).
#[tauri::command]
#[specta::specta]
pub async fn retry_sync(state: State<'_, AppState>) -> Result<WorkspaceStatus, String> {
    state.resume().await;
    Ok(status(&state).await)
}

/// After this PC was removed: sets its database aside so it can join again.
#[tauri::command]
#[specta::specta]
pub async fn leave_workspace(state: State<'_, AppState>) -> Result<WorkspaceStatus, String> {
    state.leave().await?;
    Ok(status(&state).await)
}

fn peer_info(p: sanco_net::PeerStatus) -> PeerInfo {
    let state = match p.state {
        PeerState::Unknown => "unknown",
        PeerState::Online => "online",
        PeerState::Offline => "offline",
        PeerState::Blocked => "blocked",
    };
    PeerInfo {
        addr: p.addr,
        name: p.name,
        site: p.site,
        app_version: p.app_version,
        state: state.into(),
        error: p.error,
        last_pull_ms: p.last_pull_ms,
        last_accepted: p.last_accepted.try_into().unwrap_or(u32::MAX),
        last_conflicts: p.last_conflicts.try_into().unwrap_or(u32::MAX),
    }
}

/// Release builds start with Windows from the first setup on, so the PC
/// keeps syncing. Can be turned off in Settings → This PC.
fn enable_autostart(state: &AppState) {
    if cfg!(debug_assertions) || state.extra_instance {
        return;
    }
    use tauri_plugin_autostart::ManagerExt;
    if let Err(e) = state.app.autolaunch().enable() {
        log::warn!("could not enable autostart: {e}");
    }
}

async fn ensure_not_set_up(state: &AppState) -> Result<(), String> {
    if state.node.lock().await.is_some() {
        return Err("this PC is already set up".into());
    }
    if state.is_removed() {
        return Err("this PC was removed; leave the workspace first".into());
    }
    let path = state.db_path();
    if path.exists() {
        let changes = LocalChanges::new();
        let opts = state.open_options(&changes);
        let has_workspace =
            blocking(move || Db::open(opts).and_then(|db| db.workspace()).map(|w| w.is_some()))
                .await?;
        if has_workspace {
            return Err(format!(
                "{} already holds a workspace, but sync did not start: {}",
                path.display(),
                state.error().unwrap_or_default()
            ));
        }
    }
    Ok(())
}
