//! The only write path.
//!
//! Every change, local or from a peer, runs on the writer thread inside a
//! [`WriteTx`]. Local writes stamp the HLC and version vector, bump
//! `row_versions` and append to `audit_log`. Remote rows go through the merge
//! rule in [`crate::sync::merge`]. After commit the change listener fires so
//! the UI can refetch and the network layer can poke peers.

use std::collections::BTreeSet;
use std::sync::Arc;

use rusqlite::{Connection, Transaction};
use serde_json::Value;

use crate::db::migrations::SCHEMA_VERSION;
use crate::db::registry::{self, AUDIT_LOG, CONFLICTS, Row, TableDef};
use crate::error::{Error, Result};
use crate::ids;
use crate::rng::Rng64;
use crate::sync::hlc::{Clock, Hlc, HlcState};
use crate::sync::merge::{self, Loser, Version, VersionVector};
use crate::sync::protocol::{ApplyReport, ChangeBatch};
use crate::sync::versions;

/// Who is making a change. `user_id` is `None` for system work.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Actor {
    pub user_id: Option<String>,
}

impl Actor {
    pub fn system() -> Self {
        Self { user_id: None }
    }

    pub fn user(id: impl Into<String>) -> Self {
        Self {
            user_id: Some(id.into()),
        }
    }

    fn value(&self) -> Value {
        self.user_id.clone().map_or(Value::Null, Value::from)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Written on this PC: peers should be poked.
    Local,
    /// Merged from a peer.
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEvent {
    pub origin: Origin,
    pub tables: Vec<String>,
}

pub type Listener = Arc<dyn Fn(&ChangeEvent) + Send + Sync>;

/// Columns whose values never appear in the audit log.
const REDACTED: &[&str] = &["pw_hash"];

/// State owned by the writer thread.
pub(crate) struct Writer {
    pub(crate) conn: Connection,
    pub(crate) site: String,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) rng: Rng64,
    pub(crate) hlc: HlcState,
    pub(crate) ver: i64,
    pub(crate) listener: Option<Listener>,
}

impl Writer {
    pub(crate) fn write<T>(
        &mut self,
        actor: &Actor,
        f: impl FnOnce(&mut WriteTx) -> Result<T>,
    ) -> Result<T> {
        let mut wtx = self.begin(actor)?;
        let out = f(&mut wtx)?;
        let changed = wtx.commit()?;
        self.notify(Origin::Local, changed);
        Ok(out)
    }

    pub(crate) fn apply_batch(&mut self, batch: &ChangeBatch) -> Result<ApplyReport> {
        let actor = Actor::system();
        let mut wtx = self.begin(&actor)?;
        let report = wtx.apply_batch(batch)?;
        let changed = wtx.commit()?;
        self.notify(Origin::Remote, changed);
        Ok(report)
    }

    fn begin<'a>(&'a mut self, actor: &'a Actor) -> Result<WriteTx<'a>> {
        Ok(WriteTx {
            tx: self.conn.transaction()?,
            site: &self.site,
            clock: &*self.clock,
            rng: &mut self.rng,
            hlc: &mut self.hlc,
            ver: &mut self.ver,
            actor,
            changed: BTreeSet::new(),
        })
    }

    fn notify(&self, origin: Origin, changed: BTreeSet<&'static str>) {
        if let (Some(listener), false) = (&self.listener, changed.is_empty()) {
            listener(&ChangeEvent {
                origin,
                tables: changed.into_iter().map(String::from).collect(),
            });
        }
    }
}

/// One write transaction. Domain code receives this inside `Db::write`.
pub struct WriteTx<'a> {
    tx: Transaction<'a>,
    site: &'a str,
    clock: &'a dyn Clock,
    rng: &'a mut Rng64,
    hlc: &'a mut HlcState,
    ver: &'a mut i64,
    actor: &'a Actor,
    changed: BTreeSet<&'static str>,
}

