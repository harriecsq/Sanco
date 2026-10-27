//! Schema migrations, tracked with `PRAGMA user_version`.
//!
//! Rules:
//! - Additive only: new tables, new nullable columns, new indexes and views.
//!   Never drop or rename anything a peer on an older build might send.
//! - Backfills must be deterministic and must not stamp HLCs or bump
//!   `row_versions`, so every peer computes the same result locally.
//! - The database is copied with `VACUUM INTO` before any pending migration.

use std::path::Path;

use rusqlite::Connection;

use crate::error::{Error, Result};

const MIGRATIONS: &[&str] = &[
    include_str!("0001_init.sql"),
    include_str!("0002_numbering.sql"),
    include_str!("0003_workspace.sql"),
    include_str!("0004_master_data.sql"),
    include_str!("0005_inventory.sql"),
    include_str!("0006_sales.sql"),
    include_str!("0007_accounting.sql"),
    include_str!("0008_orders.sql"),
    include_str!("0009_bank_accounts.sql"),
    include_str!("0010_shipment_charges.sql"),
];

/// Schema version this build writes. Peers must match it to sync.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

pub fn current_version(conn: &Connection) -> Result<u32> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

/// Applies pending migrations. `backup_dir` receives a pre-migration copy
/// when an existing database is upgraded.
pub fn run(conn: &mut Connection, backup_dir: &Path, now_ms: i64) -> Result<()> {
    let current = current_version(conn)?;
    if current > SCHEMA_VERSION {
        return Err(Error::SchemaTooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }
    if current == SCHEMA_VERSION {
        return Ok(());
    }
    if current > 0 {
        std::fs::create_dir_all(backup_dir)?;
        let dest = backup_dir.join(format!("pre-migration-v{current}-{now_ms}.db"));
        conn.execute("VACUUM INTO ?1", [dest.to_string_lossy()])?;
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as u32)?;
        tx.commit()?;
    }
    Ok(())
}
