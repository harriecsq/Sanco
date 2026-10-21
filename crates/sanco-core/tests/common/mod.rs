#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sanco_core::sync::hlc::ManualClock;
use sanco_core::{Db, OpenOptions};

/// 2026-10-21 00:00 UTC.
pub const T0: i64 = 1_792_540_800_000;

pub fn opts(path: &Path, machine: &str, clock: &Arc<ManualClock>, seed: u64) -> OpenOptions {
    let mut o = OpenOptions::new(path, machine);
    o.clock = clock.clone();
    o.seed = Some(seed);
    o.durable = false;
    o
}

pub struct TestPeer {
    pub path: PathBuf,
    pub machine: String,
    pub clock: Arc<ManualClock>,
    pub db: Db,
}

impl TestPeer {
    pub fn new(dir: &Path, name: &str, seed: u64) -> Self {
        let path = dir.join(name).join("sanco.db");
        let clock = Arc::new(ManualClock::new(T0));
        let db = Db::open(opts(&path, name, &clock, seed)).unwrap();
        Self {
            path,
            machine: name.to_string(),
            clock,
            db,
        }
    }
}

/// `to` pulls everything from `from`. Returns rows accepted.
pub fn pull(to: &Db, from: &Db, limit: usize) -> usize {
    let mut accepted = 0;
    loop {
        let since = to.watermark(from.site_id()).unwrap();
        let batch = from.changes_since(since, limit).unwrap();
        let more = batch.more;
        accepted += to.apply_batch(batch).unwrap().accepted;
        if !more {
            return accepted;
        }
    }
}
