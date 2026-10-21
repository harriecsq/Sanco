//! Multi-peer convergence simulation.
//!
//! N peers with skewed clocks make random edits, sync over a randomly
//! partitioned network, restart, and (in the restore variant) roll back to old
//! backups. Afterwards the network heals and every pair syncs until nothing
//! changes. Checks:
//! - every peer holds exactly the same rows, meta columns included;
//! - quiescence: a final full round accepts nothing;
//! - no edit vanishes silently: each version ever written is the final row,
//!   was seen by a later surviving version, or is recorded in `conflicts`;
//! - document numbers are unique once clashes are resolved (after restores,
//!   extra holders may remain only if no peer owns their site any more).
//!
//! `SANCO_SIM_SEED=<n>` runs a single seed (handy for reproducing a failure).

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use common::{T0, opts, pull};
use sanco_core::ids::derived_id;
use sanco_core::rng::Rng64;
use sanco_core::sync::hlc::ManualClock;
use sanco_core::sync::merge::{VersionVector, dominates};
use sanco_core::sync::numbering::{self, ClaimReply, NumberClaim, NumberPeer};
use sanco_core::{Actor, Db, Error, OpenOptions, Result, Row};
use serde_json::{Value, json};

const PEERS: usize = 4;
const STEPS: usize = 1500;
const MAX_SKEW_MS: u64 = 40_000;
const SETTING_KEYS: u64 = 3;
const SERIES: [&str; 2] = [numbering::series::DR, numbering::series::CLIENT];

struct Peer {
    name: String,
    path: PathBuf,
    clock: Arc<ManualClock>,
    skew: i64,
    db: Option<Db>,
    backup: Option<PathBuf>,
}

impl Peer {
    fn db(&self) -> &Db {
        self.db.as_ref().expect("peer is open")
    }
}

/// Another peer as seen through the (possibly partitioned) network.
struct Link<'a> {
    db: &'a Db,
    up: bool,
}

impl NumberPeer for Link<'_> {
    fn claim(&self, claim: &NumberClaim) -> Result<ClaimReply> {
        if self.up {
            self.db.claim_number(claim.clone())
        } else {
            Err(Error::PeerUnreachable(self.db.site_id().to_string()))
        }
    }
}

/// A version written by some peer, for the lost-edit check.
struct Written {
    tbl: &'static str,
    id: String,
    vv: VersionVector,
    deleted: bool,
}

#[derive(Default, Debug)]
struct Stats {
    writes: usize,
    syncs: usize,
    restarts: usize,
    restores: usize,
    conflicts: usize,
    numbered: usize,
    renumbered: usize,
    rows: usize,
}

struct Sim {
    seed: u64,
    rng: Rng64,
    now: i64,
    peers: Vec<Peer>,
    linked: Vec<Vec<bool>>,
    todo_ids: Vec<String>,
    written: Vec<Written>,
    counter: u64,
    with_restores: bool,
    stats: Stats,
    _dir: tempfile::TempDir,
}

impl Sim {
    fn new(seed: u64, with_restores: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = Rng64::seeded(seed);
        let peers = (0..PEERS)
            .map(|i| {
                let name = format!("pc{i}");
                let path = dir.path().join(&name).join("sanco.db");
                let skew = rng.below(2 * MAX_SKEW_MS) as i64 - MAX_SKEW_MS as i64;
                let clock = Arc::new(ManualClock::new(T0 + skew));
                let db = Db::open(opts(&path, &name, &clock, seed * 100 + i as u64)).unwrap();
                Peer {
                    name,
                    path,
                    clock,
                    skew,
                    db: Some(db),
                    backup: None,
                }
            })
            .collect();
        Self {
            seed,
            rng,
            now: T0,
            peers,
            linked: vec![vec![true; PEERS]; PEERS],
            todo_ids: Vec::new(),
            written: Vec::new(),
            counter: 0,
            with_restores,
            stats: Stats::default(),
            _dir: dir,
        }
    }

    fn pick_peer(&mut self) -> usize {
        self.rng.below(PEERS as u64) as usize
    }

    fn unique(&mut self, p: usize) -> String {
        self.counter += 1;
        format!("{}-{}", self.peers[p].name, self.counter)
    }

