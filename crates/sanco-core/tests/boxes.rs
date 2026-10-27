//! M11: boxes and box serials.

mod common;

use common::{TestPeer, pull};
use sanco_core::domain::bookings::{self, BookingAction, BookingInput, BookingLineInput};
use sanco_core::domain::boxes::{self, BoxDrawInput, SerialNeed, Serials};
use sanco_core::domain::clients::{self, ClientInput};
use sanco_core::domain::delivery::{self, DeliverInput, DrLineInput, DrRequestInput};
use sanco_core::domain::inventory::{self, StockDocInput, StockKind, StockLineInput};
use sanco_core::domain::items::{self, ItemInput};
use sanco_core::domain::purchasing::{
    self, PoAction, PoDetail, PoInput, PoLineInput, PoStatusInput, ReceiptInput, ReceiptLineInput,
};
use sanco_core::domain::suppliers::{self, SupplierInput};
use sanco_core::domain::users::{self, NewUser};
use sanco_core::ids::derived_id;
use sanco_core::sync::numbering::{self, NumberPeer, series};
use sanco_core::{Actor, Db, Error, Result};

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

fn office() -> String {
    derived_id("location", "office")
}

fn invalid<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(Error::Invalid(msg)) => msg,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

/// An item; `serial` = (code, digits, pcs per box).
fn add_item(
    db: &Db,
    actor: &Actor,
    code: &str,
    tracks_expiry: bool,
    serial: Option<(&str, i64, Option<i64>)>,
) -> String {
    let input = ItemInput {
        id: None,
        code: code.into(),
        name: format!("Item {code}"),
        unit: "pc".into(),
        category: String::new(),
        supplier_id: None,
        tracks_expiry,
        reorder_level: None,
        default_margin_bp: Some(2_000),
        notes: String::new(),
        active: true,
        serial_code: serial.map(|s| s.0.to_string()).unwrap_or_default(),
        serial_digits: serial.map(|s| s.1),
        pcs_per_box: serial.and_then(|s| s.2),
    };
    db.write(actor.clone(), move |tx| items::save(tx, &input, true))
        .unwrap()
}

fn add_supplier(db: &Db, actor: &Actor) -> String {
    let input = SupplierInput {
        id: None,
        name: "Formosa Medical".into(),
        country: "TW".into(),
        currency: "USD".into(),
        contact_person: String::new(),
        phone: String::new(),
        email: String::new(),
        address: String::new(),
        notes: String::new(),
        active: true,
    };
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::SUPPLIER, &id).unwrap();
    db.write(actor.clone(), move |tx| suppliers::create(tx, &input, &res))
        .unwrap()
}

/// Reserves what `needs` asks for, on this PC only.
fn reserve(db: &Db, peers: &[&dyn NumberPeer], needs: Vec<SerialNeed>, ref_id: &str) -> Serials {
    Serials::new(
        needs
            .iter()
            .map(|n| {
                numbering::reserve_batch(db, peers, &n.series, &n.period, ref_id, n.count).unwrap()
            })
            .collect(),
    )
}

fn po_line(item: &str, qty: i64, per_box: Option<i64>) -> PoLineInput {
    PoLineInput {
        id: None,
        item_id: item.into(),
        qty,
        price_minor: Some(100),
        notes: String::new(),
        pcs_per_box: per_box,
    }
}

fn create_po(
    db: &Db,
    actor: &Actor,
    supplier: &str,
    year: &str,
    lines: Vec<PoLineInput>,
) -> String {
    let input = PoInput {
        id: None,
        supplier_id: supplier.into(),
        date: TODAY.into(),
        currency: "USD".into(),
        fx_rate: "56".into(),
        expected_date: String::new(),
        supplier_ref: String::new(),
        notes: String::new(),
        serial_year: year.into(),
        lines,
    };
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::PO, &id).unwrap();
    db.write(actor.clone(), move |tx| {
        purchasing::create(tx, &input, &res, true)
    })
    .unwrap()
}

