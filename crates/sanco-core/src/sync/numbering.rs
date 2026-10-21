//! Shared document numbers (`DR-2026-0001`), client and supplier codes.
//!
//! A number is `(series, period, seq)`; `period` is the Manila year for series
//! that restart yearly and empty otherwise. Each numbered document gets one
//! replicated `doc_numbers` row with a derived id, so the number is stored once
//! and documents look it up by `ref_id`.
//!
//! Getting a number:
//! 1. [`reserve`] picks `max + 1` and claims it on this PC and on every
//!    reachable peer ([`NumberPeer`]). A peer refuses a number it has seen
//!    used or claimed, and every reply carries the peer's own max, so a retry
//!    jumps straight past it.
//! 2. [`assign`] stores the reserved number inside the document's write.
//!
//! PCs that can't reach each other can hand out the same number. After they
//! sync, the holder with the earliest `(assigned_at, id)` keeps it. Only the
//! PC that assigned a later holder ([`resolve_clashes`]) renumbers it, so all
//! PCs agree without coordination; both users get a to-do.

use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::db::Db;
use crate::db::registry::{self, DOC_NUMBERS, Row};
use crate::error::{Error, Result};
use crate::ids;
use crate::repo::{Actor, WriteTx};
use crate::time::manila_year;

/// How long a claim blocks a number for other documents.
pub const CLAIM_TTL_MS: i64 = 10 * 60 * 1000;
/// Claim rounds before [`reserve`] gives up.
pub const MAX_ROUNDS: usize = 20;
const MAX_PAD: u32 = 12;

/// Series used by the app. Their formats are editable in Settings.
pub mod series {
    pub const BD: &str = "BD";
    pub const BOOKING: &str = "BK";
    pub const DR: &str = "DR";
    pub const PAYMENT: &str = "PAY";
    pub const PO: &str = "PO";
    pub const RECEIVING: &str = "RR";
    pub const CLIENT: &str = "CL";
    pub const SUPPLIER: &str = "SUP";
}

/// Display format of a series. Stored as JSON in the setting
/// [`format_key`]; missing fields fall back to [`NumberFormat::default_for`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumberFormat {
    pub prefix: String,
    /// Show the year, e.g. `DR-2026-0001`.
    pub year: bool,
    /// Restart at 1 each year (Manila time).
    pub reset_yearly: bool,
    /// Minimum digits of the sequence number.
    pub pad: u32,
}

#[derive(Deserialize)]
struct PartialFormat {
    prefix: Option<String>,
    year: Option<bool>,
    reset_yearly: Option<bool>,
    pad: Option<u32>,
}

impl NumberFormat {
    /// Documents show and reset by year; master-data codes don't.
    pub fn default_for(series: &str) -> Self {
        let yearly = !matches!(series, series::CLIENT | series::SUPPLIER);
        Self {
            prefix: series.to_string(),
            year: yearly,
            reset_yearly: yearly,
            pad: 4,
        }
    }

    pub fn period(&self, now_ms: i64) -> String {
        if self.reset_yearly {
            manila_year(now_ms).to_string()
        } else {
            String::new()
        }
    }

    /// `PREFIX-YEAR-0001`. The year is the period's, or the current one for
    /// series that don't reset.
    pub fn render(&self, period: &str, now_ms: i64, seq: i64) -> String {
        let mut parts = Vec::new();
        if !self.prefix.is_empty() {
            parts.push(self.prefix.clone());
        }
        if self.year {
            parts.push(if period.is_empty() {
                manila_year(now_ms).to_string()
            } else {
                period.to_string()
            });
        }
        let pad = self.pad.min(MAX_PAD) as usize;
        parts.push(format!("{seq:0pad$}"));
        parts.join("-")
    }
}

/// Settings key holding a series' format.
pub fn format_key(series: &str) -> String {
    format!("number_format.{series}")
}

pub fn number_format(conn: &Connection, series: &str) -> Result<NumberFormat> {
    let mut fmt = NumberFormat::default_for(series);
    let stored: Option<Option<String>> = conn
        .prepare_cached(
            "SELECT value FROM settings WHERE key = ?1 AND _deleted = 0 ORDER BY id LIMIT 1",
        )?
        .query_row([format_key(series)], |r| r.get(0))
        .optional()?;
    if let Some(Some(json)) = stored {
        let p: PartialFormat = serde_json::from_str(&json)?;
        fmt.prefix = p.prefix.unwrap_or(fmt.prefix);
        fmt.year = p.year.unwrap_or(fmt.year);
        fmt.reset_yearly = p.reset_yearly.unwrap_or(fmt.reset_yearly);
        fmt.pad = p.pad.unwrap_or(fmt.pad);
    }
    Ok(fmt)
}

