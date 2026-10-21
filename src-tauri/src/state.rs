//! App state: where the database lives, this PC's sync settings, and the
//! running sync node once the PC belongs to a workspace.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sanco_core::{ChangeEvent, Db, OpenOptions};
use sanco_net::{LocalChanges, Node, NodeConfig, NodeEvent};
use tauri::async_runtime::Mutex;
use tauri::{AppHandle, Emitter};

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
}

impl Args {
    /// Accepts `--flag value` and `--flag=value`; ignores anything else.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut out = Args::default();
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
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
}

pub struct AppState {
    pub app: AppHandle,
    pub data_dir: PathBuf,
    pub machine_id: String,
    pub name: String,
    pub port: u16,
    pub host: Option<String>,
    /// The sync node, once this PC has created or joined a workspace.
    pub node: Mutex<Option<Node>>,
    /// Why the node isn't running, if it failed to start.
    pub error: std::sync::Mutex<Option<String>>,
}

impl AppState {
    pub fn new(app: AppHandle, args: Args, default_dir: PathBuf) -> Self {
        Self {
            app,
            data_dir: args.data_dir.unwrap_or(default_dir),
            machine_id: machine_id(),
            name: args.name.unwrap_or_else(computer_name),
            port: args.port.unwrap_or(sanco_net::DEFAULT_PORT),
            host: args.host,
            node: Mutex::new(None),
            error: std::sync::Mutex::new(None),
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("sanco.db")
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
                let _ = app.emit("renumbered", &list);
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

    /// Opens the database at startup and starts syncing if this PC already
    /// belongs to a workspace.
    pub async fn resume(&self) {
        if !self.db_path().exists() {
            return;
        }
        let changes = LocalChanges::new();
        let opts = self.open_options(&changes);
        let opened = tauri::async_runtime::spawn_blocking(move || Db::open(opts)).await;
        let db = match opened {
            Ok(Ok(db)) => db,
            Ok(Err(e)) => return self.set_error(Some(format!("could not open the database: {e}"))),
            Err(e) => return self.set_error(Some(e.to_string())),
        };
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

    pub fn set_error(&self, e: Option<String>) {
        *self.error.lock().unwrap_or_else(|p| p.into_inner()) = e;
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
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
