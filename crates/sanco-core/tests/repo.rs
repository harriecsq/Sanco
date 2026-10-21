mod common;

use common::{T0, TestPeer, opts, pull};
use sanco_core::db::registry::{Kind, META_COLS, TABLES};
use sanco_core::{Actor, Db, Error};
use serde_json::{Value, json};

fn todo(db: &Db, title: &str) -> String {
    let title = title.to_string();
    db.write(Actor::user("u1"), move |tx| {
        tx.insert("todos", json!({ "title": title, "done": false }))
    })
    .unwrap()
}

fn get(db: &Db, table: &str, id: &str) -> Option<sanco_core::Row> {
    let def = sanco_core::db::registry::table(table).unwrap();
    db.read(|c| def.load(c, id)).unwrap()
}

fn live_conflicts(db: &Db) -> Vec<sanco_core::Row> {
    db.dump().unwrap()["conflicts"].clone()
}

#[test]
fn registry_matches_schema() {
    let dir = tempfile::tempdir().unwrap();
    let p = TestPeer::new(dir.path(), "a", 1);
    p.db.read(|c| {
        for t in TABLES {
            let mut stmt = c.prepare(&format!("PRAGMA table_info(\"{}\")", t.name))?;
            let cols: Vec<(String, String)> = stmt
                .query_map([], |r| Ok((r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?;
            let expected: Vec<(String, String)> = t
                .all_cols()
                .map(|c| {
                    let ty = match c.kind {
                        Kind::Text => "TEXT",
                        Kind::Int => "INTEGER",
                    };
                    (c.name.to_string(), ty.to_string())
                })
                .collect();
            assert_eq!(cols, expected, "table {}", t.name);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(META_COLS[0].name, "id");
}

#[test]
fn insert_update_delete_write_audit() {
    let dir = tempfile::tempdir().unwrap();
    let p = TestPeer::new(dir.path(), "a", 1);
    let id = todo(&p.db, "Call client");

    let row = get(&p.db, "todos", &id).unwrap();
    assert_eq!(row["title"], json!("Call client"));
    assert_eq!(row["done"], json!(0));
    assert_eq!(row["created_by"], json!("u1"));
    assert_eq!(row["_site"], json!(p.db.site_id()));

    p.clock.advance(1000);
    let changed = {
        let id = id.clone();
        p.db.write(Actor::user("u2"), move |tx| {
            tx.update("todos", &id, json!({ "done": true }))
        })
        .unwrap()
    };
    assert!(changed);
    let unchanged = {
        let id = id.clone();
        p.db.write(Actor::user("u2"), move |tx| {
            tx.update("todos", &id, json!({ "done": 1 }))
        })
        .unwrap()
    };
    assert!(!unchanged, "no-op update writes nothing");

    let row2 = get(&p.db, "todos", &id).unwrap();
    assert_eq!(row2["updated_by"], json!("u2"));
    assert_eq!(row2["created_by"], json!("u1"));
    assert!(row2["_hlc"].as_i64() > row["_hlc"].as_i64());

    {
        let id = id.clone();
        p.db.write(Actor::user("u2"), move |tx| tx.delete("todos", &id))
            .unwrap();
    }
    assert_eq!(get(&p.db, "todos", &id).unwrap()["_deleted"], json!(1));

    let audit = &p.db.dump().unwrap()["audit_log"];
    let actions: Vec<&str> = audit
        .iter()
        .map(|r| r["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["insert", "update", "delete"]);
    assert_eq!(audit[1]["changes_json"], json!(r#"{"done":[0,1]}"#));

    // Deleted rows can't be edited.
    let id2 = id.clone();
    let err =
        p.db.write(Actor::system(), move |tx| {
            tx.update("todos", &id2, json!({ "title": "x" }))
        })
        .unwrap_err();
    assert!(matches!(err, Error::Deleted { .. }));
}

#[test]
fn writes_are_validated() {
    let dir = tempfile::tempdir().unwrap();
    let p = TestPeer::new(dir.path(), "a", 1);
    let cases: Vec<(&str, Value)> = vec![
        ("todos", json!({ "nope": 1 })),
        ("todos", json!({ "done": "yes" })),
        ("todos", json!({ "title": 5 })),
        ("todos", json!({ "_hlc": 5 })),
        ("audit_log", json!({ "action": "x" })),
        ("conflicts", json!({ "tbl": "x" })),
        ("nope", json!({})),
    ];
    for (table, data) in cases {
        let table = table.to_string();
        let res =
            p.db.write(Actor::system(), move |tx| tx.insert(&table, data));
        assert!(res.is_err());
    }
    // A failed transaction leaves nothing behind.
    let res = p.db.write(Actor::system(), |tx| {
        tx.insert("todos", json!({ "title": "kept?" }))?;
        tx.insert("todos", json!({ "bad": 1 }))
    });
    assert!(res.is_err());
    assert!(p.db.dump().unwrap()["todos"].is_empty());
    assert!(p.db.dump().unwrap()["audit_log"].is_empty());
}

#[test]
fn passwords_are_redacted_in_audit() {
    let dir = tempfile::tempdir().unwrap();
    let p = TestPeer::new(dir.path(), "a", 1);
    p.db.write(Actor::system(), |tx| {
        tx.insert(
            "users",
            json!({ "username": "ana", "pw_hash": "$argon2id$secret" }),
        )
    })
    .unwrap();
    let audit = &p.db.dump().unwrap()["audit_log"][0];
    let changes = audit["changes_json"].as_str().unwrap();
    assert!(!changes.contains("secret"), "{changes}");
}

#[test]
fn changes_replicate_and_echoes_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    for i in 0..5 {
        todo(&a.db, &format!("t{i}"));
    }
    // 5 todos + 5 audit rows, pulled two at a time.
    assert_eq!(pull(&b.db, &a.db, 2), 10);
    assert_eq!(pull(&b.db, &a.db, 2), 0);
    // B relays A's rows back to A: all echoes.
    assert_eq!(pull(&a.db, &b.db, 3), 0);
    assert_eq!(a.db.dump().unwrap(), b.db.dump().unwrap());
}

#[test]
fn concurrent_edits_converge_and_record_one_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    let c = TestPeer::new(dir.path(), "c", 3);
    let id = todo(&a.db, "Follow up");
    pull(&b.db, &a.db, 100);
    pull(&c.db, &a.db, 100);

    // A and B edit while apart; B's edit is later and wins.
    a.clock.advance(1000);
    b.clock.advance(2000);
    for (p, notes) in [(&a, "from a"), (&b, "from b")] {
        let id = id.clone();
        p.db.write(Actor::user(notes), move |tx| {
            tx.update("todos", &id, json!({ "notes": notes }))
        })
        .unwrap();
    }
    // Both sides detect the conflict independently...
    pull(&a.db, &b.db, 100);
    pull(&b.db, &a.db, 100);
    // ...and C hears about it only via relays.
    pull(&c.db, &a.db, 100);
    pull(&c.db, &b.db, 100);
    for _ in 0..2 {
        for (x, y) in [(&a, &b), (&a, &c), (&b, &a), (&b, &c), (&c, &a), (&c, &b)] {
            pull(&x.db, &y.db, 100);
        }
    }

    let da = a.db.dump().unwrap();
    assert_eq!(da, b.db.dump().unwrap());
    assert_eq!(da, c.db.dump().unwrap());
    let row = get(&a.db, "todos", &id).unwrap();
    assert_eq!(row["notes"], json!("from b"));
    let conflicts = live_conflicts(&a.db);
    assert_eq!(conflicts.len(), 1);
    let lost: Value = serde_json::from_str(conflicts[0]["lost_json"].as_str().unwrap()).unwrap();
    assert_eq!(lost["notes"], json!("from a"));
    assert_eq!(conflicts[0]["row_id"], json!(id));

    // Resolving replicates and isn't undone by late re-detection.
    let cid = conflicts[0]["id"].as_str().unwrap().to_string();
    a.db.write(Actor::user("admin"), move |tx| {
        tx.update(
            "conflicts",
            &cid,
            json!({ "resolved": 1, "resolved_by": "admin" }),
        )
    })
    .unwrap();
    pull(&b.db, &a.db, 100);
    pull(&c.db, &b.db, 100);
    assert_eq!(live_conflicts(&c.db)[0]["resolved"], json!(1));
}

#[test]
fn sequential_edits_through_a_relay_are_not_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    let c = TestPeer::new(dir.path(), "c", 3);
    let id = todo(&a.db, "v1");
    pull(&b.db, &a.db, 100);
    pull(&c.db, &a.db, 100);
    // B edits twice; C only ever sees the final state.
    for v in ["v2", "v3"] {
        b.clock.advance(100);
        let id = id.clone();
        b.db.write(Actor::system(), move |tx| {
            tx.update("todos", &id, json!({ "title": v }))
        })
        .unwrap();
    }
    pull(&a.db, &b.db, 100);
    a.clock.advance(500);
    {
        let id = id.clone();
        a.db.write(Actor::system(), move |tx| {
            tx.update("todos", &id, json!({ "title": "v4" }))
        })
        .unwrap();
    }
    pull(&c.db, &a.db, 100);
    pull(&c.db, &b.db, 100);
    assert_eq!(get(&c.db, "todos", &id).unwrap()["title"], json!("v4"));
    assert!(live_conflicts(&c.db).is_empty());
}

#[test]
fn delete_wins_over_concurrent_edit() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    let id = todo(&a.db, "x");
    pull(&b.db, &a.db, 100);
    {
        let id = id.clone();
        a.db.write(Actor::system(), move |tx| tx.delete("todos", &id))
            .unwrap();
    }
    b.clock.advance(5000);
    {
        let id = id.clone();
        b.db.write(Actor::system(), move |tx| {
            tx.update("todos", &id, json!({ "title": "later edit" }))
        })
        .unwrap();
    }
    pull(&a.db, &b.db, 100);
    pull(&b.db, &a.db, 100);
    for p in [&a, &b] {
        assert_eq!(get(&p.db, "todos", &id).unwrap()["_deleted"], json!(1));
        assert_eq!(live_conflicts(&p.db).len(), 1);
    }
}

#[test]
fn identical_concurrent_derived_rows_are_not_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    let id = sanco_core::ids::derived_id("setting", "vat_rate");
    for (p, at) in [(&a, 10), (&b, 20)] {
        p.clock.advance(at);
        let id = id.clone();
        p.db.write(Actor::system(), move |tx| {
            tx.upsert(
                "settings",
                &id,
                json!({ "key": "vat_rate", "value": "0.12" }),
            )
        })
        .unwrap();
    }
    pull(&a.db, &b.db, 100);
    pull(&b.db, &a.db, 100);
    assert!(live_conflicts(&a.db).is_empty());
    assert_eq!(a.db.dump().unwrap(), b.db.dump().unwrap());
}

#[test]
fn skewed_clock_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    a.clock.set(T0 + 3 * 60 * 1000);
    todo(&a.db, "from the future");
    let batch = a.db.changes_since(0, 100).unwrap();
    let err = b.db.apply_batch(batch).unwrap_err();
    assert!(matches!(err, Error::ClockSkew { .. }), "{err}");
    // Nothing applied, watermark unchanged.
    assert_eq!(b.db.watermark(a.db.site_id()).unwrap(), 0);
    assert!(b.db.dump().unwrap()["todos"].is_empty());
}

#[test]
fn own_batch_and_schema_mismatch_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    todo(&a.db, "x");
    let own = a.db.changes_since(0, 100).unwrap();
    assert!(matches!(a.db.apply_batch(own.clone()), Err(Error::OwnSite)));
    let mut newer = own;
    newer.schema += 1;
    assert!(matches!(
        b.db.apply_batch(newer),
        Err(Error::SchemaMismatch { .. })
    ));
}

#[test]
fn restart_keeps_identity_and_clock() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    a.clock.set(T0 + 10_000);
    let id = todo(&a.db, "x");
    let site = a.db.site_id().to_string();
    let hlc = get(&a.db, "todos", &id).unwrap()["_hlc"].as_i64().unwrap();
    let TestPeer {
        path, clock, db, ..
    } = a;
    drop(db);

    // Clock went backwards across the restart: HLC must not.
    clock.set(T0);
    let db = Db::open(opts(&path, "a", &clock, 1)).unwrap();
    assert_eq!(db.site_id(), site);
    let id2 = db
        .write(Actor::system(), |tx| {
            tx.insert("todos", json!({ "title": "y" }))
        })
        .unwrap();
    let hlc2 = get(&db, "todos", &id2).unwrap()["_hlc"].as_i64().unwrap();
    assert!(hlc2 > hlc);
}

#[test]
fn copied_database_takes_new_identity() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    todo(&a.db, "x");
    let copy = dir.path().join("copy").join("sanco.db");
    a.db.backup_to(&copy).unwrap();
    let other = Db::open(opts(&copy, "laptop", &a.clock, 9)).unwrap();
    assert_ne!(other.site_id(), a.db.site_id());
    assert_eq!(other.dump().unwrap(), a.db.dump().unwrap());
}

#[test]
fn rewound_peer_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    todo(&a.db, "x");
    let backup = dir.path().join("a-backup.db");
    a.db.backup_to(&backup).unwrap();
    todo(&a.db, "y");
    pull(&b.db, &a.db, 100);

    // Restore the file in place behind the app's back (same path, same PC).
    let TestPeer {
        path, clock, db, ..
    } = a;
    drop(db);
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
    }
    std::fs::copy(&backup, &path).unwrap();
    let a = Db::open(opts(&path, "a", &clock, 1)).unwrap();

    let batch = a
        .changes_since(b.db.watermark(a.site_id()).unwrap(), 100)
        .unwrap();
    assert!(matches!(
        b.db.apply_batch(batch),
        Err(Error::PeerRewound { .. })
    ));
    drop(a);

    // The app's restore flow takes a new identity, which syncs fine.
    let mut o = opts(&path, "a", &clock, 1);
    o.new_identity = true;
    let a = Db::open(o).unwrap();
    pull(&b.db, &a, 100);
    pull(&a, &b.db, 100);
    assert_eq!(a.dump().unwrap(), b.db.dump().unwrap());
    assert_eq!(a.dump().unwrap()["todos"].len(), 2);
}

#[test]
fn change_listener_reports_origin() {
    use sanco_core::{ChangeEvent, Origin};
    use std::sync::{Arc, Mutex};

    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let events: Arc<Mutex<Vec<ChangeEvent>>> = Arc::default();
    let path = dir.path().join("b").join("sanco.db");
    let mut o = opts(&path, "b", &a.clock, 2);
    let sink = events.clone();
    o.on_change = Some(Arc::new(move |e: &ChangeEvent| {
        sink.lock().unwrap().push(e.clone())
    }));
    let b = Db::open(o).unwrap();

    todo(&b, "local");
    todo(&a.db, "remote");
    pull(&b, &a.db, 100);
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].origin, Origin::Local);
    assert_eq!(events[1].origin, Origin::Remote);
    assert_eq!(events[1].tables, ["audit_log", "todos"]);
}