fn po_status(db: &Db, actor: &Actor, id: &str, action: PoAction) -> Result<()> {
    let input = PoStatusInput {
        action,
        date: String::new(),
        reason: "test".into(),
    };
    let needs = db.read(|c| purchasing::serials_for_status(c, id, &input, db.now_ms()))?;
    let mut serials = reserve(db, &[], needs, id);
    let id = id.to_string();
    db.write(actor.clone(), move |tx| {
        purchasing::set_status(tx, &id, &input, &mut serials)
    })
}

fn detail(db: &Db, id: &str) -> PoDetail {
    db.read(|c| purchasing::get(c, id)).unwrap().unwrap()
}

/// The input that saves the PO as it is, for edits.
fn as_input(d: &PoDetail) -> PoInput {
    PoInput {
        id: Some(d.po.id.clone()),
        supplier_id: d.po.supplier_id.clone().unwrap(),
        date: d.po.date.clone(),
        currency: d.po.currency.clone(),
        fx_rate: d.po.fx_rate.clone().unwrap_or_default(),
        expected_date: String::new(),
        supplier_ref: String::new(),
        notes: String::new(),
        serial_year: d.po.serial_year.clone().unwrap_or_default(),
        lines: d
            .lines
            .iter()
            .map(|l| PoLineInput {
                id: Some(l.id.clone()),
                item_id: l.item_id.clone(),
                qty: l.qty,
                price_minor: l.price_minor,
                notes: String::new(),
                pcs_per_box: l.pcs_per_box,
            })
            .collect(),
    }
}

/// Saves an edit; returns how many serials it reserved.
fn update_po(db: &Db, actor: &Actor, input: PoInput) -> Result<i64> {
    let needs = db.read(|c| purchasing::serials_for_update(c, &input, true, db.now_ms()))?;
    let count = needs.iter().map(|n| n.count).sum();
    let id = input.id.clone().unwrap();
    let mut serials = reserve(db, &[], needs, &id);
    db.write(actor.clone(), move |tx| {
        purchasing::update(tx, &input, true, &mut serials)
    })?;
    Ok(count)
}

/// Serials and sizes of a PO line's boxes.
fn line_boxes(d: &PoDetail, i: usize) -> Vec<(String, i64)> {
    d.lines[i]
        .boxes
        .iter()
        .map(|b| (b.serial.clone().unwrap(), b.pcs))
        .collect()
}

fn box_id(d: &PoDetail, line: usize, serial: &str) -> String {
    d.lines[line]
        .boxes
        .iter()
        .find(|b| b.serial.as_deref() == Some(serial))
        .unwrap()
        .id
        .clone()
}

fn receive(db: &Db, actor: &Actor, po: &str, lines: Vec<ReceiptLineInput>) -> Result<String> {
    let input = ReceiptInput {
        po_id: po.into(),
        date: TODAY.into(),
        location_id: warehouse(),
        notes: String::new(),
        lines,
    };
    db.read(|c| purchasing::validate_receipt(c, &input, TODAY))?;
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::RECEIVING, &id).unwrap();
    db.write(actor.clone(), move |tx| {
        purchasing::receive(tx, &input, &res)
    })
}

fn rr_line(po_line: &str, box_id: Option<&str>, expiry: &str, qty: i64) -> ReceiptLineInput {
    ReceiptLineInput {
        po_line_id: po_line.into(),
        box_id: box_id.map(String::from),
        expiry: Some(expiry.into()),
        qty,
    }
}

/// (serial, location, qty left) of an item's boxes in stock.
fn box_stock(db: &Db, item: &str) -> Vec<(String, String, i64)> {
    db.read(|c| boxes::box_stock(c, Some(item), None, TODAY))
        .unwrap()
        .into_iter()
        .map(|b| (b.serial.unwrap(), b.location_id, b.qty))
        .collect()
}

fn serial_box(db: &Db, item: &str, serial: &str) -> String {
    db.read(|c| boxes::box_stock(c, Some(item), None, TODAY))
        .unwrap()
        .into_iter()
        .find(|b| b.serial.as_deref() == Some(serial))
        .map(|b| b.box_id)
        .unwrap_or_else(|| panic!("no box {serial} in stock"))
}

