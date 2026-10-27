mod common;

use common::{TestPeer, pull};
use sanco_core::domain::boxes::Serials;
use sanco_core::domain::inventory::{self, StockDocInput, StockKind, StockLineInput};
use sanco_core::domain::items::{self, ItemInput};
use sanco_core::domain::locations;
use sanco_core::domain::users::{self, NewUser};
use sanco_core::ids::derived_id;
use sanco_core::sync::numbering;
use sanco_core::{Actor, Db, Error};

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

fn office() -> String {
    derived_id("location", "office")
}

fn warehouse() -> String {
    derived_id("location", "warehouse")
}

fn add_item(db: &Db, actor: &Actor, code: &str, tracks_expiry: bool) -> String {
    let input = ItemInput {
        id: None,
        code: code.into(),
        name: format!("Item {code}"),
        unit: "pc".into(),
        category: "Gloves".into(),
        supplier_id: None,
        tracks_expiry,
        reorder_level: Some(10),
        default_margin_bp: None,
        notes: String::new(),
        active: true,
        serial_code: String::new(),
        serial_digits: None,
        pcs_per_box: None,
    };
    db.write(actor.clone(), move |tx| items::save(tx, &input, true))
        .unwrap()
}

fn line(item: &str, expiry: Option<&str>, qty: i64) -> StockLineInput {
    StockLineInput {
        item_id: item.into(),
        expiry: expiry.map(String::from),
        qty,
        box_id: None,
        pcs_per_box: None,
    }
}

fn doc(kind: StockKind, from: &str, to: Option<&str>, lines: Vec<StockLineInput>) -> StockDocInput {
    StockDocInput {
        kind,
        date: TODAY.into(),
        location_id: from.into(),
        to_location_id: to.map(String::from),
        reason: if kind == StockKind::Adjustment {
            "Count".into()
        } else {
            String::new()
        },
        notes: String::new(),
        lines,
        serial_year: String::new(),
    }
}

fn post(db: &Db, actor: &Actor, input: StockDocInput) -> Result<String, Error> {
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], input.kind.series(), &id).unwrap();
    db.write(actor.clone(), move |tx| {
        inventory::create(tx, &input, &res, &mut Serials::none())
    })
}

/// (location, expiry, qty) of an item's non-zero lots.
fn lots(db: &Db, item: &str) -> Vec<(String, Option<String>, i64)> {
    db.read(|c| inventory::lots(c, Some(item), TODAY))
        .unwrap()
        .into_iter()
        .map(|l| (l.location_id, l.expiry, l.qty))
        .collect()
}

fn void(db: &Db, actor: &Actor, id: &str, reason: &str) -> Result<(), Error> {
    let (id, reason) = (id.to_string(), reason.to_string());
    db.write(actor.clone(), move |tx| inventory::void(tx, &id, &reason))
}

fn delete_item(db: &Db, actor: &Actor, id: &str) -> Result<(), Error> {
    let id = id.to_string();
    db.write(actor.clone(), move |tx| items::delete(tx, &id))
}