impl WriteTx<'_> {
    /// For queries inside the transaction. Never write through it.
    pub fn conn(&self) -> &Connection {
        &self.tx
    }

    pub fn actor(&self) -> &Actor {
        self.actor
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// This PC's site id.
    pub fn site(&self) -> &str {
        self.site
    }

    pub fn new_id(&mut self) -> String {
        ids::new_v7(self.clock.now_ms(), self.rng)
    }

    /// A live (not deleted) row.
    pub fn get(&self, table: &str, id: &str) -> Result<Option<Row>> {
        let def = registry::table(table)?;
        Ok(def.load(&self.tx, id)?.filter(|r| !is_deleted(r)))
    }

    /// Inserts a new row and returns its id. `data` may carry an `id` (for
    /// derived ids); otherwise a UUIDv7 is generated.
    pub fn insert(&mut self, table: &str, data: Value) -> Result<String> {
        let def = writable(table, false)?;
        let mut fields = data_fields(def, data, true)?;
        let id = match fields.remove("id") {
            None | Some(Value::Null) => self.new_id(),
            Some(Value::String(s)) if !s.is_empty() => s,
            Some(_) => return Err(bad(def, "id", "expected a non-empty string")),
        };
        if def.load(&self.tx, &id)?.is_some() {
            return Err(Error::AlreadyExists {
                table: def.name.into(),
                id,
            });
        }
        let now = self.clock.now_ms();
        let mut row = Row::new();
        for col in def.cols {
            row.insert(
                col.name.into(),
                fields.remove(col.name).unwrap_or(Value::Null),
            );
        }
        row.insert("id".into(), Value::from(id.clone()));
        row.insert("_deleted".into(), Value::from(0));
        row.insert("created_at".into(), Value::from(now));
        row.insert("created_by".into(), self.actor.value());
        let hlc = self.stamp(&mut row)?;

        let audit: Row = def
            .cols
            .iter()
            .filter(|c| !row[c.name].is_null())
            .map(|c| (c.name.to_string(), redact(c.name, &row[c.name])))
            .collect();
        self.put(def, &row)?;
        self.audit(hlc, "insert", def, &id, serde_json::to_string(&audit)?)?;
        Ok(id)
    }

    /// Changes the given columns. Returns false if nothing actually changed,
    /// in which case nothing is written.
    pub fn update(&mut self, table: &str, id: &str, patch: Value) -> Result<bool> {
        let def = writable(table, true)?;
        let patch = data_fields(def, patch, false)?;
        let mut row = self.live(def, id)?;
        let mut diff = Row::new();
        for (col, new) in patch {
            let old = row.get(&col).cloned().unwrap_or(Value::Null);
            if old != new {
                diff.insert(
                    col.clone(),
                    Value::Array(vec![redact(&col, &old), redact(&col, &new)]),
                );
                row.insert(col, new);
            }
        }
        if diff.is_empty() {
            return Ok(false);
        }
        let hlc = self.stamp(&mut row)?;
        self.put(def, &row)?;
        self.audit(hlc, "update", def, id, serde_json::to_string(&diff)?)?;
        Ok(true)
    }

    /// Inserts with the given id, or updates the live row with that id.
    /// Meant for derived ids. Such rows can't come back once deleted, because
    /// deletes always win the merge, so clear them instead of deleting.
    pub fn upsert(&mut self, table: &str, id: &str, data: Value) -> Result<()> {
        let def = registry::table(table)?;
        match def.load(&self.tx, id)? {
            None => {
                let mut data = data;
                if let Value::Object(map) = &mut data {
                    map.insert("id".into(), Value::from(id));
                }
                self.insert(table, data).map(drop)
            }
            Some(row) if is_deleted(&row) => Err(Error::Deleted {
                table: def.name.into(),
                id: id.into(),
            }),
            Some(_) => self.update(table, id, data).map(drop),
        }
    }

    /// Marks a row deleted. The tombstone replicates and always wins.
    pub fn delete(&mut self, table: &str, id: &str) -> Result<()> {
        let def = writable(table, false)?;
        let mut row = self.live(def, id)?;
        row.insert("_deleted".into(), Value::from(1));
        let hlc = self.stamp(&mut row)?;
        self.put(def, &row)?;
        self.audit(hlc, "delete", def, id, "{}".into())?;
        Ok(())
    }

    fn live(&self, def: &TableDef, id: &str) -> Result<Row> {
        match def.load(&self.tx, id)? {
            Some(row) if !is_deleted(&row) => Ok(row),
            Some(_) => Err(Error::Deleted {
                table: def.name.into(),
                id: id.into(),
            }),
            None => Err(Error::NotFound {
                table: def.name.into(),
                id: id.into(),
            }),
        }
    }

    /// Sets HLC, site, version vector and `updated_by` for a local edit.
    fn stamp(&mut self, row: &mut Row) -> Result<Hlc> {
        let hlc = self.hlc.tick(self.clock.now_ms());
        let mut vv: VersionVector = match row.get("_vv") {
            Some(Value::String(s)) => serde_json::from_str(s)?,
            _ => VersionVector::new(),
        };
        vv.insert(self.site.to_string(), hlc);
        row.insert("_hlc".into(), Value::from(hlc.0));
        row.insert("_site".into(), Value::from(self.site));
        row.insert("_vv".into(), Value::from(serde_json::to_string(&vv)?));
        row.insert("updated_by".into(), self.actor.value());
        Ok(hlc)
    }

    /// Stores a complete row and gives it the next version number.
    fn put(&mut self, def: &'static TableDef, row: &Row) -> Result<()> {
        def.store(&self.tx, row)?;
        *self.ver += 1;
        versions::bump(&self.tx, def.name, str_col(row, "id"), *self.ver)?;
        self.changed.insert(def.name);
        Ok(())
    }

    fn audit(
        &mut self,
        hlc: Hlc,
        action: &str,
        def: &TableDef,
        row_id: &str,
        changes: String,
    ) -> Result<()> {
        let log = registry::table(AUDIT_LOG)?;
        let id = self.new_id();
        let now = self.clock.now_ms();
        let vv = VersionVector::from([(self.site.to_string(), hlc)]);
        let row = Row::from([
            ("id".into(), Value::from(id)),
            ("_hlc".into(), Value::from(hlc.0)),
            ("_site".into(), Value::from(self.site)),
            ("_vv".into(), Value::from(serde_json::to_string(&vv)?)),
            ("_deleted".into(), Value::from(0)),
            ("created_at".into(), Value::from(now)),
            ("created_by".into(), self.actor.value()),
            ("updated_by".into(), self.actor.value()),
            ("at".into(), Value::from(now)),
            ("user_id".into(), self.actor.value()),
            ("action".into(), Value::from(action)),
            ("tbl".into(), Value::from(def.name)),
            ("row_id".into(), Value::from(row_id)),
            ("changes_json".into(), Value::from(changes)),
        ]);
        self.put(log, &row)
    }

    fn apply_batch(&mut self, batch: &ChangeBatch) -> Result<ApplyReport> {
        if batch.schema != SCHEMA_VERSION {
            return Err(Error::SchemaMismatch {
                ours: SCHEMA_VERSION,
                theirs: batch.schema,
            });
        }
        if batch.site == self.site {
            return Err(Error::OwnSite);
        }
        if let Some((epoch, ver)) = versions::watermark(&self.tx, &batch.site)? {
            if epoch != batch.epoch {
                return Err(Error::EpochMismatch {
                    site: batch.site.clone(),
                });
            }
            if batch.head < ver {
                return Err(Error::PeerRewound {
                    site: batch.site.clone(),
                });
            }
        }

        let now = self.clock.now_ms();
        let mut report = ApplyReport::default();
        for wire in &batch.rows {
            let def = registry::table(&wire.t)?;
            let incoming = normalize(def, &wire.row)?;
            let inc = RowMeta::parse(&incoming)?;
            self.hlc
                .observe(inc.hlc, now)
                .map_err(|ahead_ms| Error::ClockSkew { ahead_ms })?;

            let Some(local) = def.load(&self.tx, str_col(&incoming, "id"))? else {
                self.put(def, &incoming)?;
                report.accepted += 1;
                continue;
            };
            let loc = RowMeta::parse(&local)?;
            let decision = merge::decide(&loc.version(), &inc.version());
            if decision.take_incoming {
                self.put(def, &incoming)?;
                report.accepted += 1;
            } else {
                report.ignored += 1;
            }
            if let Some(side) = decision.conflict {
                let (lost, meta) = match side {
                    Loser::Local => (&local, &loc),
                    Loser::Incoming => (&incoming, &inc),
                };
                if def.name != CONFLICTS
                    && data_differs(def, &local, &incoming)
                    && self.record_conflict(def, lost, meta)?
                {
                    report.conflicts += 1;
                }
            }
        }
        versions::set_watermark(&self.tx, &batch.site, &batch.epoch, batch.upto, now)?;
        Ok(report)
    }

    /// Saves a losing concurrent version. The conflict row's id, HLC and
    /// content come only from the lost version, so every PC that detects the
    /// same conflict writes an identical row, and a resolution (a later edit)
    /// always wins over a late re-detection.
    fn record_conflict(&mut self, def: &TableDef, lost: &Row, meta: &RowMeta) -> Result<bool> {
        let conflicts = registry::table(CONFLICTS)?;
        let row_id = str_col(lost, "id");
        let id = ids::derived_id(
            &format!("conflict/{}/{row_id}", def.name),
            &format!("{}/{}", meta.hlc.0, meta.site),
        );
        if conflicts.load(&self.tx, &id)?.is_some() {
            return Ok(false);
        }
        let vv = VersionVector::from([(meta.site.clone(), meta.hlc)]);
        let author = lost.get("updated_by").cloned().unwrap_or(Value::Null);
        let row = Row::from([
            ("id".into(), Value::from(id)),
            ("_hlc".into(), Value::from(meta.hlc.0)),
            ("_site".into(), Value::from(meta.site.clone())),
            ("_vv".into(), Value::from(serde_json::to_string(&vv)?)),
            ("_deleted".into(), Value::from(0)),
            ("created_at".into(), Value::from(meta.hlc.millis())),
            ("created_by".into(), author.clone()),
            ("updated_by".into(), author),
            ("tbl".into(), Value::from(def.name)),
            ("row_id".into(), Value::from(row_id)),
            (
                "lost_json".into(),
                Value::from(serde_json::to_string(lost)?),
            ),
            ("resolved".into(), Value::from(0)),
            ("resolved_by".into(), Value::Null),
        ]);
        self.put(conflicts, &row)?;
        Ok(true)
    }

    fn commit(self) -> Result<BTreeSet<&'static str>> {
        self.tx.execute(
            "UPDATE meta SET last_hlc = ?1, ver = ?2 WHERE id = 1",
            (self.hlc.last().0, *self.ver),
        )?;
        self.tx.commit()?;
        Ok(self.changed)
    }
}