fn stock_doc(db: &Db, actor: &Actor, input: StockDocInput) -> Result<String> {
    let needs = db.read(|c| inventory::serials_for_doc(c, &input, TODAY, db.now_ms()))?;
    let id = db.new_id().unwrap();
    let mut serials = reserve(db, &[], needs, &id);
    let res = numbering::reserve(db, &[], input.kind.series(), &id).unwrap();
    db.write(actor.clone(), move |tx| {
        inventory::create(tx, &input, &res, &mut serials)
    })
}

fn doc(kind: StockKind, year: &str, lines: Vec<StockLineInput>) -> StockDocInput {
    StockDocInput {
        kind,
        date: TODAY.into(),
        location_id: warehouse(),
        to_location_id: (kind == StockKind::Transfer).then(office),
        reason: "count".into(),
        notes: String::new(),
        serial_year: year.into(),
        lines,
    }
}

fn line(item: &str, expiry: Option<&str>, qty: i64, box_id: Option<&str>) -> StockLineInput {
    StockLineInput {
        item_id: item.into(),
        expiry: expiry.map(String::from),
        qty,
        box_id: box_id.map(String::from),
        pcs_per_box: None,
    }
}

#[test]
fn po_boxes_get_serials_when_ordered_and_follow_the_quantity() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV", true, Some(("MT", 4, Some(10))));
    let msk = add_item(&a.db, &actor, "MSK", false, Some(("MT", 3, None)));
    let tape = add_item(&a.db, &actor, "TAPE", false, None);
    let sup = add_supplier(&a.db, &actor);

    // Items without a code have no boxes; a code needs letters only.
    let bad = ItemInput {
        id: None,
        code: "X".into(),
        name: "X".into(),
        unit: String::new(),
        category: String::new(),
        supplier_id: None,
        tracks_expiry: false,
        reorder_level: None,
        default_margin_bp: None,
        notes: String::new(),
        active: true,
        serial_code: "M2".into(),
        serial_digits: Some(4),
        pcs_per_box: None,
    };
    assert!(
        invalid(a.db.write(actor.clone(), move |tx| items::save(tx, &bad, true)))
            .contains("letters")
    );

    let po = create_po(
        &a.db,
        &actor,
        &sup,
        "2026",
        vec![
            po_line(&glv, 97, None),
            po_line(&msk, 12, Some(4)),
            po_line(&tape, 10, None),
        ],
    );
    // A draft uses no serials.
    assert_eq!(detail(&a.db, &po).po.box_count, 0);

    // MSK has no default box size: ordering a line without one is refused.
    let mut input = as_input(&detail(&a.db, &po));
    input.lines[1].pcs_per_box = None;
    update_po(&a.db, &actor, input).unwrap();
    assert!(invalid(po_status(&a.db, &actor, &po, PoAction::Order)).contains("go in one box"));
    let mut input = as_input(&detail(&a.db, &po));
    input.lines[1].pcs_per_box = Some(4);
    update_po(&a.db, &actor, input).unwrap();

    po_status(&a.db, &actor, &po, PoAction::Order).unwrap();
    let d = detail(&a.db, &po);
    assert_eq!(d.po.box_count, 13);
    let mut want: Vec<(String, i64)> = (1..=9).map(|n| (format!("MT26-{n:04}"), 10)).collect();
    want.push(("MT26-0010".into(), 7));
    assert_eq!(line_boxes(&d, 0), want);
    // Same code, same counter; MSK pads to 3 digits.
    assert_eq!(
        line_boxes(&d, 1),
        vec![
            ("MT26-011".into(), 4),
            ("MT26-012".into(), 4),
            ("MT26-013".into(), 4)
        ]
    );
    assert!(d.lines[2].boxes.is_empty());

    // Another PO in another year starts its own sequence.
    let old = create_po(
        &a.db,
        &actor,
        &sup,
        "2025",
        vec![po_line(&glv, 20, Some(5))],
    );
    po_status(&a.db, &actor, &old, PoAction::Order).unwrap();
    let serials: Vec<String> = line_boxes(&detail(&a.db, &old), 0)
        .into_iter()
        .map(|b| b.0)
        .collect();
    assert_eq!(
        serials,
        ["MT25-0001", "MT25-0002", "MT25-0003", "MT25-0004"]
    );

    // Raising the quantity fills the last box and adds new ones.
    let mut input = as_input(&d);
    input.lines[0].qty = 120;
    assert_eq!(update_po(&a.db, &actor, input).unwrap(), 2);
    let d = detail(&a.db, &po);
    let glv_boxes = line_boxes(&d, 0);
    assert_eq!(glv_boxes.len(), 12);
    assert_eq!(glv_boxes[9], ("MT26-0010".into(), 10));
    assert_eq!(glv_boxes[10], ("MT26-0014".into(), 10));
    assert_eq!(glv_boxes[11], ("MT26-0015".into(), 10));

    // Lowering it takes trailing boxes away; their serials aren't reused.
    let mut input = as_input(&d);
    input.lines[0].qty = 95;
    assert_eq!(update_po(&a.db, &actor, input).unwrap(), 0);
    let d = detail(&a.db, &po);
    let glv_boxes = line_boxes(&d, 0);
    assert_eq!(glv_boxes.len(), 10);
    assert_eq!(glv_boxes[9], ("MT26-0010".into(), 5));
    let mut input = as_input(&d);
    input.lines[0].qty = 100;
    update_po(&a.db, &actor, input).unwrap();
    let d = detail(&a.db, &po);
    assert_eq!(line_boxes(&d, 0)[9], ("MT26-0010".into(), 10));
    let mut input = as_input(&d);
    input.lines[1].qty = 16;
    update_po(&a.db, &actor, input).unwrap();
    let d = detail(&a.db, &po);
    assert_eq!(line_boxes(&d, 1)[3].0, "MT26-016");

    // The serial year is fixed once there are boxes.
    let mut input = as_input(&d);
    input.serial_year = "2027".into();
    assert!(invalid(update_po(&a.db, &actor, input)).contains("back to draft"));

    // Receiving: one line per box.
    let glv_line = d.lines[0].id.clone();
    let b1 = box_id(&d, 0, "MT26-0001");
    let b2 = box_id(&d, 0, "MT26-0002");
    let b10 = box_id(&d, 0, "MT26-0010");
    let msg = invalid(receive(
        &a.db,
        &actor,
        &po,
        vec![rr_line(&glv_line, None, "2027-06-30", 10)],
    ));
    assert!(msg.contains("choose the box"), "{msg}");
    let msg = invalid(receive(
        &a.db,
        &actor,
        &po,
        vec![rr_line(&glv_line, Some(&b1), "2027-06-30", 11)],
    ));
    assert!(msg.contains("holds 10"), "{msg}");
    let msg = invalid(receive(
        &a.db,
        &actor,
        &po,
        vec![
            rr_line(&glv_line, Some(&b1), "2027-06-30", 10),
            rr_line(&glv_line, Some(&b1), "2027-06-30", 10),
        ],
    ));
    assert!(msg.contains("twice"), "{msg}");
    let rr = receive(
        &a.db,
        &actor,
        &po,
        vec![
            rr_line(&glv_line, Some(&b1), "2027-06-30", 10),
            rr_line(&glv_line, Some(&b2), "2027-03-31", 10),
            rr_line(&glv_line, Some(&b10), "2027-06-30", 8),
        ],
    )
    .unwrap();
    let msg = invalid(receive(
        &a.db,
        &actor,
        &po,
        vec![rr_line(&glv_line, Some(&b1), "2027-06-30", 10)],
    ));
    assert!(msg.contains("already received"), "{msg}");
    let d = detail(&a.db, &po);
    assert_eq!(d.lines[0].received, 28);
    assert_eq!(d.lines[0].boxes[0].received, 10);
    assert_eq!(d.lines[0].boxes[9].received, 8);
    let rd =
        a.db.read(|c| purchasing::get_receipt(c, &rr))
            .unwrap()
            .unwrap();
    assert_eq!(rd.lines[0].serial.as_deref(), Some("MT26-0001"));

    // Stock per box, and lots summed with how many boxes are full.
    assert_eq!(
        box_stock(&a.db, &glv),
        vec![
            ("MT26-0001".into(), warehouse(), 10),
            ("MT26-0002".into(), warehouse(), 10),
            ("MT26-0010".into(), warehouse(), 8),
        ]
    );
    let lots =
        a.db.read(|c| inventory::lots(c, Some(&glv), TODAY))
            .unwrap();
    let lot = lots
        .iter()
        .find(|l| l.expiry.as_deref() == Some("2027-06-30"))
        .unwrap();
    assert_eq!(
        (lot.qty, lot.full_boxes, lot.open_boxes, lot.open_pcs),
        (18, 1, 1, 8)
    );
    let stock = a.db.read(|c| inventory::item_stock(c, TODAY)).unwrap();
    let s = stock.iter().find(|s| s.item_id == glv).unwrap();
    assert_eq!((s.on_hand, s.full_boxes, s.open_boxes), (28, 2, 1));

    // Received boxes stay: lowering the quantity only takes unreceived ones.
    let mut input = as_input(&d);
    input.lines[0].qty = 40;
    update_po(&a.db, &actor, input).unwrap();
    let d = detail(&a.db, &po);
    let serials: Vec<String> = line_boxes(&d, 0).into_iter().map(|b| b.0).collect();
    assert_eq!(
        serials,
        ["MT26-0001", "MT26-0002", "MT26-0003", "MT26-0010"]
    );
    assert_eq!(line_boxes(&d, 0)[2].1, 10);

    // Back to draft (nothing received) takes the boxes away; ordering again
    // gives new serials.
    let other = create_po(&a.db, &actor, &sup, "", vec![po_line(&glv, 15, None)]);
    assert_eq!(
        detail(&a.db, &other).po.serial_year.as_deref(),
        Some("2026")
    );
    po_status(&a.db, &actor, &other, PoAction::Order).unwrap();
    assert_eq!(line_boxes(&detail(&a.db, &other), 0)[0].0, "MT26-0017");
    po_status(&a.db, &actor, &other, PoAction::BackToDraft).unwrap();
    assert_eq!(detail(&a.db, &other).po.box_count, 0);
    let mut input = as_input(&detail(&a.db, &other));
    input.serial_year = "2027".into();
    update_po(&a.db, &actor, input).unwrap();
    po_status(&a.db, &actor, &other, PoAction::Order).unwrap();
    assert_eq!(line_boxes(&detail(&a.db, &other), 0)[0].0, "MT27-0001");
    po_status(&a.db, &actor, &other, PoAction::Cancel).unwrap();
    assert_eq!(detail(&a.db, &other).po.box_count, 0);
}

