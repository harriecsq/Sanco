//! Database handle: one writer thread fed by a job channel, plus a small
//! pool of read-only connections. WAL mode, `synchronous=FULL` by default.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use rusqlite::{Connection, OptionalExtension};

use crate::db::migrations;
use crate::db::registry::{self, Row, TABLES};
use crate::error::{Error, Result};
use crate::ids;
use crate::repo::{Actor, Listener, WriteTx, Writer};
use crate::rng::Rng64;
use crate::sync::hlc::{Clock, Hlc, HlcState, SystemClock};
use crate::sync::numbering::{self, ClaimReply, NumberClaim};
use crate::sync::protocol::{ApplyReport, ChangeBatch, WireRow};
use crate::sync::versions;

const MAX_IDLE_READERS: usize = 4;

pub struct OpenOptions {
    pub path: PathBuf,
    /// Identifies this PC. If it (or the database path) differs from what
    /// the file recorded, the database was copied: take a new site id.
    pub machine_id: String,
    pub clock: Arc<dyn Clock>,
    /// Fixed RNG seed for reproducible ids (simulation only).
    pub seed: Option<u64>,
    /// `synchronous=FULL` when true; tests turn it off for speed.
    pub durable: bool,
    /// Take a new site id and epoch, e.g. after restoring a backup.
    pub new_identity: bool,
    pub on_change: Option<Listener>,
}

impl OpenOptions {
    pub fn new(path: impl Into<PathBuf>, machine_id: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            machine_id: machine_id.into(),
            clock: Arc::new(SystemClock),
            seed: None,
            durable: true,
            new_identity: false,
            on_change: None,
        }
    }
}

type Job = Box<dyn FnOnce(&mut Writer) + Send>;

pub struct Db {
    sender: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
    readers: Mutex<Vec<Connection>>,
    path: PathBuf,
    clock: Arc<dyn Clock>,
    site_id: String,
    epoch: String,
}

impl Db {
    pub fn open(opts: OpenOptions) -> Result<Db> {
        let path = opts.path;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        std::fs::create_dir_all(&dir)?;

        let mut conn = Connection::open(&path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(
            None,
            "synchronous",
            if opts.durable { "FULL" } else { "OFF" },
        )?;
        conn.pragma_update(None, "foreign_keys", false)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        let now = opts.clock.now_ms();
        migrations::run(&mut conn, &dir.join("backups"), now)?;

        let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        let fingerprint = format!("{}|{}", opts.machine_id, canonical.display());
        let stored: Option<(String, String, String, i64, i64)> = conn
            .query_row(
                "SELECT site_id, epoch, fingerprint, last_hlc, ver FROM meta WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (last_hlc, ver) = stored.as_ref().map_or((0, 0), |s| (s.3, s.4));

        let mut rng = match opts.seed {
            // Mix in local state so a reopened simulated peer never repeats ids.
            Some(seed) => Rng64::seeded(
                seed ^ (ver as u64).rotate_left(32)
                    ^ last_hlc as u64
                    ^ (now as u64).rotate_left(16),
            ),
            None => Rng64::from_os(),
        };

        let (site_id, epoch) = match stored {
            Some((site, epoch, fp, ..)) if fp == fingerprint && !opts.new_identity => (site, epoch),
            Some((old_site, ..)) => {
                let (site, epoch) = (ids::new_random(&mut rng), ids::new_random(&mut rng));
                tracing::warn!(%site, "database copied or restored: new site id");
                if opts.new_identity {
                    // Restored backup: this PC takes over the old site's numbers.
                    conn.execute(
                        "INSERT OR IGNORE INTO previous_sites (site_id, retired_at) VALUES (?1, ?2)",
                        (&old_site, now),
                    )?;
                } else {
                    // A copy (e.g. a join snapshot): the old site lives on elsewhere.
                    conn.execute("DELETE FROM previous_sites", [])?;
                }
                conn.execute(
                    "UPDATE meta SET site_id = ?1, epoch = ?2, fingerprint = ?3 WHERE id = 1",
                    (&site, &epoch, &fingerprint),
                )?;
                (site, epoch)
            }
            None => {
                let (site, epoch) = (ids::new_random(&mut rng), ids::new_random(&mut rng));
                conn.execute(
                    "INSERT INTO meta (id, site_id, epoch, fingerprint, last_hlc, ver)
                     VALUES (1, ?1, ?2, ?3, 0, 0)",
                    (&site, &epoch, &fingerprint),
                )?;
                (site, epoch)
            }
        };

        let clock = opts.clock;
        let mut writer = Writer {
            conn,
            site: site_id.clone(),
            clock: clock.clone(),
            rng,
            hlc: HlcState::new(Hlc(last_hlc)),
            ver,
            listener: opts.on_change,
        };
        let (sender, jobs) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("sanco-db-writer".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    job(&mut writer);
                }
            })?;

        Ok(Db {
            sender: Some(sender),
            thread: Some(thread),
            readers: Mutex::new(Vec::new()),
            path,
            clock,
            site_id,
            epoch,
        })
    }