/// Request to hold a number for one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumberClaim {
    pub series: String,
    pub period: String,
    pub seq: i64,
    /// The document the number is for. Re-claiming for the same document is
    /// idempotent and releases its older claims in the series.
    pub ref_id: String,
    /// Site making the reservation.
    pub site: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimReply {
    pub granted: bool,
    /// Highest seq this PC knows in the series and period, other documents
    /// only (0 if none).
    pub max: i64,
}

/// Something that answers number claims: this PC's [`Db`], or a peer over
/// the network (M3). An `Err` counts as unreachable.
pub trait NumberPeer {
    fn claim(&self, claim: &NumberClaim) -> Result<ClaimReply>;
}

impl NumberPeer for Db {
    fn claim(&self, claim: &NumberClaim) -> Result<ClaimReply> {
        self.claim_number(claim.clone())
    }
}

/// Peer side of a claim. Runs on the writer thread.
pub(crate) fn handle_claim(conn: &Connection, c: &NumberClaim, now: i64) -> Result<ClaimReply> {
    conn.prepare_cached(
        "DELETE FROM number_claims
         WHERE expires_at <= ?1 OR (ref_id = ?2 AND series = ?3)",
    )?
    .execute(params![now, c.ref_id, c.series])?;
    let max = max_seq(conn, &c.series, &c.period, &c.ref_id, now)?;
    let taken: bool = conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM doc_numbers
                            WHERE series = ?1 AND period = ?2 AND seq = ?3
                              AND _deleted = 0 AND ref_id IS NOT ?4)
                 OR EXISTS (SELECT 1 FROM number_claims
                            WHERE series = ?1 AND period = ?2 AND seq = ?3)",
        )?
        .query_row(params![c.series, c.period, c.seq, c.ref_id], |r| r.get(0))?;
    if !taken {
        conn.prepare_cached(
            "INSERT INTO number_claims (series, period, seq, ref_id, site_id, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![
            c.series,
            c.period,
            c.seq,
            c.ref_id,
            c.site,
            now + CLAIM_TTL_MS
        ])?;
    }
    Ok(ClaimReply {
        granted: !taken,
        max,
    })
}

/// Highest seq used or actively claimed in a series and period, ignoring
/// `ref_id`'s own number and claims.
fn max_seq(conn: &Connection, series: &str, period: &str, ref_id: &str, now: i64) -> Result<i64> {
    let max: Option<i64> = conn
        .prepare_cached(
            "SELECT MAX(m) FROM (
                 SELECT MAX(seq) AS m FROM doc_numbers
                 WHERE series = ?1 AND period = ?2 AND ref_id IS NOT ?3
                 UNION ALL
                 SELECT MAX(seq) FROM number_claims
                 WHERE series = ?1 AND period = ?2 AND ref_id <> ?3 AND expires_at > ?4)",
        )?
        .query_row(params![series, period, ref_id, now], |r| r.get(0))?;
    Ok(max.unwrap_or(0))
}

/// A number held for a document, ready for [`assign`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reservation {
    pub series: String,
    pub period: String,
    pub seq: i64,
    pub no: String,
    pub ref_id: String,
    /// Peers that answered the final round.
    pub reached: usize,
    /// Peers that didn't. If any, the number may still change after a sync.
    pub unreachable: usize,
}

impl Reservation {
    pub fn confirmed(&self) -> bool {
        self.unreachable == 0
    }
}

/// Reserves the next number of `series` for document `ref_id`.
/// `peers` are the other PCs; unreachable ones are skipped.
pub fn reserve(
    local: &Db,
    peers: &[&dyn NumberPeer],
    series: &str,
    ref_id: &str,
) -> Result<Reservation> {
    let now = local.now_ms();
    let fmt = local.read(|c| number_format(c, series))?;
    let period = fmt.period(now);
    reserve_in(local, peers, &fmt, series, &period, ref_id)
}

