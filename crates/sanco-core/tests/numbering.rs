mod common;

use common::{T0, TestPeer, opts, pull};
use sanco_core::ids::derived_id;
use sanco_core::sync::numbering::{
    self, CLAIM_TTL_MS, ClaimReply, NumberClaim, NumberPeer, Reservation, format_key, series,
};
use sanco_core::{Actor, Db, Error, Result};
use serde_json::json;

/// A peer that can't be reached.
struct Down;

impl NumberPeer for Down {
    fn claim(&self, _: &NumberClaim) -> Result<ClaimReply> {
        Err(Error::PeerUnreachable("down".into()))
    }
}

/// Creates a todo numbered in `series`, like a document would be.
fn create(db: &Db, peers: &[&dyn NumberPeer], series: &str, user: &str) -> (String, Reservation) {
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, peers, series, &id).unwrap();
    let r = res.clone();
    let todo = id.clone();
    db.write(Actor::user(user), move |tx| {
        tx.insert("todos", json!({ "id": todo, "title": "doc" }))?;
        numbering::assign(tx, &r, "todos")
    })
    .unwrap();
    (id, res)
}

fn no_of(db: &Db, series: &str, id: &str) -> String {
    db.read(|c| numbering::number_of(c, series, id))
        .unwrap()
        .unwrap()
}

fn sync(a: &Db, b: &Db) {
    pull(a, b, 100);
    pull(b, a, 100);
}

#[test]
fn numbers_count_up_per_series() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let (d1, r1) = create(&a.db, &[], series::DR, "u1");
    let (_, r2) = create(&a.db, &[], series::DR, "u1");
    let (_, c1) = create(&a.db, &[], series::CLIENT, "u1");
    assert_eq!(r1.no, "DR-2026-0001");
    assert_eq!(r2.no, "DR-2026-0002");
    assert_eq!(c1.no, "CL-0001");
    assert!(r1.confirmed());
    assert_eq!(no_of(&a.db, series::DR, &d1), "DR-2026-0001");

    // A document gets one number per series.
    let again = r1.clone();
    let err =
        a.db.write(Actor::system(), move |tx| {
            numbering::assign(tx, &again, "todos")
        })
        .unwrap_err();
    assert!(matches!(err, Error::AlreadyExists { .. }));
}

#[test]
fn reachable_peers_never_hand_out_the_same_number() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);

    // B claims before A's number has synced: the claim on B blocks it.
    let (_, ra) = create(&a.db, &[&b.db], series::DR, "u1");
    let (_, rb) = create(&b.db, &[&a.db], series::DR, "u2");
    assert_eq!((ra.seq, rb.seq), (1, 2));
    assert_eq!(rb.reached, 1);

    sync(&a.db, &b.db);
    let (_, ra) = create(&a.db, &[], series::DR, "u1");
    assert_eq!(ra.seq, 3);
}

#[test]
fn offline_clash_renumbers_the_later_document_once() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);

    let (da, ra) = create(&a.db, &[&Down], series::DR, "ana");
    b.clock.advance(1_000);
    let (db_, rb) = create(&b.db, &[&Down], series::DR, "ben");
    assert_eq!(
        (ra.no.as_str(), rb.no.as_str()),
        ("DR-2026-0001", "DR-2026-0001")
    );
    assert!(!ra.confirmed() && !rb.confirmed());

    sync(&a.db, &b.db);
    // A's document is earlier, so A has nothing to fix.
    assert!(
        numbering::resolve_clashes(&a.db, &[&b.db])
            .unwrap()
            .is_empty()
    );
    let fixed = numbering::resolve_clashes(&b.db, &[&a.db]).unwrap();
    assert_eq!(fixed.len(), 1);
    assert_eq!(fixed[0].old_no, "DR-2026-0001");
    assert_eq!(fixed[0].new_no, "DR-2026-0002");
    assert!(fixed[0].confirmed);
    assert!(
        numbering::resolve_clashes(&b.db, &[&a.db])
            .unwrap()
            .is_empty()
    );

    sync(&a.db, &b.db);
    assert_eq!(a.db.dump().unwrap(), b.db.dump().unwrap());
    assert_eq!(no_of(&a.db, series::DR, &da), "DR-2026-0001");
    assert_eq!(no_of(&a.db, series::DR, &db_), "DR-2026-0002");

    let dump = a.db.dump().unwrap();
    let row = dump["doc_numbers"]
        .iter()
        .find(|r| r["ref_id"] == json!(db_))
        .unwrap();
    assert_eq!(row["prev_no"], json!("DR-2026-0001"));
    // Both users get a to-do pointing at the renumbered document.
    let mut notes: Vec<_> = dump["todos"]
        .iter()
        .filter(|t| t["ref_id"] == json!(db_))
        .map(|t| t["assignee_id"].as_str().unwrap().to_string())
        .collect();
    notes.sort();
    assert_eq!(notes, ["ana", "ben"]);
    assert!(
        dump["audit_log"]
            .iter()
            .any(|l| l["tbl"] == json!("doc_numbers") && l["action"] == json!("update"))
    );
}

