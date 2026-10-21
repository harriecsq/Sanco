//! Local change log (`row_versions`) and per-peer pull watermarks.
//!
//! Every local write and every accepted remote row gets the next version
//! number. A peer pulls rows with `ver > watermark`, so changes relay through
//! intermediate PCs. Only the latest version of each row is kept.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;

pub fn bump(conn: &Connection, tbl: &str, id: &str, ver: i64) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO row_versions (tbl, id, ver) VALUES (?1, ?2, ?3)
         ON CONFLICT (tbl, id) DO UPDATE SET ver = excluded.ver",
    )?
    .execute(params![tbl, id, ver])?;
    Ok(())
}

/// `(tbl, id, ver)` after `since`, oldest first, at most `limit` entries.
pub fn changed_since(
    conn: &Connection,
    since: i64,
    limit: usize,
) -> Result<Vec<(String, String, i64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT tbl, id, ver FROM row_versions WHERE ver > ?1 ORDER BY ver LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![since, limit as i64], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// `(epoch, ver)` last pulled from a peer site.
pub fn watermark(conn: &Connection, site: &str) -> Result<Option<(String, i64)>> {
    Ok(conn
        .prepare_cached("SELECT epoch, ver FROM peer_watermarks WHERE site_id = ?1")?
        .query_row([site], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?)
}

pub fn set_watermark(conn: &Connection, site: &str, epoch: &str, ver: i64, now: i64) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO peer_watermarks (site_id, epoch, ver, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (site_id) DO UPDATE
         SET epoch = excluded.epoch, ver = excluded.ver, updated_at = excluded.updated_at",
    )?
    .execute(params![site, epoch, ver, now])?;
    Ok(())
}