/// Parsed meta columns of a full row.
#[derive(Debug, Clone)]
pub(crate) struct RowMeta {
    pub hlc: Hlc,
    pub site: String,
    pub deleted: bool,
    pub vv: VersionVector,
}

impl RowMeta {
    pub fn parse(row: &Row) -> Result<Self> {
        let malformed = |what: &str| Error::Malformed(format!("{what} in row {:?}", row.get("id")));
        let hlc = Hlc(row
            .get("_hlc")
            .and_then(Value::as_i64)
            .ok_or_else(|| malformed("missing _hlc"))?);
        let site = row
            .get("_site")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| malformed("missing _site"))?
            .to_string();
        let vv: VersionVector = row
            .get("_vv")
            .and_then(Value::as_str)
            .map(serde_json::from_str)
            .transpose()?
            .ok_or_else(|| malformed("missing _vv"))?;
        // The last writer's entry is the row's own HLC and the newest one.
        if vv.get(&site) != Some(&hlc) || vv.values().any(|h| *h > hlc) {
            return Err(malformed("inconsistent _vv"));
        }
        Ok(Self {
            hlc,
            site,
            deleted: is_deleted(row),
            vv,
        })
    }

    pub fn version(&self) -> Version<'_> {
        Version {
            hlc: self.hlc,
            site: &self.site,
            deleted: self.deleted,
            vv: &self.vv,
        }
    }
}

