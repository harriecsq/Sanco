mod common;

use common::{TestPeer, opts, pull};
use sanco_core::backup;
use sanco_core::{Actor, Db, Error};
use serde_json::json;

const DAY: i64 = 24 * 60 * 60 * 1000;

fn todo(db: &Db, title: &str) -> String {
    let title = title.to_string();
    db.write(Actor::system(), move |tx| {
        tx.insert("todos", json!({ "title": title, "done": 0 }))
    })
    .unwrap()
}

fn count_todos(db: &Db) -> i64 {
    db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM todos", [], |r| r.get(0))?))
        .unwrap()
}

#[test]
fn daily_backups_are_made_once_and_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let bdir = backup::dir_for(&a.path);
    todo(&a.db, "x");

    assert!(backup::daily(&a.db, &bdir, 3).unwrap().is_some());
    assert!(backup::daily(&a.db, &bdir, 3).unwrap().is_none());
    for _ in 0..4 {
        a.clock.advance(DAY);
        assert!(backup::daily(&a.db, &bdir, 3).unwrap().is_some());
    }
    backup::manual(&a.db, &bdir).unwrap();

    let list = backup::list(&bdir).unwrap();
    let daily: Vec<_> = list.iter().filter(|b| b.kind == "daily").map(|b| b.name.as_str()).collect();
    assert_eq!(daily.len(), 3);
    assert!(daily.contains(&"sanco-daily-2026-10-25.db"));
    assert!(!daily.contains(&"sanco-daily-2026-10-22.db"));
    assert_eq!(list.iter().filter(|b| b.kind == "manual").count(), 1);
    assert!(list.iter().all(|b| b.size > 0));
}

#[test]
fn backup_names_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 2);
    let bdir = backup::dir_for(&a.path);
    let made = backup::manual(&a.db, &bdir).unwrap();
    let name = made.file_name().unwrap().to_str().unwrap();
    assert_eq!(backup::resolve(&bdir, name).unwrap(), made);
    for bad in ["", "..", "../sanco.db", "..\\sanco.db", "C:\\x.db", "missing.db", "notes.txt"] {
        assert!(backup::resolve(&bdir, bad).is_err(), "{bad}");
    }
}

#[test]
fn restore_takes_a_new_identity_and_resyncs() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = TestPeer::new(dir.path(), "a", 3);
    let b = TestPeer::new(dir.path(), "b", 4);
    let ws = a.db.create_workspace("Sanco").unwrap();
    let bdir = backup::dir_for(&a.path);

    todo(&a.db, "before backup");
    let saved = backup::manual(&a.db, &bdir).unwrap();
    todo(&a.db, "after backup, synced");
    pull(&b.db, &a.db, 100);
    todo(&a.db, "after backup, never synced");

    // Wrong workspace, or no workspace at all, is refused.
    let other = TestPeer::new(dir.path(), "other", 5);
    let other_saved = backup::manual(&other.db, &backup::dir_for(&other.path)).unwrap();
    assert!(matches!(
        backup::check_restorable(&other_saved, &ws.workspace_id),
        Err(Error::Invalid(_))
    ));
    other.db.create_workspace("Elsewhere").unwrap();
    let other_saved = backup::manual(&other.db, &backup::dir_for(&other.path)).unwrap();
    assert!(backup::check_restorable(&other_saved, &ws.workspace_id).is_err());
    backup::check_restorable(&saved, &ws.workspace_id).unwrap();

    let old_site = a.db.site_id().to_string();
    let before = backup::before_restore_path(&bdir, a.db.now_ms());
    a.db.backup_to(&before).unwrap();
    let TestPeer { path, machine, clock, db } = a;
    drop(db);
    backup::replace_db(&path, &saved).unwrap();
    let mut o = opts(&path, &machine, &clock, 33);
    o.new_identity = true;
    a = TestPeer { path, machine, clock, db: Db::open(o).unwrap() };

    assert_ne!(a.db.site_id(), old_site);
    assert_eq!(a.db.previous_sites().unwrap(), vec![old_site]);
    assert_eq!(count_todos(&a.db), 1);
    assert_eq!(a.db.workspace().unwrap().unwrap().workspace_id, ws.workspace_id);

    // B still has what was synced; the restored PC gets it back from B, and
    // B accepts the restored PC as a new site.
    pull(&a.db, &b.db, 100);
    assert_eq!(count_todos(&a.db), 2);
    todo(&a.db, "after restore");
    pull(&b.db, &a.db, 100);
    assert_eq!(count_todos(&b.db), 3);

    // The pre-restore copy still holds the unsynced edit.
    assert!(backup::list(&bdir).unwrap().iter().any(|b| b.kind == "before-restore"));
}