fn add_client(db: &Db, actor: &Actor) -> String {
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::CLIENT, &id).unwrap();
    let input = ClientInput {
        id: None,
        name: "St. Luke's".into(),
        contact_person: String::new(),
        phone: String::new(),
        email: String::new(),
        tin: String::new(),
        address: String::new(),
        terms_days: Some(30),
        margin_tier_bp: None,
        credit_limit_c: None,
        notes: String::new(),
        active: true,
    };
    db.write(actor.clone(), move |tx| {
        clients::create(tx, &input, &res, true)
    })
    .unwrap()
}

/// A confirmed booking for `qty` of `item` at ₱125. Returns (booking, line).
fn booking(db: &Db, actor: &Actor, client: &str, item: &str, qty: i64) -> (String, String) {
    let input = BookingInput {
        id: None,
        client_id: client.into(),
        date: TODAY.into(),
        vat: false,
        client_ref: String::new(),
        deliver_to: String::new(),
        notes: String::new(),
        lines: vec![BookingLineInput {
            id: None,
            item_id: item.into(),
            qty,
            price_c: Some(12_500),
            notes: String::new(),
        }],
    };
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::BOOKING, &id).unwrap();
    let id = db
        .write(actor.clone(), move |tx| bookings::create(tx, &input, &res))
        .unwrap();
    let bid = id.clone();
    db.write(actor.clone(), move |tx| {
        bookings::set_status(tx, &bid, BookingAction::Confirm, "")
    })
    .unwrap();
    let line = db.read(|c| bookings::get(c, &id)).unwrap().unwrap().lines[0]
        .id
        .clone();
    (id, line)
}

