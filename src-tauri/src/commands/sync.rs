//! Workspace setup and sync status.

use std::sync::Arc;

use sanco_core::Db;
use sanco_net::{LocalChanges, Node, PeerState};
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::state::{AppState, JOIN_TIMEOUT};

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
    /// Why sync isn't running, if it failed.
    pub error: Option<String>,
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

/// Shared database handle, for commands that read or write data.
pub async fn db(state: &AppState) -> Result<Arc<Db>, String> {
    with_node(state, |n| n.db().clone()).await
}

async fn status(state: &AppState) -> WorkspaceStatus {
    let node = state.node.lock().await;
    let ws = node
        .as_ref()
        .and_then(|n| n.db().workspace().ok().flatten());
    WorkspaceStatus {
        ready: node.is_some(),
        workspace: ws.map(|w| w.name),
        pc_name: state.name.clone(),
        site_id: node.as_ref().map(|n| n.db().site_id().to_string()),
        addr: node.as_ref().and_then(|n| n.addr()),
        port: node.as_ref().map_or(state.port, |n| n.port()),
        data_dir: state.data_dir.display().to_string(),
        error: state.error(),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn workspace_status(state: State<'_, AppState>) -> Result<WorkspaceStatus, String> {
    Ok(status(&state).await)
}

/// Makes this PC the first one of a new workspace.
#[tauri::command]
#[specta::specta]
pub async fn create_workspace(
    state: State<'_, AppState>,
    name: String,
) -> Result<WorkspaceStatus, String> {
    if name.trim().is_empty() {
        return Err("enter a workspace name".into());
    }
    ensure_not_set_up(&state).await?;
    let changes = LocalChanges::new();
    let opts = state.open_options(&changes);
    let db = tauri::async_runtime::spawn_blocking(move || -> sanco_core::Result<Db> {
        let db = Db::open(opts)?;
        db.create_workspace(&name)?;
        Ok(db)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    state.start(db, changes).await?;
    Ok(status(&state).await)
}

/// Joins an existing workspace with a code from a PC that is already in it.
#[tauri::command]
#[specta::specta]
pub async fn join_workspace(
    state: State<'_, AppState>,
    code: String,
) -> Result<WorkspaceStatus, String> {
    ensure_not_set_up(&state).await?;
    set_aside_unused_db(&state)?;
    let changes = LocalChanges::new();
    let opts = state.open_options(&changes);
    let db = sanco_net::join(&code, opts, JOIN_TIMEOUT)
        .await
        .map_err(|e| e.to_string())?;
    state.start(db, changes).await?;
    Ok(status(&state).await)
}

/// Code another PC enters to join. It contains the workspace sync key.
#[tauri::command]
#[specta::specta]
pub async fn invite_code(state: State<'_, AppState>) -> Result<String, String> {
    with_node(&state, |n| n.invite_code())
        .await?
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn sync_status(state: State<'_, AppState>) -> Result<Vec<PeerInfo>, String> {
    with_node(&state, |n| n.peers().into_iter().map(peer_info).collect()).await
}

/// Pulls from every peer now.
#[tauri::command]
#[specta::specta]
pub async fn sync_now(state: State<'_, AppState>) -> Result<Vec<PeerInfo>, String> {
    let node = state.node.lock().await;
    let node = node.as_ref().ok_or("this PC is not set up yet")?;
    node.sync_now().await;
    Ok(node.peers().into_iter().map(peer_info).collect())
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

async fn ensure_not_set_up(state: &AppState) -> Result<(), String> {
    if state.node.lock().await.is_some() {
        return Err("this PC is already set up".into());
    }
    let path = state.db_path();
    if path.exists() {
        let changes = LocalChanges::new();
        let opts = state.open_options(&changes);
        let has_workspace = tauri::async_runtime::spawn_blocking(move || {
            Db::open(opts)
                .and_then(|db| db.workspace())
                .map(|w| w.is_some())
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
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

/// A database without a workspace holds nothing yet; move it out of the way
/// (instead of deleting it) so a join snapshot can take its place.
fn set_aside_unused_db(state: &AppState) -> Result<(), String> {
    let path = state.db_path();
    if !path.exists() {
        return Ok(());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let dest = path.with_extension(format!("unused-{stamp}.db"));
    std::fs::rename(&path, &dest).map_err(|e| e.to_string())?;
    for ext in ["db-wal", "db-shm"] {
        let _ = std::fs::remove_file(path.with_extension(ext));
    }
    Ok(())
}