    fn run(mut self) -> Stats {
        for _ in 0..STEPS {
            self.now += self.rng.below(1500) as i64;
            for p in &self.peers {
                p.clock.set(self.now + p.skew);
            }
            match self.rng.below(100) {
                0..40 => {
                    let p = self.pick_peer();
                    self.local_write(p);
                }
                40..80 => self.sync_once(),
                80..86 => {
                    let (a, b) = (self.pick_peer(), self.pick_peer());
                    if a != b {
                        let v = !self.linked[a][b];
                        self.linked[a][b] = v;
                        self.linked[b][a] = v;
                    }
                }
                86..88 => {
                    let p = self.pick_peer();
                    self.restart(p, false);
                }
                88..91 if self.with_restores => {
                    let p = self.pick_peer();
                    let dest = self.peers[p]
                        .path
                        .with_file_name(format!("backup-{}.db", self.counter));
                    self.counter += 1;
                    self.peers[p].db().backup_to(&dest).unwrap();
                    self.peers[p].backup = Some(dest);
                }
                91..93 if self.with_restores => {
                    let p = self.pick_peer();
                    if self.peers[p].backup.is_some() {
                        self.restart(p, true);
                    }
                }
                93..98 => {
                    let p = self.pick_peer();
                    self.numbered_write(p);
                }
                98..100 => {
                    let p = self.pick_peer();
                    self.resolve_clashes(p);
                }
                _ => {}
            }
        }
        self.settle();
        self.stats
    }

    fn local_write(&mut self, p: usize) {
        let roll = self.rng.below(100);
        let value = self.unique(p);
        let actor = Actor::user(self.peers[p].name.clone());
        let db = self.peers[p].db();
        let written = if roll < 25 || self.todo_ids.is_empty() {
            let id = db
                .write(actor, move |tx| {
                    tx.insert(
                        "todos",
                        json!({ "title": value.clone(), "notes": value, "done": 0 }),
                    )
                })
                .unwrap();
            self.todo_ids.push(id.clone());
            Some(("todos", id))
        } else if roll < 80 {
            let id = self.todo_ids[self.rng.below(self.todo_ids.len() as u64) as usize].clone();
            let delete = roll >= 75;
            let done = self.rng.chance(50);
            let target = id.clone();
            let changed = db
                .write(actor, move |tx| {
                    if tx.get("todos", &target)?.is_none() {
                        return Ok(false);
                    }
                    if delete {
                        tx.delete("todos", &target)?;
                        return Ok(true);
                    }
                    tx.update("todos", &target, json!({ "notes": value, "done": done }))
                })
                .unwrap();
            changed.then_some(("todos", id))
        } else if roll < 95 {
            let key = format!("k{}", self.rng.below(SETTING_KEYS));
            let id = derived_id("setting", &key);
            let target = id.clone();
            db.write(actor, move |tx| {
                tx.upsert("settings", &target, json!({ "key": key, "value": value }))
            })
            .unwrap();
            Some(("settings", id))
        } else {
            let open: Vec<String> = db.dump().unwrap()["conflicts"]
                .iter()
                .filter(|c| c["resolved"] == json!(0))
                .map(|c| c["id"].as_str().unwrap().to_string())
                .collect();
            if let Some(cid) = open.first().cloned() {
                db.write(actor, move |tx| {
                    tx.update(
                        "conflicts",
                        &cid,
                        json!({ "resolved": 1, "resolved_by": value }),
                    )
                })
                .unwrap();
            }
            None
        };
        if let Some((tbl, id)) = written {
            let row = load(db, tbl, &id);
            self.written.push(Written {
                tbl,
                id,
                vv: vv_of(&row),
                deleted: row["_deleted"] == json!(1),
            });
            self.stats.writes += 1;
        }
    }