#[test]
fn format_comes_from_settings_and_years_reset() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let key = format_key(series::DR);
    a.db.write(Actor::system(), move |tx| {
        tx.upsert(
            "settings",
            &derived_id("setting", &key),
            json!({ "key": key, "value": r#"{"prefix":"D","pad":6}"# }),
        )
    })
    .unwrap();
    let (_, r) = create(&a.db, &[], series::DR, "u1");
    assert_eq!(r.no, "D-2026-000001");

    // 2027-01-01 08:00 in Manila.
    a.clock.set(1_798_761_600_000);
    let (_, r) = create(&a.db, &[], series::DR, "u1");
    assert_eq!(
        (r.period.as_str(), r.no.as_str()),
        ("2027", "D-2027-000001")
    );
    // Client codes don't reset.
    create(&a.db, &[], series::CLIENT, "u1");
    a.clock.set(T0);
    let (_, c) = create(&a.db, &[], series::CLIENT, "u1");
    assert_eq!(c.no, "CL-0002");
}

#[test]
fn abandoned_claims_expire() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let claim = NumberClaim {
        series: series::DR.into(),
        period: "2026".into(),
        seq: 1,
        ref_id: "cancelled-form".into(),
        site: "x".into(),
    };
    assert!(a.db.claim_number(claim.clone()).unwrap().granted);
    // Re-claiming for the same document is fine; another document is refused.
    assert!(a.db.claim_number(claim.clone()).unwrap().granted);
    let other = NumberClaim {
        ref_id: "other".into(),
        ..claim.clone()
    };
    assert_eq!(
        a.db.claim_number(other.clone()).unwrap(),
        ClaimReply {
            granted: false,
            max: 1
        }
    );
    let (_, r) = create(&a.db, &[], series::DR, "u1");
    assert_eq!(r.seq, 2);

    a.clock.advance(CLAIM_TTL_MS);
    let (_, r) = create(&a.db, &[], series::DR, "u1");
    assert_eq!(r.seq, 3, "numbers never go backwards");
    let fresh = NumberClaim { seq: 1, ..other };
    assert!(a.db.claim_number(fresh).unwrap().granted);
}

#[test]
fn restore_takes_over_old_site_but_a_copy_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let old = a.db.site_id().to_string();
    let backup = dir.path().join("backup.db");
    a.db.backup_to(&backup).unwrap();

    let restored_path = dir.path().join("restored").join("sanco.db");
    std::fs::create_dir_all(restored_path.parent().unwrap()).unwrap();
    std::fs::copy(&backup, &restored_path).unwrap();
    let mut o = opts(&restored_path, "a", &a.clock, 5);
    o.new_identity = true;
    let restored = Db::open(o).unwrap();
    assert_ne!(restored.site_id(), old);
    assert_eq!(restored.previous_sites().unwrap(), vec![old.clone()]);

    // A copy of the restored database (e.g. a join snapshot) inherits nothing.
    let copy = dir.path().join("copy").join("sanco.db");
    restored.backup_to(&copy).unwrap();
    let joined = Db::open(opts(&copy, "laptop", &a.clock, 6)).unwrap();
    assert!(joined.previous_sites().unwrap().is_empty());
}
