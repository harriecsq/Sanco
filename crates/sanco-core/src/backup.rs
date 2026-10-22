//! Backups: consistent `VACUUM INTO` copies in `<data dir>/backups`.
//!
//! - `sanco-daily-YYYY-MM-DD.db`: one per Manila day, the newest
//!   [`KEEP_DAILY`] kept.
//! - `sanco-manual-YYYYMMDD-HHMMSS.db`: made on request, never pruned.
//! - `sanco-before-restore-….db`: the database as it was before a restore.
//! - `pre-migration-v….db`: made by the migration runner.
//!
//! Restoring replaces the database file and must be followed by
//! `Db::open` with `new_identity`, so peers see a new site instead of a
//! site that went back in time.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;

use crate::db::Db;
use crate::db::migrations::SCHEMA_VERSION;
use crate::error::{Error, Result};
use crate::time::{manila_date_string, manila_stamp};

pub const KEEP_DAILY: usize = 14;
const DAILY_PREFIX: &str = "sanco-daily-";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BackupFile {
    pub name: String,
    pub size: u64,
    /// File modification time (UTC ms).
    pub modified_ms: i64,
    /// daily | manual | before-restore | pre-migration | other
    pub kind: String,
}

/// `<dir of the database>/backups`, where the migration runner also writes.
pub fn dir_for(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("backups")
}

/// Makes today's daily backup unless it exists, then prunes old dailies.
/// Returns the new file, if one was made.
pub fn daily(db: &Db, dir: &Path, keep: usize) -> Result<Option<PathBuf>> {
    let dest = dir.join(format!("{DAILY_PREFIX}{}.db", manila_date_string(db.now_ms())));
    if dest.exists() {
        return Ok(None);
    }
    copy_to(db, &dest)?;
    prune_daily(dir, keep)?;
    Ok(Some(dest))
}

/// Makes a backup now.
pub fn manual(db: &Db, dir: &Path) -> Result<PathBuf> {
    let dest = dir.join(format!("sanco-manual-{}.db", manila_stamp(db.now_ms())));
    copy_to(db, &dest)?;
    Ok(dest)
}

/// Every backup in `dir`, newest first.
pub fn list(dir: &Path) -> Result<Vec<BackupFile>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = entry.metadata()?;
        if !meta.is_file() || !name.ends_with(".db") {
            continue;
        }
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as i64);
        out.push(BackupFile {
            kind: kind_of(&name).into(),
            name,
            size: meta.len(),
            modified_ms,
        });
    }
    out.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms).then(b.name.cmp(&a.name)));
    Ok(out)
}

/// Resolves a backup name from the UI to a file in `dir`. Only plain file
/// names of existing `.db` files are accepted.
pub fn resolve(dir: &Path, name: &str) -> Result<PathBuf> {
    let plain = !name.is_empty()
        && name.ends_with(".db")
        && !name.contains(['/', '\\', ':'])
        && name != ".."
        && !name.starts_with('.');
    let path = dir.join(name);
    if !plain || !path.is_file() {
        return Err(Error::Invalid(format!("no backup named {name}")));
    }
    Ok(path)
}

/// Checks that `backup` is a Sanco database of the given workspace that this
/// build can open.
pub fn check_restorable(backup: &Path, workspace_id: &str) -> Result<()> {
    let conn = Connection::open_with_flags(backup, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| Error::Invalid("the backup can't be opened".into()))?;
    let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(Error::SchemaTooNew {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    let ws: Option<String> = conn
        .query_row("SELECT workspace_id FROM workspace WHERE id = 1", [], |r| r.get(0))
        .optional()
        .unwrap_or(None);
    match ws {
        Some(id) if id == workspace_id => Ok(()),
        Some(_) => Err(Error::Invalid("the backup belongs to another workspace".into())),
        None => Err(Error::Invalid(
            "the backup has no workspace (made before setup or by an older version)".into(),
        )),
    }
}

/// Replaces the database at `db_path` with a copy of `backup`. The database
/// must be closed. Open it afterwards with `new_identity`.
pub fn replace_db(db_path: &Path, backup: &Path) -> Result<()> {
    let tmp = db_path.with_extension("restoring");
    std::fs::copy(backup, &tmp)?;
    for ext in ["db-wal", "db-shm"] {
        let side = db_path.with_extension(ext);
        if side.exists() {
            std::fs::remove_file(side)?;
        }
    }
    std::fs::rename(&tmp, db_path)?;
    Ok(())
}

/// Name for the copy made just before a restore.
pub fn before_restore_path(dir: &Path, now_ms: i64) -> PathBuf {
    dir.join(format!("sanco-before-restore-{}.db", manila_stamp(now_ms)))
}

fn copy_to(db: &Db, dest: &Path) -> Result<()> {
    // VACUUM INTO refuses an existing file, and a half-written one must not
    // look like a backup.
    let tmp = dest.with_extension("partial");
    let _ = std::fs::remove_file(&tmp);
    db.backup_to(&tmp)?;
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

fn prune_daily(dir: &Path, keep: usize) -> Result<()> {
    let mut daily: Vec<String> = list(dir)?
        .into_iter()
        .filter(|b| b.kind == "daily")
        .map(|b| b.name)
        .collect();
    // Names sort by date.
    daily.sort_unstable_by(|a, b| b.cmp(a));
    for name in daily.into_iter().skip(keep) {
        std::fs::remove_file(dir.join(name))?;
    }
    Ok(())
}

fn kind_of(name: &str) -> &'static str {
    if name.starts_with(DAILY_PREFIX) {
        "daily"
    } else if name.starts_with("sanco-manual-") {
        "manual"
    } else if name.starts_with("sanco-before-restore-") {
        "before-restore"
    } else if name.starts_with("pre-migration-") {
        "pre-migration"
    } else {
        "other"
    }
}