fn invalid(r: Result<String, Error>) -> String {
    match r {
        Err(Error::Invalid(msg)) => msg,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[test]
fn receipts_transfers_and_adjustments_move_stock() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV-100", true);
    let tape = add_item(&a.db, &actor, "TAPE", false);

    let sr = post(
        &a.db,
        &actor,
        doc(
            StockKind::Receipt,
            &warehouse(),
            None,
            vec![
                line(&glv, Some("2027-03-31"), 100),
                line(&glv, Some("2026-12-31"), 20),
                // Expiry is dropped for items that don't track it.
                line(&tape, Some("2030-01-01"), 50),
            ],
        ),
    )
    .unwrap();
    let detail = a.db.read(|c| inventory::get_doc(c, &sr)).unwrap().unwrap();
    assert_eq!(detail.doc.no.as_deref(), Some("SR-2026-0001"));
    assert_eq!(detail.lines.len(), 3);
    assert_eq!(detail.lines[2].expiry, None);
    assert_eq!(detail.doc.summary, "GLV-100 ×100, GLV-100 ×20, TAPE ×50");

    let tr = post(
        &a.db,
        &actor,
        doc(
            StockKind::Transfer,
            &warehouse(),
            Some(&office()),
            vec![line(&glv, Some("2026-12-31"), 15)],
        ),
    )
    .unwrap();
    post(
        &a.db,
        &actor,
        doc(
            StockKind::Adjustment,
            &warehouse(),
            None,
            vec![line(&tape, None, -3)],
        ),
    )
    .unwrap();

    assert_eq!(
        lots(&a.db, &glv),
        vec![
            (office(), Some("2026-12-31".into()), 15),
            (warehouse(), Some("2026-12-31".into()), 5),
            (warehouse(), Some("2027-03-31".into()), 100),
        ]
    );
    assert_eq!(lots(&a.db, &tape), vec![(warehouse(), None, 47)]);

    let stock = a.db.read(|c| inventory::item_stock(c, TODAY)).unwrap();
    let g = stock.iter().find(|s| s.item_id == glv).unwrap();
    assert_eq!(g.on_hand, 120);
    assert_eq!(g.next_expiry.as_deref(), Some("2026-12-31"));
    assert_eq!(
        g.by_location
            .iter()
            .map(|l| (l.location_id.clone(), l.qty))
            .collect::<Vec<_>>(),
        vec![(office(), 15), (warehouse(), 105)]
    );

    // The item's history: the transfer gives an out and an in row.
    let moves = a.db.read(|c| inventory::item_moves(c, &glv)).unwrap();
    assert_eq!(moves.len(), 4);
    assert!(moves.iter().any(|m| m.doc_id == tr && m.qty == -15));
    assert!(moves.iter().any(|m| m.doc_id == tr && m.qty == 15));

    // Voiding the transfer moves the stock back.
    void(&a.db, &actor, &tr, "wrong lot").unwrap();
    assert_eq!(
        lots(&a.db, &glv),
        vec![
            (warehouse(), Some("2026-12-31".into()), 20),
            (warehouse(), Some("2027-03-31".into()), 100),
        ]
    );
    let voided = a.db.read(|c| inventory::get_doc(c, &tr)).unwrap().unwrap();
    assert!(voided.doc.voided);
    assert_eq!(voided.doc.voided_by_name.as_deref(), Some("Ana"));
    assert_eq!(voided.doc.void_reason.as_deref(), Some("wrong lot"));
    let again = void(&a.db, &actor, &tr, "x");
    assert!(matches!(again, Err(Error::Invalid(_))));
}

#[test]
fn stock_never_goes_below_zero_locally() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV-100", true);
    let sr = post(
        &a.db,
        &actor,
        doc(
            StockKind::Receipt,
            &warehouse(),
            None,
            vec![line(&glv, Some("2027-03-31"), 10)],
        ),
    )
    .unwrap();

    // More than on hand, a lot that doesn't exist, the wrong location.
    let msg = invalid(post(
        &a.db,
        &actor,
        doc(
            StockKind::Transfer,
            &warehouse(),
            Some(&office()),
            vec![line(&glv, Some("2027-03-31"), 11)],
        ),
    ));
    assert_eq!(
        msg,
        "not enough GLV-100 (expiry 2027-03-31) at Warehouse: 10 pc on hand, 11 pc needed"
    );
    // The same check runs before a number is reserved.
    let input = doc(
        StockKind::Transfer,
        &warehouse(),
        Some(&office()),
        vec![line(&glv, Some("2027-03-31"), 11)],
    );
    match a.db.read(|c| inventory::validate(c, &input, TODAY)) {
        Err(Error::Invalid(m)) => assert_eq!(m, msg),
        other => panic!("{other:?}"),
    }
    let mut ok = input.clone();
    ok.lines[0].qty = 10;
    a.db.read(|c| inventory::validate(c, &ok, TODAY)).unwrap();
    invalid(post(
        &a.db,
        &actor,
        doc(
            StockKind::Adjustment,
            &warehouse(),
            None,
            vec![line(&glv, Some("2027-04-01"), -1)],
        ),
    ));
    invalid(post(
        &a.db,
        &actor,
        doc(
            StockKind::Adjustment,
            &office(),
            None,
            vec![line(&glv, Some("2027-03-31"), -1)],
        ),
    ));
    // Two lines that together take too much.
    invalid(post(
        &a.db,
        &actor,
        doc(
            StockKind::Adjustment,
            &warehouse(),
            None,
            vec![
                line(&glv, Some("2027-03-31"), -6),
                line(&glv, Some("2027-03-31"), -6),
            ],
        ),
    ));
    // A refused document leaves nothing behind, not even its number.
    assert_eq!(a.db.read(inventory::list_docs).unwrap().len(), 1);

    // Can't void a receipt whose stock has moved on.
    post(
        &a.db,
        &actor,
        doc(
            StockKind::Transfer,
            &warehouse(),
            Some(&office()),
            vec![line(&glv, Some("2027-03-31"), 4)],
        ),
    )
    .unwrap();
    let r = void(&a.db, &actor, &sr, "typo");
    match r {
        Err(Error::Invalid(msg)) => {
            assert!(msg.starts_with("can't void: not enough GLV-100"), "{msg}")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn documents_are_validated() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV-100", true);
    let ok = || {
        doc(
            StockKind::Receipt,
            &warehouse(),
            None,
            vec![line(&glv, Some("2027-03-31"), 1)],
        )
    };

    let mut d = ok();
    d.lines[0].expiry = None;
    assert_eq!(
        invalid(post(&a.db, &actor, d)),
        "enter the expiry date of GLV-100"
    );
    let mut d = ok();
    d.lines[0].expiry = Some("2027-02-30".into());
    invalid(post(&a.db, &actor, d));
    let mut d = ok();
    d.lines[0].qty = -1;
    invalid(post(&a.db, &actor, d));
    let mut d = ok();
    d.lines.clear();
    invalid(post(&a.db, &actor, d));
    let mut d = ok();
    d.date = "2026-10-22".into();
    assert_eq!(
        invalid(post(&a.db, &actor, d)),
        "the date can't be in the future"
    );
    let mut d = ok();
    d.kind = StockKind::Transfer;
    assert_eq!(
        invalid(post(&a.db, &actor, d)),
        "choose where the stock goes"
    );
    let mut d = ok();
    d.kind = StockKind::Transfer;
    d.to_location_id = Some(warehouse());
    invalid(post(&a.db, &actor, d));
    let mut d = ok();
    d.kind = StockKind::Adjustment;
    assert_eq!(
        invalid(post(&a.db, &actor, d)),
        "enter a reason for the adjustment"
    );

    let id = a.db.new_id().unwrap();
    let res = numbering::reserve(&a.db, &[], StockKind::Transfer.series(), &id).unwrap();
    let input = ok();
    let wrong = a.db.write(actor.clone(), move |tx| {
        inventory::create(tx, &input, &res, &mut Serials::none())
    });
    assert!(matches!(wrong, Err(Error::Invalid(_))));

    post(&a.db, &actor, ok()).unwrap();
}

#[test]
fn locations_and_items_with_movements_cannot_be_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV-100", true);
    let unused = add_item(&a.db, &actor, "UNUSED", true);
    let sr = post(
        &a.db,
        &actor,
        doc(
            StockKind::Receipt,
            &warehouse(),
            None,
            vec![line(&glv, Some("2027-03-31"), 5)],
        ),
    )
    .unwrap();
    void(&a.db, &actor, &sr, "test").unwrap();

    // Even a voided document keeps them.
    let r =
        a.db.write(actor.clone(), |tx| locations::delete(tx, &warehouse()));
    assert!(matches!(r, Err(Error::Invalid(_))));
    let r = delete_item(&a.db, &actor, &glv);
    assert!(matches!(r, Err(Error::Invalid(_))));

    a.db.write(actor.clone(), |tx| locations::delete(tx, &office()))
        .unwrap();
    delete_item(&a.db, &actor, &unused).unwrap();
}

#[test]
fn low_stock_and_expiry_lists() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    // Reorder level 10 for both.
    let glv = add_item(&a.db, &actor, "GLV-100", true);
    let msk = add_item(&a.db, &actor, "MSK-1", true);
    post(
        &a.db,
        &actor,
        doc(
            StockKind::Receipt,
            &warehouse(),
            None,
            vec![
                line(&glv, Some("2026-10-01"), 3),
                line(&glv, Some("2027-01-19"), 4),
                line(&glv, Some("2027-01-20"), 3),
                line(&msk, Some("2028-01-01"), 11),
            ],
        ),
    )
    .unwrap();

    let low = a.db.read(|c| inventory::low_stock(c, TODAY)).unwrap();
    assert_eq!(
        low.iter().map(|s| s.code.as_str()).collect::<Vec<_>>(),
        ["GLV-100"]
    );

    // 90 days from 2026-10-21 is 2027-01-19: the expired lot and that day.
    let soon = a.db.read(|c| inventory::expiring(c, TODAY, 90)).unwrap();
    assert_eq!(
        soon.iter()
            .map(|l| (l.expiry.clone().unwrap(), l.days_left.unwrap()))
            .collect::<Vec<_>>(),
        vec![("2026-10-01".into(), -20), ("2027-01-19".into(), 90)]
    );
}