    pub fn site_id(&self) -> &str {
        &self.site_id
    }

    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// A fresh row id, for callers that need it before the write (e.g. to
    /// reserve a document number first).
    pub fn new_id(&self) -> Result<String> {
        self.call(|w| Ok(ids::new_v7(w.clock.now_ms(), &mut w.rng)))
    }

    /// Answers a number claim from a reservation on this PC or a peer.
    pub fn claim_number(&self, claim: NumberClaim) -> Result<ClaimReply> {
        self.call(move |w| {
            let now = w.clock.now_ms();
            let tx = w.conn.transaction()?;
            let reply = numbering::handle_claim(&tx, &claim, now)?;
            tx.commit()?;
            Ok(reply)
        })
    }

    /// Site ids this database used before it was restored from a backup.
    pub fn previous_sites(&self) -> Result<Vec<String>> {
        self.read(|c| {
            let mut stmt = c.prepare("SELECT site_id FROM previous_sites ORDER BY site_id")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// Runs `f` in one write transaction on the writer thread.
    pub fn write<T, F>(&self, actor: Actor, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut WriteTx) -> Result<T> + Send + 'static,
    {
        self.call(move |w| w.write(&actor, f))
    }

    /// Runs `f` on a read-only connection.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        self.with_reader(|c| f(c))
    }

    /// Rows changed after `since` (a version number on this PC), for a peer.
    pub fn changes_since(&self, since: i64, limit: usize) -> Result<ChangeBatch> {
        let limit = limit.max(1);
        self.with_reader(|conn| {
            // One read transaction, so the head and the rows match.
            let tx = conn.transaction()?;
            let head: i64 = tx.query_row("SELECT ver FROM meta WHERE id = 1", [], |r| r.get(0))?;
            let mut list = versions::changed_since(&tx, since, limit + 1)?;
            let more = list.len() > limit;
            list.truncate(limit);
            let mut rows = Vec::with_capacity(list.len());
            for (t, id, _) in &list {
                let def = registry::table(t)?;
                if let Some(row) = def.load(&tx, id)? {
                    rows.push(WireRow { t: t.clone(), row });
                }
            }
            let upto = match list.last() {
                Some((.., ver)) if more => *ver,
                _ => head.max(since),
            };
            Ok(ChangeBatch {
                site: self.site_id.clone(),
                epoch: self.epoch.clone(),
                schema: migrations::SCHEMA_VERSION,
                head,
                upto,
                more,
                rows,
            })
        })
    }

    /// Merges a batch pulled from a peer and advances that peer's watermark.
    pub fn apply_batch(&self, batch: ChangeBatch) -> Result<ApplyReport> {
        self.call(move |w| w.apply_batch(&batch))
    }

    /// Last version pulled from `site` (0 if never).
    pub fn watermark(&self, site: &str) -> Result<i64> {
        self.read(|c| Ok(versions::watermark(c, site)?.map_or(0, |(_, v)| v)))
    }

    /// Consistent copy of the database file.
    pub fn backup_to(&self, dest: impl Into<PathBuf>) -> Result<()> {
        let dest = dest.into();
        self.call(move |w| {
            if let Some(dir) = dest.parent() {
                std::fs::create_dir_all(dir)?;
            }
            w.conn.execute("VACUUM INTO ?1", [dest.to_string_lossy()])?;
            Ok(())
        })
    }

    /// All replicated rows, by table, ordered by id.
    pub fn dump(&self) -> Result<BTreeMap<String, Vec<Row>>> {
        self.read(|c| {
            TABLES
                .iter()
                .map(|t| Ok((t.name.to_string(), t.load_all(c)?)))
                .collect()
        })
    }

    pub(crate) fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Writer) -> Result<T> + Send + 'static,
    {
        let (reply, answer) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |w| {
            let _ = reply.send(f(w));
        });
        self.sender
            .as_ref()
            .ok_or(Error::WriterGone)?
            .send(job)
            .map_err(|_| Error::WriterGone)?;
        answer.recv().map_err(|_| Error::WriterGone)?
    }

    fn with_reader<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let pooled = self.readers.lock().unwrap_or_else(|e| e.into_inner()).pop();
        let mut conn = match pooled {
            Some(c) => c,
            None => {
                let c = Connection::open(&self.path)?;
                c.pragma_update(None, "query_only", true)?;
                c.busy_timeout(std::time::Duration::from_secs(5))?;
                c
            }
        };
        let out = f(&mut conn);
        let mut pool = self.readers.lock().unwrap_or_else(|e| e.into_inner());
        if pool.len() < MAX_IDLE_READERS {
            pool.push(conn);
        }
        out
    }
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db")
            .field("path", &self.path)
            .field("site_id", &self.site_id)
            .finish_non_exhaustive()
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        // Closing the channel ends the writer loop; wait so the file is free.
        self.sender.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
