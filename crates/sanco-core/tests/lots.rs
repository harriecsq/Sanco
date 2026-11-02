//! M14: lots. One item's stock that arrived in one month.

mod common;

use common::{TestPeer, pull};
use sanco_core::domain::inventory::{self, StockDocInput, StockKind, StockLineInput};
use sanco_core::domain::items::{self, ItemInput};
use sanco_core::domain::lots::{self, LotInput};
use sanco_core::domain::users::{self, NewUser};
use sanco_core::ids::derived_id;
use sanco_core::sync::numbering;
use sanco_core::{Actor, Db};

const TODAY: &str = "2026-10-21";

fn setup(db: &Db) -> Actor {
    users::create_first_admin(
        db,
        NewUser {
            username: "ana".into(),
            display_name: "Ana".into(),
            password: "secret1".into(),
            role_ids: vec![],
        },
    )
    .unwrap()
    .actor()
}

fn warehouse() -> String {
    derived_id("location", "warehouse")
}

fn add_item(db: &Db, actor: &Actor, tracks_expiry: bool) -> String {
    let input = ItemInput {
        id: None,
        code: "ANE".into(),
        name: "Aneroid".into(),
        unit: "pc".into(),
        category: String::new(),
        supplier_id: None,
        tracks_expiry,
        reorder_level: None,
        default_margin_bp: None,
        notes: String::new(),
        active: true,
    };
    db.write(actor.clone(), move |tx| items::save(tx, &input, true))
        .unwrap()
}

fn receive(db: &Db, actor: &Actor, item: &str, date: &str, qty: i64, serials: (&str, &str)) {
    let input = StockDocInput {
        kind: StockKind::Receipt,
        date: date.into(),
        location_id: warehouse(),
        to_location_id: None,
        reason: String::new(),
        notes: String::new(),
        lines: vec![StockLineInput {
            item_id: item.into(),
            lot_id: None,
            qty,
            expiry: None,
            serial_from: Some(serials.0.into()),
            serial_to: Some(serials.1.into()),
        }],
    };
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], input.kind.series(), &id).unwrap();
    db.write(actor.clone(), move |tx| inventory::create(tx, &input, &res))
        .unwrap();
}

fn list(db: &Db) -> Vec<lots::LotInfo> {
    db.read(|c| lots::list(c, TODAY)).unwrap()
}

#[test]
fn shipments_of_a_month_share_a_lot_and_other_months_stay_apart() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let ane = add_item(&a.db, &actor, false);

    receive(&a.db, &actor, &ane, "2026-07-31", 30, ("ANE 1", "ANE 30"));
    receive(&a.db, &actor, &ane, "2026-10-11", 20, ("X-9", "whatever"));
    // Blank serials on a second shipment keep what the lot has.
    receive(&a.db, &actor, &ane, "2026-07-02", 5, ("", ""));

    let all = list(&a.db);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].month, "2026-07");
    assert_eq!(all[0].on_hand, 35);
    assert_eq!(all[0].serial_from.as_deref(), Some("ANE 1"));
    assert_eq!(all[0].serial_to.as_deref(), Some("ANE 30"));
    assert_eq!(all[1].month, "2026-10");
    assert_eq!(all[1].on_hand, 20);
    // The range is only text: it needn't make sense or fit the qty.
    assert_eq!(all[1].serial_to.as_deref(), Some("whatever"));

    // Stock lists show the oldest lot first, with its range.
    let stock =
        a.db.read(|c| inventory::lots(c, Some(&ane), TODAY))
            .unwrap();
    assert_eq!(
        stock
            .iter()
            .map(|l| (l.month.as_deref(), l.qty))
            .collect::<Vec<_>>(),
        vec![(Some("2026-07"), 35), (Some("2026-10"), 20)]
    );
    assert_eq!(stock[0].serial_from.as_deref(), Some("ANE 1"));
}

#[test]
fn a_lot_stays_editable_and_listed_when_sold_out() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    let actor = setup(&a.db);
    let ane = add_item(&a.db, &actor, false);
    receive(&a.db, &actor, &ane, "2026-07-31", 30, ("ANE 1", "ANE 30"));

    let id = list(&a.db)[0].id.clone();
    let input = LotInput {
        id: id.clone(),
        expiry: Some("2029-01-31".into()),
        serial_from: "ANE 100".into(),
        serial_to: String::new(),
    };
    a.db.write(actor.clone(), move |tx| lots::update(tx, &input))
        .unwrap();

    let adj = StockDocInput {
        kind: StockKind::Adjustment,
        date: TODAY.into(),
        location_id: warehouse(),
        to_location_id: None,
        reason: "Count".into(),
        notes: String::new(),
        lines: vec![StockLineInput {
            item_id: ane.clone(),
            lot_id: Some(id.clone()),
            qty: -30,
            expiry: None,
            serial_from: None,
            serial_to: None,
        }],
    };
    let doc = a.db.new_id().unwrap();
    let res = numbering::reserve(&a.db, &[], adj.kind.series(), &doc).unwrap();
    a.db.write(actor.clone(), move |tx| inventory::create(tx, &adj, &res))
        .unwrap();

    pull(&b.db, &a.db, 100);
    for db in [&a.db, &b.db] {
        let all = list(db);
        assert_eq!(all[0].on_hand, 0);
        assert!(all[0].by_location.is_empty());
        assert_eq!(all[0].serial_from.as_deref(), Some("ANE 100"));
        assert_eq!(all[0].serial_to, None);
        assert_eq!(all[0].expiry.as_deref(), Some("2029-01-31"));
    }
}