#[test]
fn offline_pcs_can_oversell_and_it_shows_as_negative() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV-100", true);
    post(
        &a.db,
        &actor,
        doc(
            StockKind::Receipt,
            &warehouse(),
            None,
            vec![line(&glv, Some("2027-03-31"), 10)],
        ),
    )
    .unwrap();
    pull(&b.db, &a.db, 100);
    assert_eq!(lots(&b.db, &glv), lots(&a.db, &glv));

    // Both PCs, apart, take 8 of the 10.
    let take = || {
        doc(
            StockKind::Adjustment,
            &warehouse(),
            None,
            vec![line(&glv, Some("2027-03-31"), -8)],
        )
    };
    post(&a.db, &actor, take()).unwrap();
    post(&b.db, &actor, take()).unwrap();
    pull(&a.db, &b.db, 100);
    pull(&b.db, &a.db, 100);

    for db in [&a.db, &b.db] {
        assert_eq!(
            lots(db, &glv),
            vec![(warehouse(), Some("2027-03-31".into()), -6)]
        );
        let s = db.read(|c| inventory::item_stock(c, TODAY)).unwrap();
        assert!(s[0].negative && s[0].low);
        // Separate documents: nothing conflicts.
        assert!(
            db.read(|c| sanco_core::domain::conflicts::list(c, true))
                .unwrap()
                .is_empty()
        );
    }
    // Further reductions are refused until the count is fixed.
    invalid(post(
        &a.db,
        &actor,
        doc(
            StockKind::Adjustment,
            &warehouse(),
            None,
            vec![line(&glv, Some("2027-03-31"), -1)],
        ),
    ));
    post(
        &a.db,
        &actor,
        doc(
            StockKind::Adjustment,
            &warehouse(),
            None,
            vec![line(&glv, Some("2027-03-31"), 6)],
        ),
    )
    .unwrap();
    assert!(lots(&a.db, &glv).is_empty());
}
