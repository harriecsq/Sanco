//! Settings → Devices, Conflicts, Backups, Audit log, and this PC's options.

use sanco_core::backup::{self, BackupFile};
use sanco_core::db::registry::TABLES;
use sanco_core::domain::audit::{self, AuditEntry, AuditFilter};
use sanco_core::domain::auth::{Action, Module};
use sanco_core::domain::conflicts::{self, ConflictInfo};
use serde::Serialize;
use specta::Type;
use tauri::State;
use tauri_plugin_autostart::ManagerExt;

use crate::commands::{blocking, require, require_admin};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Type)]
pub struct DeviceInfo {
    /// The device's site id.
    pub site: String,
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub app_version: Option<String>,
    pub schema_version: Option<i64>,
    /// When the device last refreshed its row (about hourly while running).
    pub last_seen: Option<i64>,
    pub this_pc: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn list_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, String> {
    let (db, _) = require(&state, Module::Settings, Action::View).await?;
    let own = db.site_id().to_string();
    let list = blocking(move || db.devices()).await?;
    Ok(list
        .into_iter()
        .map(|d| DeviceInfo {
            this_pc: d.site == own,
            site: d.site,
            name: d.name,
            host: d.host,
            port: d.port,
            app_version: d.app_version,
            schema_version: d.schema_version,
            last_seen: d.last_seen,
        })
        .collect())
}

/// Removes another PC from the workspace. It stops syncing once it hears.
#[tauri::command]
#[specta::specta]
pub async fn remove_device(state: State<'_, AppState>, site: String) -> Result<(), String> {
    let (db, admin) = require_admin(&state).await?;
    blocking(move || db.remove_device(admin.actor(), &site)).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_conflicts(
    state: State<'_, AppState>,
    include_resolved: bool,
) -> Result<Vec<ConflictInfo>, String> {
    let (db, _) = require(&state, Module::Settings, Action::View).await?;
    blocking(move || db.read(|c| conflicts::list(c, include_resolved))).await
}

/// Marks a conflict reviewed; with `restore`, puts the lost values back.
#[tauri::command]
#[specta::specta]
pub async fn resolve_conflict(
    state: State<'_, AppState>,
    id: String,
    restore: bool,
) -> Result<(), String> {
    let (db, user) = require(&state, Module::Settings, Action::Edit).await?;
    blocking(move || db.write(user.actor(), move |tx| conflicts::resolve(tx, &id, restore))).await
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct Backups {
    pub dir: String,
    pub files: Vec<BackupFile>,
    pub keep_daily: u32,
}

#[tauri::command]
#[specta::specta]
pub async fn list_backups(state: State<'_, AppState>) -> Result<Backups, String> {
    require(&state, Module::Settings, Action::View).await?;
    let dir = state.backup_dir();
    let shown = dir.display().to_string();
    let files = blocking(move || backup::list(&dir)).await?;
    Ok(Backups {
        dir: shown,
        files,
        keep_daily: backup::KEEP_DAILY as u32,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn backup_now(state: State<'_, AppState>) -> Result<String, String> {
    let (db, _) = require(&state, Module::Settings, Action::Edit).await?;
    let dir = state.backup_dir();
    let path = blocking(move || backup::manual(&db, &dir)).await?;
    Ok(path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default())
}

/// Replaces this PC's database with a backup. Sync restarts with a new site
/// id; anything newer comes back from the other PCs.
#[tauri::command]
#[specta::specta]
pub async fn restore_backup(state: State<'_, AppState>, name: String) -> Result<(), String> {
    require_admin(&state).await?;
    state.restore(&name).await
}

#[tauri::command]
#[specta::specta]
pub async fn audit_log(
    state: State<'_, AppState>,
    filter: AuditFilter,
) -> Result<Vec<AuditEntry>, String> {
    let (db, _) = require(&state, Module::Settings, Action::View).await?;
    blocking(move || db.read(|c| audit::list(c, &filter))).await
}

/// Table names, for the audit log filter.
#[tauri::command]
#[specta::specta]
pub fn audit_tables() -> Vec<String> {
    let mut names: Vec<String> = TABLES.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    names
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct PcSettings {
    /// Start Sanco (hidden in the tray) when someone signs in to Windows.
    pub autostart: bool,
    /// Test instances started with `--data-dir` can't change autostart.
    pub autostart_available: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn pc_settings(state: State<'_, AppState>) -> Result<PcSettings, String> {
    require(&state, Module::Settings, Action::View).await?;
    Ok(PcSettings {
        autostart: state.app.autolaunch().is_enabled().unwrap_or(false),
        autostart_available: !state.extra_instance,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn set_autostart(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    require(&state, Module::Settings, Action::Edit).await?;
    if state.extra_instance {
        return Err("test instances can't change autostart".into());
    }
    let launch = state.app.autolaunch();
    if enabled {
        launch.enable()
    } else {
        launch.disable()
    }
    .map_err(|e| e.to_string())
}