fn reserve_in(
    local: &Db,
    peers: &[&dyn NumberPeer],
    fmt: &NumberFormat,
    series: &str,
    period: &str,
    ref_id: &str,
) -> Result<Reservation> {
    let now = local.now_ms();
    let mut seq = local.read(|c| max_seq(c, series, period, ref_id, now))? + 1;
    for _ in 0..MAX_ROUNDS {
        let claim = NumberClaim {
            series: series.to_string(),
            period: period.to_string(),
            seq,
            ref_id: ref_id.to_string(),
            site: local.site_id().to_string(),
        };
        let mine = local.claim_number(claim.clone())?;
        let (mut granted, mut max) = (mine.granted, mine.max);
        let (mut reached, mut unreachable) = (0, 0);
        if granted && max < seq {
            for peer in peers {
                match peer.claim(&claim) {
                    Ok(r) => {
                        reached += 1;
                        granted &= r.granted;
                        max = max.max(r.max);
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "number claim: peer skipped");
                        unreachable += 1;
                    }
                }
            }
        }
        if granted && max < seq {
            return Ok(Reservation {
                series: claim.series,
                period: claim.period,
                seq,
                no: fmt.render(period, now, seq),
                ref_id: claim.ref_id,
                reached,
                unreachable,
            });
        }
        seq = max.max(seq) + 1;
    }
    Err(Error::NumberUnavailable {
        series: series.to_string(),
        rounds: MAX_ROUNDS,
    })
}

/// Id of a document's number row in a series.
pub fn number_row_id(ref_id: &str, series: &str) -> String {
    ids::derived_id(ref_id, &format!("number/{series}"))
}

/// Stores a reserved number for document `ref_tbl`/`ref_id` (the one it was
/// reserved for). Call inside the write that creates the document.
pub fn assign(tx: &mut WriteTx, res: &Reservation, ref_tbl: &str) -> Result<String> {
    let id = number_row_id(&res.ref_id, &res.series);
    let (site, now) = (tx.site().to_string(), tx.now_ms());
    tx.insert(
        DOC_NUMBERS,
        json!({
            "id": id,
            "series": res.series,
            "period": res.period,
            "seq": res.seq,
            "no": res.no,
            "ref_tbl": ref_tbl,
            "ref_id": res.ref_id,
            "site": site,
            "assigned_at": now,
            "confirmed": res.confirmed(),
        }),
    )?;
    Ok(res.no.clone())
}