fn writable(table: &str, allow_conflicts: bool) -> Result<&'static TableDef> {
    let def = registry::table(table)?;
    if def.name == AUDIT_LOG || (def.name == CONFLICTS && !allow_conflicts) {
        return Err(Error::ReadOnlyTable(def.name.into()));
    }
    Ok(def)
}

/// Validates caller-supplied data columns. Meta columns are rejected, except
/// `id` when `allow_id` is set.
fn data_fields(def: &TableDef, data: Value, allow_id: bool) -> Result<Row> {
    let Value::Object(map) = data else {
        return Err(bad(def, "*", "expected an object"));
    };
    let mut out = Row::new();
    for (col, v) in map {
        if registry::is_meta(&col) && !(allow_id && col == "id") {
            return Err(bad(def, &col, "managed by the write path"));
        }
        let v = def.check_value(&col, v)?;
        out.insert(col, v);
    }
    Ok(out)
}

/// Validates a row from a peer and fills missing columns with null.
fn normalize(def: &TableDef, row: &Row) -> Result<Row> {
    let mut out = Row::new();
    for (col, v) in row {
        out.insert(col.clone(), def.check_value(col, v.clone())?);
    }
    for col in def.all_cols() {
        out.entry(col.name.to_string()).or_insert(Value::Null);
    }
    if !matches!(out.get("id"), Some(Value::String(s)) if !s.is_empty()) {
        return Err(Error::Malformed(format!("{} row without id", def.name)));
    }
    if out["created_at"].is_null() {
        return Err(Error::Malformed(format!(
            "{} row without created_at",
            def.name
        )));
    }
    if out["_deleted"].is_null() {
        out.insert("_deleted".into(), Value::from(0));
    }
    Ok(out)
}

fn data_differs(def: &TableDef, a: &Row, b: &Row) -> bool {
    is_deleted(a) != is_deleted(b) || def.cols.iter().any(|c| a.get(c.name) != b.get(c.name))
}

fn is_deleted(row: &Row) -> bool {
    row.get("_deleted").and_then(Value::as_i64).unwrap_or(0) != 0
}

fn str_col<'r>(row: &'r Row, col: &str) -> &'r str {
    row.get(col).and_then(Value::as_str).unwrap_or_default()
}

fn redact(col: &str, v: &Value) -> Value {
    if REDACTED.contains(&col) && !v.is_null() {
        Value::from("***")
    } else {
        v.clone()
    }
}

fn bad(def: &TableDef, column: &str, reason: &str) -> Error {
    Error::BadValue {
        table: def.name.into(),
        column: column.into(),
        reason: reason.into(),
    }
}