fn dr_input(booking: &str, line: &str, qty: i64, draws: Vec<(&str, i64)>) -> DrRequestInput {
    DrRequestInput {
        booking_id: booking.into(),
        date: TODAY.into(),
        from_location_id: warehouse(),
        deliver_to: String::new(),
        notes: String::new(),
        lines: vec![DrLineInput {
            id: None,
            booking_line_id: line.into(),
            expiry: None,
            qty,
            boxes: draws
                .into_iter()
                .map(|(b, q)| BoxDrawInput {
                    box_id: b.into(),
                    qty: q,
                })
                .collect(),
        }],
    }
}

fn deliver(db: &Db, actor: &Actor, input: DrRequestInput, no: &str) -> Result<String> {
    let id = db.write(actor.clone(), move |tx| delivery::request(tx, &input))?;
    let res = numbering::reserve_exact(db, &[], series::DR, no, &id)?;
    let dr = id.clone();
    db.write(actor.clone(), move |tx| {
        delivery::approve(tx, &dr, Some(&res))
    })?;
    let (dr, input) = (
        id.clone(),
        DeliverInput {
            date: TODAY.into(),
            received_by: "Clerk".into(),
        },
    );
    db.write(actor.clone(), move |tx| {
        delivery::deliver(tx, &dr, &input, None)
    })?;
    Ok(id)
}