/// A document's current number in a series.
pub fn number_of(conn: &Connection, series: &str, ref_id: &str) -> Result<Option<String>> {
    let def = registry::table(DOC_NUMBERS)?;
    Ok(def
        .load(conn, &number_row_id(ref_id, series))?
        .filter(|r| r["_deleted"] == json!(0))
        .and_then(|r| r["no"].as_str().map(String::from)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Renumbered {
    pub series: String,
    pub ref_tbl: String,
    pub ref_id: String,
    pub old_no: String,
    pub new_no: String,
    pub confirmed: bool,
}

/// Renumbers documents this PC (or a site it was restored from) numbered
/// that lost a clash. Run after merging a batch from a peer.
pub fn resolve_clashes(db: &Db, peers: &[&dyn NumberPeer]) -> Result<Vec<Renumbered>> {
    let mut owned: BTreeSet<String> = db.previous_sites()?.into_iter().collect();
    owned.insert(db.site_id().to_string());
    let losers: Vec<Row> = db
        .read(clash_losers)?
        .into_iter()
        .filter(|r| owned.contains(text(r, "site")))
        .collect();

    let mut done = Vec::new();
    for loser in losers {
        let series = text(&loser, "series").to_string();
        let fmt = db.read(|c| number_format(c, &series))?;
        let res = reserve_in(
            db,
            peers,
            &fmt,
            &series,
            text(&loser, "period"),
            text(&loser, "ref_id"),
        )?;
        if let Some(r) = db.write(Actor::system(), move |tx| renumber(tx, &loser, &res))? {
            tracing::info!(old = %r.old_no, new = %r.new_no, "renumbered after a clash");
            done.push(r);
        }
    }
    Ok(done)
}

/// Live number rows that share their number with an earlier holder.
fn clash_losers(conn: &Connection) -> Result<Vec<Row>> {
    let def = registry::table(DOC_NUMBERS)?;
    let sql = format!(
        "{} AS d WHERE d._deleted = 0 AND EXISTS (
             SELECT 1 FROM doc_numbers o
             WHERE o._deleted = 0 AND o.series = d.series AND o.period = d.period
               AND o.seq = d.seq AND (o.assigned_at, o.id) < (d.assigned_at, d.id))
         ORDER BY d.assigned_at, d.id",
        def.select_sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| def.row_from_sql(r))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Earliest live holder of a number.
fn first_holder(conn: &Connection, series: &str, period: &str, seq: i64) -> Result<Option<Row>> {
    let def = registry::table(DOC_NUMBERS)?;
    let sql = format!(
        "{} WHERE _deleted = 0 AND series = ?1 AND period = ?2 AND seq = ?3
         ORDER BY assigned_at, id LIMIT 1",
        def.select_sql()
    );
    Ok(conn
        .prepare_cached(&sql)?
        .query_row(params![series, period, seq], |r| def.row_from_sql(r))
        .optional()?)
}

fn renumber(tx: &mut WriteTx, seen: &Row, res: &Reservation) -> Result<Option<Renumbered>> {
    let id = text(seen, "id").to_string();
    // Re-check: the row may have changed since the clash was found.
    let Some(row) = tx.get(DOC_NUMBERS, &id)? else {
        return Ok(None);
    };
    let (series, period, old_seq) = (text(&row, "series"), text(&row, "period"), int(&row, "seq"));
    if old_seq != int(seen, "seq") {
        return Ok(None);
    }
    let Some(winner) = first_holder(tx.conn(), series, period, old_seq)? else {
        return Ok(None);
    };
    if text(&winner, "id") == id {
        return Ok(None);
    }

    let out = Renumbered {
        series: series.to_string(),
        ref_tbl: text(&row, "ref_tbl").to_string(),
        ref_id: text(&row, "ref_id").to_string(),
        old_no: text(&row, "no").to_string(),
        new_no: res.no.clone(),
        confirmed: res.confirmed(),
    };
    let now = tx.now_ms();
    tx.update(
        DOC_NUMBERS,
        &id,
        json!({
            "seq": res.seq,
            "no": res.no,
            "prev_no": out.old_no,
            "assigned_at": now,
            "confirmed": res.confirmed(),
        }),
    )?;

    // Tell whoever created each of the two documents.
    let users: BTreeSet<&str> = [&row, &winner]
        .iter()
        .filter_map(|r| r["created_by"].as_str())
        .collect();
    let todos = registry::table("todos")?;
    for user in users {
        let todo_id = ids::derived_id(&id, &format!("renumbered/{old_seq}/{user}"));
        if todos.load(tx.conn(), &todo_id)?.is_some() {
            continue;
        }
        tx.insert(
            "todos",
            json!({
                "id": todo_id,
                "title": format!("{} renumbered to {}", out.old_no, out.new_no),
                "notes": format!(
                    "Two PCs gave out {} while they couldn't reach each other. \
                     The earlier record keeps it; this one is now {}.",
                    out.old_no, out.new_no
                ),
                "assignee_id": user,
                "done": 0,
                "ref_tbl": out.ref_tbl,
                "ref_id": out.ref_id,
            }),
        )?;
    }
    Ok(Some(out))
}

fn text<'r>(row: &'r Row, col: &str) -> &'r str {
    row.get(col).and_then(Value::as_str).unwrap_or_default()
}

fn int(row: &Row, col: &str) -> i64 {
    row.get(col).and_then(Value::as_i64).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_792_540_800_000;

    #[test]
    fn formats_render() {
        let dr = NumberFormat::default_for(series::DR);
        assert_eq!(dr.period(T0), "2026");
        assert_eq!(dr.render("2026", T0, 1), "DR-2026-0001");
        assert_eq!(dr.render("2026", T0, 12345), "DR-2026-12345");

        let cl = NumberFormat::default_for(series::CLIENT);
        assert_eq!(cl.period(T0), "");
        assert_eq!(cl.render("", T0, 7), "CL-0007");

        let odd = NumberFormat {
            prefix: String::new(),
            year: true,
            reset_yearly: false,
            pad: 99,
        };
        assert_eq!(odd.render("", T0, 5), "2026-000000000005");
    }
}