    /// The other peers, up or down as the network currently is for `p`.
    fn links(&self, p: usize) -> Vec<Link<'_>> {
        (0..PEERS)
            .filter(|&q| q != p)
            .map(|q| Link {
                db: self.peers[q].db(),
                up: self.linked[p][q],
            })
            .collect()
    }

    /// Creates a todo numbered like a document, as the app would.
    fn numbered_write(&mut self, p: usize) {
        let series = SERIES[self.rng.below(SERIES.len() as u64) as usize];
        let actor = Actor::user(self.peers[p].name.clone());
        let db = self.peers[p].db();
        let id = db.new_id().unwrap();
        let res = {
            let links = self.links(p);
            let peers: Vec<&dyn NumberPeer> = links.iter().map(|l| l as &dyn NumberPeer).collect();
            numbering::reserve(db, &peers, series, &id).unwrap()
        };
        let todo = id.clone();
        db.write(actor, move |tx| {
            tx.insert("todos", json!({ "id": todo, "title": res.no, "done": 0 }))?;
            numbering::assign(tx, &res, "todos")
        })
        .unwrap();
        let row = load(db, "todos", &id);
        self.written.push(Written {
            tbl: "todos",
            id: id.clone(),
            vv: vv_of(&row),
            deleted: false,
        });
        self.todo_ids.push(id);
        self.stats.writes += 1;
        self.stats.numbered += 1;
    }

    fn resolve_clashes(&mut self, p: usize) -> usize {
        let links = self.links(p);
        let peers: Vec<&dyn NumberPeer> = links.iter().map(|l| l as &dyn NumberPeer).collect();
        let n = numbering::resolve_clashes(self.peers[p].db(), &peers)
            .unwrap()
            .len();
        self.stats.renumbered += n;
        n
    }

    fn sync_once(&mut self) {
        let (a, b) = (self.pick_peer(), self.pick_peer());
        if a == b || !self.linked[a][b] {
            return;
        }
        let limit = 1 + self.rng.below(30) as usize;
        let (to, from) = (self.peers[a].db(), self.peers[b].db());
        if self.rng.chance(50) {
            pull(to, from, limit);
        } else {
            // A single, possibly partial batch.
            let since = to.watermark(from.site_id()).unwrap();
            let batch = from.changes_since(since, limit).unwrap();
            to.apply_batch(batch).unwrap();
        }
        self.stats.syncs += 1;
    }

    fn restart(&mut self, p: usize, restore: bool) {
        let seed = self.seed * 100 + p as u64 + self.counter * 7;
        let peer = &mut self.peers[p];
        let old_site = peer.db().site_id().to_string();
        drop(peer.db.take());
        let mut o: OpenOptions = opts(&peer.path, &peer.name, &peer.clock, seed);
        if restore {
            for ext in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", peer.path.display()));
            }
            std::fs::copy(peer.backup.as_ref().unwrap(), &peer.path).unwrap();
            o.new_identity = true;
            self.stats.restores += 1;
        } else {
            self.stats.restarts += 1;
        }
        let db = Db::open(o).unwrap();
        assert_eq!(old_site == db.site_id(), !restore, "identity after reopen");
        peer.db = Some(db);
    }

    fn settle(&mut self) {
        for row in &mut self.linked {
            row.fill(true);
        }
        self.sync_until_quiet();
        let mut rounds = 0;
        loop {
            rounds += 1;
            assert!(rounds < 10, "seed {}: clashes keep coming back", self.seed);
            let fixed: usize = (0..PEERS).map(|p| self.resolve_clashes(p)).sum();
            if fixed == 0 {
                break;
            }
            self.sync_until_quiet();
        }

        let reference = self.peers[0].db().dump().unwrap();
        for p in &self.peers[1..] {
            let dump = p.db().dump().unwrap();
            for (table, rows) in &reference {
                assert!(
                    rows == &dump[table],
                    "seed {}: table {table} differs between {} and {}",
                    self.seed,
                    self.peers[0].name,
                    p.name
                );
            }
        }
        self.stats.conflicts = reference["conflicts"].len();
        self.stats.rows = reference.values().map(Vec::len).sum();

        self.check_numbers(&reference);
        if !self.with_restores {
            self.check_no_silent_loss(&reference);
        }
    }

    fn sync_until_quiet(&mut self) {
        let mut rounds = 0;
        loop {
            rounds += 1;
            assert!(
                rounds < 20,
                "seed {}: no quiescence after 20 rounds",
                self.seed
            );
            let mut accepted = 0;
            for a in 0..PEERS {
                for b in 0..PEERS {
                    if a != b {
                        accepted += pull(self.peers[a].db(), self.peers[b].db(), 50);
                    }
                }
            }
            if accepted == 0 {
                break;
            }
        }
    }

    /// Every number has one holder. After restores, extra holders may remain
    /// only if they were assigned by a site no peer owns any more.
    fn check_numbers(&self, dump: &BTreeMap<String, Vec<Row>>) {
        let mut owned: Vec<String> = Vec::new();
        for p in &self.peers {
            owned.push(p.db().site_id().to_string());
            owned.extend(p.db().previous_sites().unwrap());
        }
        let str_of = |r: &Row, c: &str| r[c].as_str().unwrap().to_string();
        assert!(
            dump["conflicts"]
                .iter()
                .all(|c| c["tbl"] != json!("doc_numbers")),
            "seed {}: renumbering produced a concurrent edit",
            self.seed
        );
        let mut holders: BTreeMap<(String, String, i64), Vec<&Row>> = BTreeMap::new();
        let mut nos: BTreeMap<(String, String), usize> = BTreeMap::new();
        for r in dump["doc_numbers"]
            .iter()
            .filter(|r| r["_deleted"] == json!(0))
        {
            let key = (
                str_of(r, "series"),
                str_of(r, "period"),
                r["seq"].as_i64().unwrap(),
            );
            holders.entry(key).or_default().push(r);
            *nos.entry((str_of(r, "series"), str_of(r, "no")))
                .or_default() += 1;
            assert!(
                dump["todos"].iter().any(|t| t["id"] == r["ref_id"]),
                "seed {}: number for a missing document",
                self.seed
            );
        }
        for (key, mut rows) in holders {
            rows.sort_by_key(|r| (r["assigned_at"].as_i64(), str_of(r, "id")));
            for extra in &rows[1..] {
                let site = extra["site"].as_str().unwrap();
                assert!(
                    self.with_restores && !owned.iter().any(|s| s == site),
                    "seed {}: number {key:?} has {} holders",
                    self.seed,
                    rows.len()
                );
            }
        }
        if !self.with_restores {
            assert!(
                nos.values().all(|&n| n == 1),
                "seed {}: duplicate formatted number",
                self.seed
            );
        }
    }

    fn check_no_silent_loss(&self, dump: &BTreeMap<String, Vec<Row>>) {
        let mut finals: BTreeMap<(&str, &str), &Row> = BTreeMap::new();
        for tbl in ["todos", "settings"] {
            for row in &dump[tbl] {
                finals.insert((tbl, row["id"].as_str().unwrap()), row);
            }
        }
        let mut lost: BTreeMap<(String, String), Vec<VersionVector>> = BTreeMap::new();
        for c in &dump["conflicts"] {
            let row: Row = serde_json::from_str(c["lost_json"].as_str().unwrap()).unwrap();
            let key = (
                c["tbl"].as_str().unwrap().to_string(),
                c["row_id"].as_str().unwrap().to_string(),
            );
            lost.entry(key).or_default().push(vv_of(&row));
        }
        for w in &self.written {
            let f = finals
                .get(&(w.tbl, w.id.as_str()))
                .unwrap_or_else(|| panic!("seed {}: {} {} missing", self.seed, w.tbl, w.id));
            let final_deleted = f["_deleted"] == json!(1);
            if w.deleted {
                assert!(final_deleted, "seed {}: delete of {} lost", self.seed, w.id);
                continue;
            }
            let mut covers: Vec<VersionVector> = vec![vv_of(f)];
            if let Some(vs) = lost.get(&(w.tbl.to_string(), w.id.clone())) {
                covers.extend(vs.iter().cloned());
            }
            if final_deleted {
                // Knowingly deleted after this edit counts as seen.
                covers.extend(
                    self.written
                        .iter()
                        .filter(|o| o.deleted && o.tbl == w.tbl && o.id == w.id)
                        .map(|o| o.vv.clone()),
                );
            }
            assert!(
                covers.iter().any(|c| dominates(c, &w.vv)),
                "seed {}: edit {:?} of {} {} vanished without a conflict",
                self.seed,
                w.vv,
                w.tbl,
                w.id
            );
        }
    }
}

