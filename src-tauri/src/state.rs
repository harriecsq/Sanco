//! App state: where the database lives, this PC's sync settings, the
//! running sync node once the PC belongs to a workspace, and who is signed in.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sanco_core::{ChangeEvent, Db, OpenOptions, backup};
use sanco_net::{LocalChanges, Node, NodeConfig, NodeEvent};
use tauri::async_runtime::Mutex;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

/// Command-line flags. `--data-dir`, `--port` and `--host` let several
/// instances run on one PC for testing.
#[derive(Debug, Default, Clone)]
pub struct Args {
    pub data_dir: Option<PathBuf>,
    pub port: Option<u16>,
    /// Address advertised to peers instead of the Tailscale IP.
    pub host: Option<String>,
    /// PC name shown to peers instead of the Windows computer name.
    pub name: Option<String>,
    /// Start hidden in the tray (used by autostart).
    pub minimized: bool,
}

impl Args {
    /// Accepts `--flag value` and `--flag=value`; ignores anything else.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut out = Args::default();
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            if arg == "--minimized" {
                out.minimized = true;
                continue;
            }
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) => (f.to_string(), Some(v.to_string())),
                None => (arg, None),
            };
            let known = ["--data-dir", "--port", "--host", "--name"];
            if !known.contains(&flag.as_str()) {
                continue;
            }
            let Some(value) = inline.or_else(|| it.next()) else {
                break;
            };
            match flag.as_str() {
                "--data-dir" => out.data_dir = Some(value.into()),
                "--port" => out.port = value.parse().ok(),
                "--host" => out.host = Some(value),
                _ => out.name = Some(value),
            }
        }
        out
    }

    /// A test instance next to the main one: no single-instance lock, no
    /// autostart.
    pub fn is_extra_instance(&self) -> bool {
        self.data_dir.is_some()
    }
}

pub struct AppState {
    pub app: AppHandle,
    pub data_dir: PathBuf,
    pub machine_id: String,
    pub name: String,
    pub port: u16,
    pub host: Option<String>,
    pub extra_instance: bool,
    /// The sync node, once this PC has created or joined a workspace.
    pub node: Mutex<Option<Node>>,
    /// Why the node isn't running, if it failed to start.
    pub error: std::sync::Mutex<Option<String>>,
    /// An admin removed this PC; it no longer syncs.
    pub removed: std::sync::Mutex<bool>,
    /// False until the startup `resume` finished, so the UI can wait
    /// instead of showing the setup screen.
    pub started: std::sync::atomic::AtomicBool,
    /// The signed-in user's id. Permissions are re-read on every command.
    session: std::sync::Mutex<Option<String>>,
}