/// (serial, qty) drawn by the DR's first line.
fn draws(db: &Db, dr: &str) -> Vec<(String, i64)> {
    db.read(|c| delivery::get(c, dr)).unwrap().unwrap().lines[0]
        .boxes
        .iter()
        .map(|b| (b.serial.clone().unwrap(), b.qty))
        .collect()
}

#[test]
fn stock_receipts_make_boxes_and_drs_draw_from_them() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV", true, Some(("MT", 4, Some(10))));
    let tape = add_item(&a.db, &actor, "TAPE", false, Some(("TP", 3, Some(6))));

    // A stock receipt without a box makes boxes in its serial year.
    stock_doc(
        &a.db,
        &actor,
        doc(
            StockKind::Receipt,
            "",
            vec![line(&glv, Some("2027-01-31"), 25, None)],
        ),
    )
    .unwrap();
    let late = stock_doc(
        &a.db,
        &actor,
        doc(
            StockKind::Receipt,
            "2025",
            vec![line(&glv, Some("2026-12-31"), 10, None)],
        ),
    )
    .unwrap();
    assert_eq!(
        box_stock(&a.db, &glv),
        vec![
            ("MT25-0001".into(), warehouse(), 10),
            ("MT26-0001".into(), warehouse(), 10),
            ("MT26-0002".into(), warehouse(), 10),
            ("MT26-0003".into(), warehouse(), 5),
        ]
    );
    let detail =
        a.db.read(|c| inventory::get_doc(c, &late))
            .unwrap()
            .unwrap();
    assert_eq!(detail.lines[0].serial.as_deref(), Some("MT25-0001"));

    // Positive adjustments need a box; outgoing ones need one when there is
    // no stock outside boxes.
    let msg = invalid(stock_doc(
        &a.db,
        &actor,
        doc(StockKind::Adjustment, "", vec![line(&glv, None, 2, None)]),
    ));
    assert!(msg.contains("choose the box"), "{msg}");
    let msg = invalid(stock_doc(
        &a.db,
        &actor,
        doc(StockKind::Adjustment, "", vec![line(&glv, None, -2, None)]),
    ));
    assert!(msg.contains("choose the box"), "{msg}");

    // An adjustment takes 3 out of MT26-0002 (keeping its lot), which is now
    // started; a box can't go below zero.
    let b2 = serial_box(&a.db, &glv, "MT26-0002");
    stock_doc(
        &a.db,
        &actor,
        doc(
            StockKind::Adjustment,
            "",
            vec![line(&glv, None, -3, Some(&b2))],
        ),
    )
    .unwrap();
    let msg = invalid(stock_doc(
        &a.db,
        &actor,
        doc(
            StockKind::Adjustment,
            "",
            vec![line(&glv, None, -8, Some(&b2))],
        ),
    ));
    assert!(msg.contains("in box MT26-0002"), "{msg}");
    let started =
        a.db.read(|c| boxes::box_stock(c, Some(&glv), None, TODAY))
            .unwrap();
    let s2 = started
        .iter()
        .find(|b| b.serial.as_deref() == Some("MT26-0002"))
        .unwrap();
    assert_eq!(
        (s2.qty, s2.open, s2.expiry.as_deref()),
        (7, true, Some("2027-01-31"))
    );

    // A transfer moves a box to the office.
    let b3 = serial_box(&a.db, &glv, "MT26-0003");
    stock_doc(
        &a.db,
        &actor,
        doc(
            StockKind::Transfer,
            "",
            vec![line(&glv, None, 5, Some(&b3))],
        ),
    )
    .unwrap();
    assert!(box_stock(&a.db, &glv).contains(&("MT26-0003".into(), office(), 5)));

    // A DR without draws takes from started boxes first, then the earliest
    // expiry, then the lowest serial.
    let client = add_client(&a.db, &actor);
    let (bk, bl) = booking(&a.db, &actor, &client, &glv, 40);
    let first = deliver(&a.db, &actor, dr_input(&bk, &bl, 12, vec![]), "1001").unwrap();
    assert_eq!(
        draws(&a.db, &first),
        vec![("MT25-0001".into(), 5), ("MT26-0002".into(), 7)]
    );
    assert_eq!(
        box_stock(&a.db, &glv),
        vec![
            ("MT25-0001".into(), warehouse(), 5),
            ("MT26-0001".into(), warehouse(), 10),
            ("MT26-0003".into(), office(), 5),
        ]
    );
    let detail = a.db.read(|c| delivery::get(c, &first)).unwrap().unwrap();
    assert_eq!(detail.lines[0].pcs_per_box, Some(10));

    // Typed draws must add up to the line and hold the item.
    let b1 = serial_box(&a.db, &glv, "MT26-0001");
    let msg = invalid(deliver(
        &a.db,
        &actor,
        dr_input(&bk, &bl, 8, vec![(&b1, 6)]),
        "1002",
    ));
    assert!(msg.contains("add up to 6"), "{msg}");
    stock_doc(
        &a.db,
        &actor,
        doc(StockKind::Receipt, "", vec![line(&tape, None, 6, None)]),
    )
    .unwrap();
    let tp = serial_box(&a.db, &tape, "TP26-001");
    let msg = invalid(deliver(
        &a.db,
        &actor,
        dr_input(&bk, &bl, 6, vec![(&tp, 6)]),
        "1002",
    ));
    assert!(msg.contains("doesn't hold GLV"), "{msg}");
    let msg = invalid(deliver(
        &a.db,
        &actor,
        dr_input(&bk, &bl, 11, vec![(&b1, 11)]),
        "1002",
    ));
    assert!(msg.contains("in box MT26-0001"), "{msg}");
    // Not enough in boxes at the warehouse for an automatic pick.
    let msg = invalid(deliver(
        &a.db,
        &actor,
        dr_input(&bk, &bl, 16, vec![]),
        "1002",
    ));
    assert!(msg.contains("only 15"), "{msg}");

    let second = deliver(&a.db, &actor, dr_input(&bk, &bl, 8, vec![(&b1, 8)]), "1002").unwrap();
    assert_eq!(draws(&a.db, &second), vec![("MT26-0001".into(), 8)]);

    // Editing a delivered DR moves the difference box by box.
    let b25 = serial_box(&a.db, &glv, "MT25-0001");
    let mut edit = dr_input(&bk, &bl, 8, vec![(&b1, 4), (&b25, 4)]);
    edit.lines[0].id = Some(
        a.db.read(|c| delivery::get(c, &second))
            .unwrap()
            .unwrap()
            .lines[0]
            .id
            .clone(),
    );
    let id = second.clone();
    a.db.write(actor.clone(), move |tx| {
        delivery::update(tx, &id, &edit, None)
    })
    .unwrap();
    assert_eq!(
        draws(&a.db, &second),
        vec![("MT25-0001".into(), 4), ("MT26-0001".into(), 4)]
    );
    assert_eq!(
        box_stock(&a.db, &glv),
        vec![
            ("MT25-0001".into(), warehouse(), 1),
            ("MT26-0001".into(), warehouse(), 6),
            ("MT26-0003".into(), office(), 5),
        ]
    );
    // An automatic re-pick counts what the DR itself took as available.
    let mut edit = dr_input(&bk, &bl, 8, vec![]);
    edit.lines[0].id = Some(
        a.db.read(|c| delivery::get(c, &second))
            .unwrap()
            .unwrap()
            .lines[0]
            .id
            .clone(),
    );
    let id = second.clone();
    a.db.write(actor.clone(), move |tx| {
        delivery::update(tx, &id, &edit, None)
    })
    .unwrap();
    assert_eq!(
        draws(&a.db, &second),
        vec![("MT25-0001".into(), 5), ("MT26-0001".into(), 3)]
    );

    // Voiding the first DR puts its pieces back in the same boxes.
    let id = first.clone();
    a.db.write(actor.clone(), move |tx| {
        delivery::void(tx, &id, "wrong client")
    })
    .unwrap();
    assert_eq!(
        box_stock(&a.db, &glv),
        vec![
            ("MT25-0001".into(), warehouse(), 5),
            ("MT26-0001".into(), warehouse(), 7),
            ("MT26-0002".into(), warehouse(), 7),
            ("MT26-0003".into(), office(), 5),
        ]
    );

    // Voiding a stock receipt whose boxes were never used deletes them.
    let msg = invalid(a.db.write(actor.clone(), {
        let late = late.clone();
        move |tx| inventory::void(tx, &late, "typo")
    }));
    assert!(msg.contains("can't void"), "{msg}");
    let extra = stock_doc(
        &a.db,
        &actor,
        doc(
            StockKind::Receipt,
            "",
            vec![line(&glv, Some("2027-05-31"), 20, None)],
        ),
    )
    .unwrap();
    assert!(box_stock(&a.db, &glv).iter().any(|b| b.0 == "MT26-0005"));
    let id = extra.clone();
    a.db.write(actor.clone(), move |tx| inventory::void(tx, &id, "typo"))
        .unwrap();
    assert!(
        !box_stock(&a.db, &glv)
            .iter()
            .any(|b| b.0 == "MT26-0004" || b.0 == "MT26-0005")
    );
    let left: i64 =
        a.db.read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM boxes WHERE doc_id = ?1 AND _deleted = 0",
                [&extra],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn box_serials_are_claimed_in_batches_and_never_renumbered() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    let actor = setup(&a.db);

    // A batch claimed with B blocks the whole range there.
    let peer: &dyn NumberPeer = &b.db;
    let first = numbering::reserve_batch(&a.db, &[peer], "BOX:MT", "2026", "po-1", 5).unwrap();
    assert_eq!((first.first, first.count, first.reached), (1, 5, 1));
    let peer: &dyn NumberPeer = &a.db;
    let next = numbering::reserve_batch(&b.db, &[peer], "BOX:MT", "2026", "po-2", 3).unwrap();
    assert_eq!(next.first, 6);
    let single = numbering::reserve_batch(&b.db, &[peer], "BOX:MT", "2025", "po-3", 1).unwrap();
    assert_eq!(single.first, 1);

    // Two PCs offline from each other give out the same serials: both keep
    // them (no renumbering for box serials).
    pull(&b.db, &a.db, 100);
    let actor_b = actor.clone();
    let glv = add_item(&a.db, &actor, "GLV", false, Some(("GX", 4, Some(10))));
    pull(&b.db, &a.db, 100);
    stock_doc(
        &a.db,
        &actor,
        doc(StockKind::Receipt, "", vec![line(&glv, None, 10, None)]),
    )
    .unwrap();
    stock_doc(
        &b.db,
        &actor_b,
        doc(StockKind::Receipt, "", vec![line(&glv, None, 10, None)]),
    )
    .unwrap();
    pull(&a.db, &b.db, 100);
    pull(&b.db, &a.db, 100);
    // (The stock receipts' own SR numbers are renumbered as usual.)
    for db in [&a.db, &b.db] {
        let renumbered = numbering::resolve_clashes(db, &[]).unwrap();
        assert!(renumbered.iter().all(|r| !r.series.starts_with("BOX:")));
    }
    let serials: Vec<String> = box_stock(&a.db, &glv).into_iter().map(|b| b.0).collect();
    assert_eq!(serials, ["GX26-0001", "GX26-0001"]);
}
