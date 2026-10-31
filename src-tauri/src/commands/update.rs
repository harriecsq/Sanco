//! App updates handed from PC to PC (see `sanco_net::update`): a banner
//! offers the newest version a peer on the same OS runs; installing
//! downloads that peer's installer, starts it (Windows asks for admin
//! rights) and quits, and the installer starts Sanco again when it's done.
//! On macOS it opens the disk image instead.

use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::commands::signed_in;
use crate::commands::sync::with_node;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Type)]
pub struct UpdateOffer {
    pub version: String,
    /// The PC that has it.
    pub from_name: Option<String>,
    pub addr: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct OlderPeer {
    pub name: Option<String>,
    pub version: String,
    /// It can't sync with this PC until it is updated (other schema).
    pub blocked: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct UpdateStatus {
    pub current: String,
    pub offer: Option<UpdateOffer>,
    /// Peers on an older version.
    pub older: Vec<OlderPeer>,
    /// This PC has its installer to share with them.
    pub can_share: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn update_status(state: State<'_, AppState>) -> Result<UpdateStatus, String> {
    signed_in(&state).await?;
    let current = state.app_version.clone();
    let (offer, peers) = with_node(&state, |n| (n.update_offer(), n.peers())).await?;
    let older = peers
        .into_iter()
        .filter_map(|p| {
            let version = p.app_version?;
            sanco_net::update::is_newer(&current, &version).then(|| OlderPeer {
                name: p.name,
                version,
                blocked: p.state == sanco_net::PeerState::Blocked,
            })
        })
        .collect();
    Ok(UpdateStatus {
        offer: offer.map(|o| UpdateOffer {
            version: o.version,
            from_name: o.name,
            addr: o.addr,
        }),
        older,
        can_share: state.installer.is_file(),
        current,
    })
}

/// Downloads the installer from the PC at `addr`, starts it and quits.
#[tauri::command]
#[specta::specta]
pub async fn install_update(state: State<'_, AppState>, addr: String) -> Result<(), String> {
    signed_in(&state).await?;
    let dir = state.data_dir.join("updates");
    let path = {
        let node = state.node.lock().await;
        let node = node.as_ref().ok_or("sync is not running")?;
        node.download_update(&addr, &dir)
            .await
            .map_err(|e| e.to_string())?
    };
    let path = std::path::absolute(&path).unwrap_or(path);
    log::info!("starting installer {}", path.display());
    start_installer(&path).map_err(|e| format!("could not start the installer: {e}"))?;
    state.app.exit(0);
    Ok(())
}

/// `start` goes through ShellExecute, so Windows shows the admin prompt the
/// per-machine installer needs. /P = passive, /R = start Sanco after.
#[cfg(windows)]
fn start_installer(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .args(["/P", "/R"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(drop)
}

/// Opens the disk image in Finder; the user drags Sanco to Applications and
/// starts it again.
#[cfg(target_os = "macos")]
fn start_installer(path: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .map(drop)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn start_installer(_: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "updates are not supported on this system",
    ))
}
