mod common;

use std::sync::Arc;

use common::{T0, TestPeer, opts, pull};
use sanco_core::sync::hlc::ManualClock;
use sanco_core::sync::numbering::{self, NumberPeer, series};
use sanco_core::sync::peer::{DEVICE_REFRESH_MS, DeviceInfo};
use sanco_core::{Actor, Db, Error};
use serde_json::json;

fn info(name: &str, port: u16) -> DeviceInfo {
    DeviceInfo {
        name: name.into(),
        host: Some("100.64.0.1".into()),
        port,
        app_version: "0.1.0".into(),
    }
}

fn add_todo(db: &Db, title: &str) -> String {
    let title = title.to_string();
    db.write(Actor::user("u1"), move |tx| {
        tx.insert("todos", json!({ "title": title }))
    })
    .unwrap()
}

#[test]
fn workspace_is_created_once() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    assert_eq!(a.db.workspace().unwrap(), None);
    let ws = a.db.create_workspace("  Sanco ").unwrap();
    assert_eq!(ws.name, "Sanco");
    assert_eq!(ws.sync_key.len(), 64);
    assert_eq!(a.db.workspace().unwrap(), Some(ws));
    assert!(matches!(
        a.db.create_workspace("again"),
        Err(Error::WorkspaceExists)
    ));
}

#[test]
fn handshake_rejects_bad_peers() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    let hello = b.db.hello("PC-B", "0.1.0").unwrap();
    a.db.check_peer(&hello).unwrap();

    let own = a.db.hello("PC-A", "0.1.0").unwrap();
    assert!(matches!(a.db.check_peer(&own), Err(Error::OwnSite)));

    let mut h = hello.clone();
    h.schema += 1;
    assert!(matches!(
        a.db.check_peer(&h),
        Err(Error::SchemaMismatch { .. })
    ));

    for (skew, bad) in [(119_000, false), (121_000, true), (-121_000, true)] {
        let mut h = hello.clone();
        h.now_ms = T0 + skew;
        let res = a.db.check_peer(&h);
        assert_eq!(
            matches!(res, Err(Error::ClockSkew { .. })),
            bad,
            "{skew}: {res:?}"
        );
    }

    // Once a site is known, its epoch and head must not go backwards.
    add_todo(&b.db, "x");
    add_todo(&b.db, "y");
    pull(&a.db, &b.db, 100);
    let hello = b.db.hello("PC-B", "0.1.0").unwrap();
    a.db.check_peer(&hello).unwrap();
    let mut h = hello.clone();
    h.epoch = "other".into();
    assert!(matches!(
        a.db.check_peer(&h),
        Err(Error::EpochMismatch { .. })
    ));
    let mut h = hello;
    h.head -= 1;
    assert!(matches!(
        a.db.check_peer(&h),
        Err(Error::PeerRewound { .. })
    ));
}

#[test]
fn joining_from_a_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let ws = a.db.create_workspace("Sanco").unwrap();
    a.db.register_device(&info("PC-A", 7470)).unwrap();
    let first = add_todo(&a.db, "before join");
    // A pending claim on the sender must not travel with the copy.
    let claim_ref = a.db.new_id().unwrap();
    numbering::reserve(&a.db, &[], series::DR, &claim_ref).unwrap();

    let path = dir.path().join("b").join("sanco.db");
    let snap = a.db.snapshot(&path).unwrap();
    assert_eq!(snap.site, a.db.site_id());
    assert_eq!(snap.head, a.db.head().unwrap());

    // Same machine, different path: the copy takes a new identity.
    let clock = Arc::new(ManualClock::new(T0));
    let b = Db::open(opts(&path, "a", &clock, 2)).unwrap();
    assert_ne!(b.site_id(), a.db.site_id());
    assert_ne!(b.epoch(), a.db.epoch());
    assert!(b.previous_sites().unwrap().is_empty());
    b.finish_join(&snap).unwrap();

    assert_eq!(b.workspace().unwrap(), Some(ws));
    assert_eq!(b.watermark(a.db.site_id()).unwrap(), snap.head);
    let claims: i64 = b
        .read(|c| Ok(c.query_row("SELECT count(*) FROM number_claims", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(claims, 0);
    assert_eq!(b.devices().unwrap().len(), 1);
    assert!(
        b.dump().unwrap()["todos"]
            .iter()
            .any(|r| r["id"] == json!(first))
    );

    // Nothing to pull from A until A changes again; then only the change.
    assert_eq!(pull(&b, &a.db, 100), 0);
    add_todo(&a.db, "after join");
    assert_eq!(pull(&b, &a.db, 100), 2); // the todo + its audit row

    // A learns about B from B's device row.
    b.register_device(&info("PC-B", 7471)).unwrap();
    pull(&a.db, &b, 100);
    let devices = a.db.devices().unwrap();
    assert_eq!(devices.len(), 2);
    let peer = devices.iter().find(|d| d.site == b.site_id()).unwrap();
    assert_eq!(peer.addr().as_deref(), Some("100.64.0.1:7471"));

    // A claim B makes reaches A like any peer claim would.
    let r = numbering::reserve(&b, &[&a.db as &dyn NumberPeer], series::DR, "doc").unwrap();
    assert!(r.confirmed());

    let mut own = snap;
    own.site = b.site_id().to_string();
    assert!(matches!(b.finish_join(&own), Err(Error::OwnSite)));
}

#[test]
fn device_row_is_written_only_when_needed() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    assert!(a.db.register_device(&info("PC-A", 7470)).unwrap());
    let head = a.db.head().unwrap();
    assert!(!a.db.register_device(&info("PC-A", 7470)).unwrap());
    assert_eq!(a.db.head().unwrap(), head);

    a.clock.advance(DEVICE_REFRESH_MS);
    assert!(a.db.register_device(&info("PC-A", 7470)).unwrap());
    assert!(a.db.register_device(&info("PC-A", 7480)).unwrap());
    let d = &a.db.devices().unwrap()[0];
    assert_eq!(d.site, a.db.site_id());
    assert_eq!(d.port, Some(7480));
    assert_eq!(d.last_seen, Some(T0 + DEVICE_REFRESH_MS));
}

#[test]
fn restored_device_replaces_its_old_row() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    a.db.register_device(&info("PC-A", 7470)).unwrap();
    let old_site = a.db.site_id().to_string();
    let backup = dir.path().join("backup.db");
    a.db.backup_to(&backup).unwrap();
    drop(a.db);

    std::fs::copy(&backup, &a.path).unwrap();
    let mut o = opts(&a.path, "a", &a.clock, 3);
    o.new_identity = true;
    let restored = Db::open(o).unwrap();
    assert_ne!(restored.site_id(), old_site);
    assert!(restored.register_device(&info("PC-A", 7470)).unwrap());
    let devices = restored.devices().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].site, restored.site_id());
}
