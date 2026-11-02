//! M14: the one-time opening stock import.

mod common;

use common::TestPeer;
use sanco_core::domain::import::{self, ImportItem, ImportLine, ImportPlan};
use sanco_core::domain::locations::{self, LocationInput};
use sanco_core::domain::lots;
use sanco_core::domain::users::{self, NewUser};
use sanco_core::sync::numbering::{self, Reservation};
use sanco_core::{Actor, Db};

const TODAY: &str = "2026-10-21";

fn setup(db: &Db) -> Actor {
    let actor = users::create_first_admin(
        db,
        NewUser {
            username: "ana".into(),
            display_name: "Ana".into(),
            password: "secret1".into(),
            role_ids: vec![],
        },
    )
    .unwrap()
    .actor();
    db.write(actor.clone(), |tx| {
        locations::save(
            tx,
            &LocationInput {
                id: None,
                name: "KB".into(),
                notes: String::new(),
                sort: 3,
                active: true,
            },
        )
    })
    .unwrap();
    actor
}

fn item(code: &str) -> ImportItem {
    ImportItem {
        code: code.into(),
        name: format!("{code} name"),
    }
}

fn line(code: &str, date: &str, location: &str, qty: i64) -> ImportLine {
    ImportLine {
        code: code.into(),
        date: date.into(),
        location: location.into(),
        qty,
    }
}

fn plan() -> ImportPlan {
    ImportPlan {
        items: vec![item("DBC-6"), item("MS-200M")],
        lines: vec![
            line("DBC-6", "2026-07-15", "Warehouse", 100),
            line("DBC-6", "2026-07-15", "KB", 10),
            line("DBC-6", "2026-10-21", "Warehouse", 40),
            line("MS-200M", "2026-10-21", "Office", 5),
            line("MS-200M", "2026-10-21", "Warehouse", 7),
        ],
    }
}

fn reserve(db: &Db, plan: &ImportPlan) -> Vec<Reservation> {
    (0..import::document_count(plan))
        .map(|_| {
            let id = db.new_id().unwrap();
            numbering::reserve(db, &[], "SR", &id).unwrap()
        })
        .collect()
}

fn run(db: &Db, actor: &Actor, plan: &ImportPlan) -> sanco_core::Result<import::ImportSummary> {
    let res = reserve(db, plan);
    let plan = plan.clone();
    db.write(actor.clone(), move |tx| import::apply(tx, &plan, &res))
}

#[test]
fn imports_items_and_one_receipt_per_date_and_location() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);

    let p = plan();
    a.db.read(|c| import::validate(c, &p)).unwrap();
    let sum = run(&a.db, &actor, &p).unwrap();
    assert_eq!((sum.items, sum.documents, sum.lines), (2, 4, 5));
    assert_eq!(
        sum.totals,
        vec![
            ("KB".to_string(), 10),
            ("Office".to_string(), 5),
            ("Warehouse".to_string(), 147)
        ]
    );

    // Two shipments of DBC-6 make two lots; MS-200M has one.
    let all = a.db.read(|c| lots::list(c, TODAY)).unwrap();
    let months = |code: &str| -> Vec<(String, i64)> {
        all.iter()
            .filter(|l| l.item_code == code)
            .map(|l| (l.month.clone(), l.on_hand))
            .collect()
    };
    assert_eq!(
        months("DBC-6"),
        vec![("2026-07".into(), 110), ("2026-10".into(), 40)]
    );
    assert_eq!(months("MS-200M"), vec![("2026-10".into(), 12)]);
}

#[test]
fn refuses_to_run_twice_or_with_a_bad_plan() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);

    let mut bad = plan();
    bad.lines.push(line("NOPE", "2026-07-15", "KB", 1));
    assert!(run(&a.db, &actor, &bad).is_err());
    let mut bad = plan();
    bad.lines[0].location = "Nowhere".into();
    assert!(run(&a.db, &actor, &bad).is_err());
    let mut bad = plan();
    bad.lines[0].qty = 0;
    assert!(run(&a.db, &actor, &bad).is_err());
    let mut bad = plan();
    bad.lines.push(bad.lines[0].clone());
    assert!(run(&a.db, &actor, &bad).is_err());
    // Nothing was written by the refused attempts.
    let n: i64 =
        a.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?))
            .unwrap();
    assert_eq!(n, 0);

    run(&a.db, &actor, &plan()).unwrap();
    assert!(run(&a.db, &actor, &plan()).is_err());
}

#[test]
fn splits_a_document_at_500_lines() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);

    let items: Vec<ImportItem> = (0..501).map(|i| item(&format!("I-{i}"))).collect();
    let lines = items
        .iter()
        .map(|i| line(&i.code, "2026-10-21", "Warehouse", 1))
        .collect();
    let p = ImportPlan { items, lines };
    assert_eq!(import::document_count(&p), 2);
    let sum = run(&a.db, &actor, &p).unwrap();
    assert_eq!((sum.items, sum.documents, sum.lines), (501, 2, 501));
}

/// Dry run of a real import file: `SANCO_IMPORT_FILE=... cargo test -p
/// sanco-core --test import -- --ignored --nocapture`.
#[test]
#[ignore]
fn dry_run_of_a_real_file() {
    let path = std::env::var("SANCO_IMPORT_FILE").expect("set SANCO_IMPORT_FILE");
    let p: ImportPlan = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    // 2026-11-02, after the file's default date.
    a.clock.set(common::T0 + 12 * 86_400_000);
    let actor = setup(&a.db);
    a.db.write(actor.clone(), |tx| {
        locations::save(
            tx,
            &LocationInput {
                id: None,
                name: "Warehouse".into(),
                notes: String::new(),
                sort: 2,
                active: true,
            },
        )
        .or(Ok(String::new()))
    })
    .unwrap();
    let sum = run(&a.db, &actor, &p).unwrap();
    println!("{sum:#?}");
    let all = a.db.read(|c| lots::list(c, "2026-11-02")).unwrap();
    for code in [
        "DBC-6",
        "DBC-9",
        "B-043-608-BK",
        "CK-605P",
        "H-01",
        "H-05",
        "H-06",
        "H-08",
        "H-09",
        "H-16",
        "TH01C",
        "MS-116",
        "MS-200M",
        "CK-605-01A",
        "EN1-EB1",
    ] {
        let months: Vec<_> = all
            .iter()
            .filter(|l| l.item_code == code)
            .map(|l| format!("{}={}", l.month, l.on_hand))
            .collect();
        println!("{code}: {}", months.join(", "));
    }
    let n: i64 =
        a.db.read(|c| {
            Ok(
                c.query_row("SELECT COUNT(*) FROM items WHERE _deleted=0", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(n, 251);
}
