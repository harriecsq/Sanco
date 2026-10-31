//! What the network layer needs from the database: the workspace and its
//! sync key, the handshake checks, the device address book and join
//! snapshots. The transport itself lives in `sanco-net`.

use std::path::PathBuf;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::db::Db;
use crate::db::migrations::SCHEMA_VERSION;
use crate::error::{Error, Result};
use crate::repo::Actor;
use crate::sync::hlc::MAX_SKEW_MS;
use crate::sync::versions;

pub const DEVICES: &str = "devices";

/// A device refreshes `last_seen` this often even when nothing else changed.
/// Rarely, because every refresh is a replicated write.
pub const DEVICE_REFRESH_MS: i64 = 60 * 60 * 1000;

/// The workspace this database belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub workspace_id: String,
    pub name: String,
    /// 32 random bytes as hex. Bearer tokens are derived from it.
    pub sync_key: String,
    pub created_at: i64,
}

/// First message of every exchange: who the peer is and where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub site: String,
    pub epoch: String,
    pub schema: u32,
    pub app_version: String,
    /// The peer's OS (`std::env::consts::OS`), so updates only come from a
    /// PC with the same kind of installer. Older versions ran on Windows only.
    #[serde(default = "windows")]
    pub os: String,
    /// The peer's PC name.
    pub name: String,
    /// The peer's wall clock when it answered.
    pub now_ms: i64,
    /// The peer's latest version number.
    pub head: i64,
}

fn windows() -> String {
    "windows".into()
}

/// Where a join snapshot came from, so the new PC can skip pulling it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub site: String,
    pub epoch: String,
    pub schema: u32,
    /// The sender's version number when the copy was made.
    pub head: i64,
}

/// What this PC advertises in its `devices` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    /// Address peers use to reach this PC (its Tailscale IP). `None` if
    /// unknown; peers still learn it when this PC pokes them.
    pub host: Option<String>,
    pub port: u16,
    pub app_version: String,
}

/// A live `devices` row. Its id is the device's site id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub site: String,
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub app_version: Option<String>,
    pub schema_version: Option<i64>,
    pub last_seen: Option<i64>,
}

impl Device {
    /// `host:port`, if both are known.
    pub fn addr(&self) -> Option<String> {
        match (&self.host, self.port) {
            (Some(h), Some(p)) if !h.is_empty() && p > 0 => Some(join_host_port(h, p)),
            _ => None,
        }
    }
}

/// `host:port`, bracketing IPv6 hosts.
pub fn join_host_port(host: &str, port: i64) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

impl Db {
    pub fn workspace(&self) -> Result<Option<Workspace>> {
        self.read(|c| {
            Ok(c.query_row(
                "SELECT workspace_id, name, sync_key, created_at FROM workspace WHERE id = 1",
                [],
                |r| {
                    Ok(Workspace {
                        workspace_id: r.get(0)?,
                        name: r.get(1)?,
                        sync_key: r.get(2)?,
                        created_at: r.get(3)?,
                    })
                },
            )
            .optional()?)
        })
    }

    /// Starts a new workspace on this PC (the first PC). Others join it.
    pub fn create_workspace(&self, name: &str) -> Result<Workspace> {
        let name = name.trim().to_string();
        self.call(move |w| {
            let exists: bool =
                w.conn
                    .query_row("SELECT EXISTS (SELECT 1 FROM workspace)", [], |r| r.get(0))?;
            if exists {
                return Err(Error::WorkspaceExists);
            }
            let ws = Workspace {
                workspace_id: uuid::Uuid::new_v4().to_string(),
                name,
                sync_key: new_sync_key(),
                created_at: w.clock.now_ms(),
            };
            w.conn.execute(
                "INSERT INTO workspace (id, workspace_id, name, sync_key, created_at)
                 VALUES (1, ?1, ?2, ?3, ?4)",
                params![ws.workspace_id, ws.name, ws.sync_key, ws.created_at],
            )?;
            Ok(ws)
        })
    }

    /// Latest version number on this PC.
    pub fn head(&self) -> Result<i64> {
        self.read(|c| Ok(c.query_row("SELECT ver FROM meta WHERE id = 1", [], |r| r.get(0))?))
    }

    pub fn hello(&self, name: &str, app_version: &str) -> Result<Hello> {
        Ok(Hello {
            site: self.site_id().to_string(),
            epoch: self.epoch().to_string(),
            schema: SCHEMA_VERSION,
            app_version: app_version.to_string(),
            os: std::env::consts::OS.to_string(),
            name: name.to_string(),
            now_ms: self.now_ms(),
            head: self.head()?,
        })
    }

    /// Checks a peer's hello before pulling from it. Same checks as
    /// [`Db::apply_batch`], plus clocks that are behind, so problems show
    /// up before any rows move.
    pub fn check_peer(&self, hello: &Hello) -> Result<()> {
        if hello.schema != SCHEMA_VERSION {
            return Err(Error::SchemaMismatch {
                ours: SCHEMA_VERSION,
                theirs: hello.schema,
            });
        }
        if hello.site == self.site_id() {
            return Err(Error::OwnSite);
        }
        let ahead_ms = hello.now_ms - self.now_ms();
        if ahead_ms.abs() > MAX_SKEW_MS {
            return Err(Error::ClockSkew { ahead_ms });
        }
        let known = self.read(|c| versions::watermark(c, &hello.site))?;
        if let Some((epoch, ver)) = known {
            if epoch != hello.epoch {
                return Err(Error::EpochMismatch {
                    site: hello.site.clone(),
                });
            }
            if hello.head < ver {
                return Err(Error::PeerRewound {
                    site: hello.site.clone(),
                });
            }
        }
        Ok(())
    }