impl AppState {
    pub fn new(app: AppHandle, args: Args, default_dir: PathBuf) -> Self {
        Self {
            app,
            extra_instance: args.is_extra_instance(),
            data_dir: args.data_dir.unwrap_or(default_dir),
            machine_id: machine_id(),
            name: args.name.unwrap_or_else(computer_name),
            port: args.port.unwrap_or(sanco_net::DEFAULT_PORT),
            host: args.host,
            node: Mutex::new(None),
            error: std::sync::Mutex::new(None),
            removed: std::sync::Mutex::new(false),
            started: std::sync::atomic::AtomicBool::new(false),
            session: std::sync::Mutex::new(None),
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("sanco.db")
    }

    pub fn backup_dir(&self) -> PathBuf {
        backup::dir_for(&self.db_path())
    }

    pub fn session(&self) -> Option<String> {
        lock(&self.session).clone()
    }

    /// Signs a user in or out and tells the UI (`session-changed`).
    pub fn set_session(&self, user_id: Option<String>) {
        let changed = {
            let mut s = lock(&self.session);
            let changed = *s != user_id;
            *s = user_id;
            changed
        };
        if changed {
            let _ = self.app.emit("session-changed", ());
        }
    }

    pub fn is_removed(&self) -> bool {
        *lock(&self.removed)
    }

    /// Open options whose change listener wakes the sync node and tells the
    /// UI to refetch (`db-changed`, payload: table names).
    pub fn open_options(&self, changes: &LocalChanges) -> OpenOptions {
        let app = self.app.clone();
        let ui = Arc::new(move |ev: &ChangeEvent| {
            let _ = app.emit("db-changed", &ev.tables);
        });
        let mut opts = OpenOptions::new(self.db_path(), &self.machine_id);
        opts.on_change = Some(changes.listener(Some(ui)));
        opts
    }

    /// Starts syncing. Emits `sync-status` when peers change and
    /// `renumbered` when this PC's documents got new numbers.
    pub async fn start(&self, db: Db, changes: LocalChanges) -> Result<(), String> {
        let mut cfg = NodeConfig::new(&self.name, env!("CARGO_PKG_VERSION"));
        cfg.port = self.port;
        cfg.host = self.host.clone();
        let app = self.app.clone();
        let on_event = Arc::new(move |ev: NodeEvent| match ev {
            NodeEvent::PeersChanged => {
                let _ = app.emit("sync-status", ());
            }
            NodeEvent::Renumbered(list) => {
                let body = list
                    .iter()
                    .map(|r| format!("{} is now {}", r.old_no, r.new_no))
                    .collect::<Vec<_>>()
                    .join("\n");
                let _ = app
                    .notification()
                    .builder()
                    .title("Document numbers changed")
                    .body(body)
                    .show();
                let _ = app.emit("renumbered", &list);
            }
            NodeEvent::Removed => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Manager;
                    app.state::<AppState>().on_removed().await;
                });
            }
        });
        let started = Node::start(Arc::new(db), changes, cfg, on_event).await;
        let mut slot = self.node.lock().await;
        match started {
            Ok(node) => {
                *slot = Some(node);
                self.set_error(None);
                Ok(())
            }
            Err(e) => {
                let msg = format!("sync could not start on port {}: {e}", self.port);
                self.set_error(Some(msg.clone()));
                Err(msg)
            }
        }
    }

    /// Opens the database and starts syncing if this PC belongs to a
    /// workspace. Runs at startup and from "Retry" after a failed start.
    pub async fn resume(&self) {
        if !self.db_path().exists() || self.node.lock().await.is_some() {
            return;
        }
        let changes = LocalChanges::new();
        let opts = self.open_options(&changes);
        let opened = tauri::async_runtime::spawn_blocking(move || {
            let db = Db::open(opts)?;
            let removed = db.is_removed()?;
            Ok::<_, sanco_core::Error>((db, removed))
        })
        .await;
        let (db, removed) = match opened {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return self.set_error(Some(format!("could not open the database: {e}"))),
            Err(e) => return self.set_error(Some(e.to_string())),
        };
        if removed {
            *lock(&self.removed) = true;
            return;
        }
        match db.workspace() {
            Ok(Some(_)) => {
                if let Err(e) = self.start(db, changes).await {
                    log::error!("{e}");
                }
            }
            Ok(None) => {} // Created but never set up; create/join replaces it.
            Err(e) => self.set_error(Some(e.to_string())),
        }
    }

    /// Stops the sync node and waits until nothing else holds the database,
    /// so the file can be replaced. Returns the database, still open.
    pub async fn stop(&self) -> Result<Db, String> {
        let node = self
            .node
            .lock()
            .await
            .take()
            .ok_or("this PC is not set up yet")?;
        let mut db = node.db().clone();
        node.shutdown().await;
        // Commands in flight hold clones for a moment.
        for _ in 0..100 {
            match Arc::try_unwrap(db) {
                Ok(db) => return Ok(db),
                Err(shared) => db = shared,
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("the database is still in use; try again".into())
    }

    /// Replaces the database with a backup and starts again as a new site.
    /// The current database is backed up first.
    pub async fn restore(&self, name: &str) -> Result<(), String> {
        let src = backup::resolve(&self.backup_dir(), name).map_err(|e| e.to_string())?;
        let db = self.stop().await?;
        let dir = self.backup_dir();
        let path = self.db_path();
        let (outcome, kept, replaced) = tauri::async_runtime::spawn_blocking(move || {
            // Checked and saved while the current database is still open.
            let check = (|| {
                let ws = db.workspace()?.ok_or(sanco_core::Error::NoWorkspace)?;
                backup::check_restorable(&src, &ws.workspace_id)?;
                db.backup_to(backup::before_restore_path(&dir, db.now_ms()))
            })();
            if let Err(e) = check {
                return (Err(e.to_string()), Some(db), false);
            }
            drop(db);
            match backup::replace_db(&path, &src) {
                Ok(()) => (Ok(()), None, true),
                Err(e) => (Err(format!("restore failed: {e}")), None, false),
            }
        })
        .await
        .map_err(|e| e.to_string())?;
        let changes = LocalChanges::new();
        let db = match kept {
            Some(db) => db,
            None => {
                let mut opts = self.open_options(&changes);
                // A restored file must not pass for the site it was copied from.
                opts.new_identity = replaced;
                tauri::async_runtime::spawn_blocking(move || Db::open(opts))
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| format!("could not open the database: {e}"))?
            }
        };
        self.start(db, changes).await?;
        let _ = self.app.emit("db-changed", Vec::<String>::new());
        outcome
    }

    /// An admin removed this PC: stop syncing and tell the UI.
    pub async fn on_removed(&self) {
        {
            let mut removed = lock(&self.removed);
            if *removed {
                return;
            }
            *removed = true;
        }
        log::warn!("this PC was removed from the workspace; sync stopped");
        if let Some(node) = self.node.lock().await.take() {
            node.shutdown().await;
        }
        let _ = self.app.emit("sync-status", ());
    }

    /// After a removal: set the old database aside so this PC can join again.
    pub async fn leave(&self) -> Result<(), String> {
        if !self.is_removed() {
            return Err("this PC is still part of the workspace".into());
        }
        if let Some(node) = self.node.lock().await.take() {
            node.shutdown().await;
        }
        set_aside_db(&self.db_path(), "removed")?;
        *lock(&self.removed) = false;
        self.set_error(None);
        self.set_session(None);
        Ok(())
    }

    /// Makes today's backup if it's missing. Runs every few minutes.
    pub async fn daily_backup(&self) {
        let db = match self.node.lock().await.as_ref() {
            Some(n) => n.db().clone(),
            None => return,
        };
        let dir = self.backup_dir();
        let res = tauri::async_runtime::spawn_blocking(move || {
            backup::daily(&db, &dir, backup::KEEP_DAILY)
        })
        .await;
        match res {
            Ok(Ok(Some(path))) => log::info!("backup saved to {}", path.display()),
            Ok(Ok(None)) => {}
            Ok(Err(e)) => log::error!("daily backup failed: {e}"),
            Err(e) => log::error!("daily backup failed: {e}"),
        }
    }

    pub fn set_error(&self, e: Option<String>) {
        *lock(&self.error) = e;
    }

    pub fn error(&self) -> Option<String> {
        lock(&self.error).clone()
    }
}

/// Moves `sanco.db` aside as `sanco.<tag>-<unix secs>.db` (never deletes it).
pub fn set_aside_db(path: &std::path::Path, tag: &str) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let dest = path.with_extension(format!("{tag}-{stamp}.db"));
    std::fs::rename(path, &dest).map_err(|e| e.to_string())?;
    for ext in ["db-wal", "db-shm"] {
        let _ = std::fs::remove_file(path.with_extension(ext));
    }
    Ok(())
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Stable id of this Windows install (`MachineGuid`). Changes when the
/// database is copied to another PC, which then takes a new site id.
fn machine_id() -> String {
    let mut cmd = std::process::Command::new("reg");
    cmd.args([
        "query",
        r"HKLM\SOFTWARE\Microsoft\Cryptography",
        "/v",
        "MachineGuid",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output()
        .ok()
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .find(|l| l.contains("MachineGuid"))
                .and_then(|l| l.split_whitespace().last())
                .map(String::from)
        })
        .unwrap_or_else(computer_name)
}

fn computer_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into())
}

/// Timeout for downloading a join snapshot's first bytes.
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(15);