fn load(db: &Db, tbl: &str, id: &str) -> Row {
    let def = sanco_core::db::registry::table(tbl).unwrap();
    db.read(|c| def.load(c, id)).unwrap().unwrap()
}

fn vv_of(row: &Row) -> VersionVector {
    match &row["_vv"] {
        Value::String(s) => serde_json::from_str(s).unwrap(),
        other => panic!("bad _vv {other:?}"),
    }
}

fn seeds(default: std::ops::RangeInclusive<u64>) -> Vec<u64> {
    match std::env::var("SANCO_SIM_SEED") {
        Ok(s) => vec![s.parse().expect("SANCO_SIM_SEED must be a number")],
        Err(_) => default.collect(),
    }
}

#[test]
fn peers_converge_without_losing_edits() {
    let (mut conflicts, mut renumbered) = (0, 0);
    for seed in seeds(1..=6) {
        let stats = Sim::new(seed, false).run();
        eprintln!("seed {seed}: {stats:?}");
        assert!(stats.writes > 300 && stats.syncs > 150, "{stats:?}");
        conflicts += stats.conflicts;
        renumbered += stats.renumbered;
    }
    assert!(conflicts > 0, "simulation never produced a concurrent edit");
    assert!(renumbered > 0, "simulation never produced a number clash");
}

#[test]
fn peers_converge_after_restores() {
    for seed in seeds(101..=103) {
        let stats = Sim::new(seed, true).run();
        eprintln!("seed {seed}: {stats:?}");
        assert!(
            stats.restores > 0 || std::env::var("SANCO_SIM_SEED").is_ok(),
            "{stats:?}"
        );
    }
}