    /// Copies the whole database to `dest` for a PC that is joining.
    pub fn snapshot(&self, dest: impl Into<PathBuf>) -> Result<SnapshotInfo> {
        let dest = dest.into();
        let (site, epoch) = (self.site_id().to_string(), self.epoch().to_string());
        // On the writer thread, so no commit lands between the copy and `head`.
        self.call(move |w| {
            if let Some(dir) = dest.parent() {
                std::fs::create_dir_all(dir)?;
            }
            w.conn.execute("VACUUM INTO ?1", [dest.to_string_lossy()])?;
            Ok(SnapshotInfo {
                site,
                epoch,
                schema: SCHEMA_VERSION,
                head: w.ver,
            })
        })
    }

    /// Run once after opening a join snapshot. The database took a new site
    /// id when it was opened (it was copied); here it records that it
    /// already has everything the sender had, and drops the sender's
    /// pending number claims.
    pub fn finish_join(&self, from: &SnapshotInfo) -> Result<()> {
        if from.site == self.site_id() {
            return Err(Error::OwnSite);
        }
        let from = from.clone();
        self.call(move |w| {
            let now = w.clock.now_ms();
            let tx = w.conn.transaction()?;
            versions::set_watermark(&tx, &from.site, &from.epoch, from.head, now)?;
            tx.execute("DELETE FROM number_claims", [])?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Live devices, this PC included.
    pub fn devices(&self) -> Result<Vec<Device>> {
        self.read(|c| {
            let mut stmt = c.prepare(
                "SELECT id, name, host, port, app_version, schema_version, last_seen
                 FROM devices WHERE _deleted = 0 ORDER BY name, id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(Device {
                    site: r.get(0)?,
                    name: r.get(1)?,
                    host: r.get(2)?,
                    port: r.get(3)?,
                    app_version: r.get(4)?,
                    schema_version: r.get(5)?,
                    last_seen: r.get(6)?,
                })
            })?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// Whether an admin removed this PC (its `devices` row is deleted).
    pub fn is_removed(&self) -> Result<bool> {
        let site = self.site_id().to_string();
        self.read(|c| {
            Ok(c.query_row(
                "SELECT EXISTS (SELECT 1 FROM devices WHERE id = ?1 AND _deleted = 1)",
                [&site],
                |r| r.get(0),
            )?)
        })
    }

    /// Removes another PC from the workspace. Peers stop syncing with it,
    /// and it stops syncing once it learns of the removal. It can't come
    /// back with the same database (deletes always win); it has to join
    /// again, which gives it a new site id.
    pub fn remove_device(&self, actor: Actor, site: &str) -> Result<()> {
        if site == self.site_id() {
            return Err(Error::Invalid("this PC can't remove itself".into()));
        }
        let site = site.to_string();
        self.write(actor, move |tx| {
            if tx.get(DEVICES, &site)?.is_none() {
                return Err(Error::Invalid("that PC is already removed".into()));
            }
            tx.delete(DEVICES, &site)
        })
    }

    /// Writes this PC's `devices` row if anything changed or `last_seen` is
    /// older than [`DEVICE_REFRESH_MS`], and removes the rows of sites this
    /// database was restored from. Returns whether anything was written.
    ///
    /// Fails with [`Error::Deleted`] if an admin removed this PC.
    pub fn register_device(&self, info: &DeviceInfo) -> Result<bool> {
        let retired = self.previous_sites()?;
        let site = self.site_id().to_string();
        let info = info.clone();
        self.write(Actor::system(), move |tx| {
            let mut wrote = false;
            for old in &retired {
                if tx.get(DEVICES, old)?.is_some() {
                    tx.delete(DEVICES, old)?;
                    wrote = true;
                }
            }
            let now = tx.now_ms();
            let fields = json!({
                "name": info.name,
                "host": info.host,
                "port": info.port,
                "app_version": info.app_version,
                "schema_version": SCHEMA_VERSION,
            });
            let stale = match tx.get(DEVICES, &site)? {
                None => true,
                Some(row) => {
                    let seen = row.get("last_seen").and_then(Value::as_i64).unwrap_or(0);
                    let changed = fields
                        .as_object()
                        .into_iter()
                        .flatten()
                        .any(|(k, v)| row.get(k) != Some(v));
                    changed || now - seen >= DEVICE_REFRESH_MS
                }
            };
            if stale {
                let mut data = fields;
                data["last_seen"] = json!(now);
                tx.upsert(DEVICES, &site, data)?;
                wrote = true;
            }
            Ok(wrote)
        })
    }
}

/// 32 bytes from the OS random source, as hex. Each v4 UUID carries 122
/// random bits, so two of them give 244.
fn new_sync_key() -> String {
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    a.as_bytes()
        .iter()
        .chain(b.as_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}
