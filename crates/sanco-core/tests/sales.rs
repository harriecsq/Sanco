mod common;

use common::{TestPeer, pull};
use sanco_core::domain::bd::{self, BdInput, BdLineInput, BdStatus};
use sanco_core::domain::bookings::{
    self, BookingAction, BookingInput, BookingLineInput, BookingStatus, DeliveryProgress,
    PriceSource,
};
use sanco_core::domain::boxes::Serials;
use sanco_core::domain::clients::{self, ClientInput};
use sanco_core::domain::delivery::{
    self, DeliverInput, DrLineInput, DrRequestInput, DrStatus, ReceiptKind,
};
use sanco_core::domain::inventory::{self, StockDocInput, StockKind, StockLineInput};
use sanco_core::domain::items::{self, CostSheetInput, ItemInput};
use sanco_core::domain::pricing::{self, RateInput};
use sanco_core::domain::settings;
use sanco_core::domain::users::{self, NewUser};
use sanco_core::ids::derived_id;
use sanco_core::sync::numbering::{self, ClaimReply, NumberClaim, NumberPeer, series};
use sanco_core::{Actor, Db, Error, Result};
use serde_json::json;

const TODAY: &str = "2026-10-21";

/// A peer that can't be reached.
struct Down;

impl NumberPeer for Down {
    fn claim(&self, _: &NumberClaim) -> Result<ClaimReply> {
        Err(Error::PeerUnreachable("down".into()))
    }
}

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

fn add_client(db: &Db, actor: &Actor, name: &str, tier: Option<i64>) -> String {
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::CLIENT, &id).unwrap();
    let input = ClientInput {
        id: None,
        name: name.into(),
        contact_person: String::new(),
        phone: String::new(),
        email: String::new(),
        tin: String::new(),
        address: String::new(),
        terms_days: Some(30),
        margin_tier_bp: tier,
        credit_limit_c: None,
        notes: String::new(),
        active: true,
    };
    db.write(actor.clone(), move |tx| {
        clients::create(tx, &input, &res, true)
    })
    .unwrap()
}

/// An item costing ₱100 (PHP cost sheet) at a 20% default margin: ₱125.
fn add_item(db: &Db, actor: &Actor, code: &str, tracks_expiry: bool) -> String {
    let input = ItemInput {
        id: None,
        code: code.into(),
        name: format!("Item {code}"),
        unit: "box".into(),
        category: String::new(),
        supplier_id: None,
        tracks_expiry,
        reorder_level: None,
        default_margin_bp: Some(2_000),
        notes: String::new(),
        active: true,
        serial_code: String::new(),
        serial_digits: None,
        pcs_per_box: None,
    };
    let id = db
        .write(actor.clone(), move |tx| items::save(tx, &input, true))
        .unwrap();
    let sheet = CostSheetInput {
        item_id: id.clone(),
        currency: "PHP".into(),
        supplier_price_minor: Some(10_000),
        fx_rate: String::new(),
        shipment_qty: None,
        notes: String::new(),
        lines: vec![],
    };
    db.write(actor.clone(), move |tx| items::save_cost_sheet(tx, &sheet))
        .unwrap();
    id
}

fn receive(db: &Db, actor: &Actor, item: &str, expiry: Option<&str>, qty: i64) {
    let input = StockDocInput {
        kind: StockKind::Receipt,
        date: TODAY.into(),
        location_id: warehouse(),
        to_location_id: None,
        reason: String::new(),
        notes: String::new(),
        lines: vec![StockLineInput {
            item_id: item.into(),
            expiry: expiry.map(String::from),
            qty,
            box_id: None,
            pcs_per_box: None,
        }],
        serial_year: String::new(),
    };
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::STOCK_RECEIPT, &id).unwrap();
    db.write(actor.clone(), move |tx| {
        inventory::create(tx, &input, &res, &mut Serials::none())
    })
    .unwrap();
}

fn on_hand(db: &Db, item: &str) -> i64 {
    db.read(|c| inventory::lots(c, Some(item), TODAY))
        .unwrap()
        .iter()
        .map(|l| l.qty)
        .sum()
}

fn booking_line(item: &str, qty: i64, price_c: Option<i64>) -> BookingLineInput {
    BookingLineInput {
        id: None,
        item_id: item.into(),
        qty,
        price_c,
        notes: String::new(),
    }
}

fn booking_input(client: &str, lines: Vec<BookingLineInput>) -> BookingInput {
    BookingInput {
        id: None,
        client_id: client.into(),
        date: TODAY.into(),
        vat: true,
        client_ref: "PO-77".into(),
        deliver_to: String::new(),
        notes: String::new(),
        lines,
    }
}

fn create_booking(db: &Db, actor: &Actor, input: BookingInput) -> String {
    db.read(|c| bookings::validate(c, &input)).unwrap();
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::BOOKING, &id).unwrap();
    db.write(actor.clone(), move |tx| bookings::create(tx, &input, &res))
        .unwrap()
}

fn set_status(db: &Db, actor: &Actor, id: &str, action: BookingAction) -> Result<()> {
    let id = id.to_string();
    db.write(actor.clone(), move |tx| {
        bookings::set_status(tx, &id, action, "")
    })
}

fn booking(db: &Db, id: &str) -> bookings::BookingDetail {
    db.read(|c| bookings::get(c, id)).unwrap().unwrap()
}

fn request_dr(
    db: &Db,
    actor: &Actor,
    booking_id: &str,
    lines: Vec<(String, Option<&str>, i64)>,
) -> Result<String> {
    let input = DrRequestInput {
        booking_id: booking_id.into(),
        kind: None,
        date: TODAY.into(),
        from_location_id: warehouse(),
        deliver_to: String::new(),
        notes: String::new(),
        lines: lines
            .into_iter()
            .map(|(booking_line_id, expiry, qty)| DrLineInput {
                id: None,
                booking_line_id,
                expiry: expiry.map(String::from),
                qty,
                boxes: vec![],
            })
            .collect(),
    };
    db.write(actor.clone(), move |tx| delivery::request(tx, &input))
}

/// Approves with a typed DR number (`None`: no number yet).
fn approve(
    db: &Db,
    actor: &Actor,
    peers: &[&dyn NumberPeer],
    id: &str,
    no: Option<&str>,
) -> Result<()> {
    let res = match no {
        Some(no) => Some(numbering::reserve_exact(
            db,
            peers,
            db.read(|c| delivery::number_series(c, id))?,
            no,
            id,
        )?),
        None => None,
    };
    let id = id.to_string();
    db.write(actor.clone(), move |tx| {
        delivery::approve(tx, &id, res.as_ref())
    })
}

fn deliver(db: &Db, actor: &Actor, id: &str, date: &str) -> Result<()> {
    let (id, input) = (
        id.to_string(),
        DeliverInput {
            date: date.into(),
            received_by: "Store clerk".into(),
        },
    );
    db.write(actor.clone(), move |tx| {
        delivery::deliver(tx, &id, &input, None)
    })
}

fn dr(db: &Db, id: &str) -> delivery::DrInfo {
    db.read(|c| delivery::get(c, id)).unwrap().unwrap().dr
}

fn invalid<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(Error::Invalid(msg)) => msg,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[test]
fn bd_request_to_delivered_dr() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV", true);
    let tape = add_item(&a.db, &actor, "TAPE", false);
    receive(&a.db, &actor, &glv, Some("2027-03-31"), 50);
    receive(&a.db, &actor, &glv, Some("2026-12-31"), 10);
    receive(&a.db, &actor, &tape, None, 100);

    // A prospect asks for gloves (quoted) and tape, plus something not in
    // the catalog.
    let input = BdInput {
        id: None,
        client_id: None,
        prospect_name: "Mercury Drug Cubao".into(),
        contact: "Liza 0917".into(),
        date: TODAY.into(),
        status: BdStatus::Quoted,
        follow_up: Some("2026-10-25".into()),
        vat: true,
        notes: String::new(),
        lost_reason: String::new(),
        lines: vec![
            BdLineInput {
                id: None,
                item_id: Some(glv.clone()),
                description: String::new(),
                qty: 30,
                price_c: Some(12_000),
            },
            BdLineInput {
                id: None,
                item_id: Some(tape.clone()),
                description: String::new(),
                qty: 20,
                price_c: None,
            },
            BdLineInput {
                id: None,
                item_id: None,
                description: "Wheelchair, folding".into(),
                qty: 1,
                price_c: None,
            },
        ],
    };
    a.db.read(|c| bd::validate(c, &input)).unwrap();
    let bd_id = a.db.new_id().unwrap();
    let res = numbering::reserve(&a.db, &[], series::BD, &bd_id).unwrap();
    a.db.write(actor.clone(), move |tx| bd::create(tx, &input, &res))
        .unwrap();
    let req = a.db.read(|c| bd::get(c, &bd_id)).unwrap().unwrap();
    assert_eq!(req.request.no.as_deref(), Some("BD-2026-0001"));
    assert_eq!(req.request.quoted_net_c, 30 * 12_000);
    assert_eq!(
        req.request.summary,
        "GLV ×30, TAPE ×20, Wheelchair, folding ×1"
    );

    // A prospect has to become a client first.
    let msg = invalid(a.db.read(|c| bd::check_convert(c, &bd_id, None, TODAY)));
    assert!(msg.contains("client"), "{msg}");
    let client = add_client(&a.db, &actor, "Mercury Drug Cubao", None);
    a.db.read(|c| bd::check_convert(c, &bd_id, Some(&client), TODAY))
        .unwrap();
    let bk_id = a.db.new_id().unwrap();
    let res = numbering::reserve(&a.db, &[], series::BOOKING, &bk_id).unwrap();
    let (b, c) = (bd_id.clone(), client.clone());
    a.db.write(actor.clone(), move |tx| bd::convert(tx, &b, Some(&c), &res))
        .unwrap();
    let req = a.db.read(|c| bd::get(c, &bd_id)).unwrap().unwrap().request;
    assert_eq!(req.status, BdStatus::Won);
    assert_eq!(req.client_id.as_deref(), Some(client.as_str()));
    assert_eq!(req.booking_no.as_deref(), Some("BK-2026-0001"));

    // The booking: quoted gloves at ₱120, tape at the item default ₱125.
    let det = booking(&a.db, &bk_id);
    assert_eq!(det.booking.status, BookingStatus::Draft);
    assert_eq!(det.booking.bd_no.as_deref(), Some("BD-2026-0001"));
    assert_eq!(det.lines.len(), 2);
    assert_eq!(
        (det.lines[0].price_c, det.lines[0].price_rule),
        (12_000, PriceSource::Quoted)
    );
    assert_eq!(
        (det.lines[1].price_c, det.lines[1].price_rule),
        (12_500, PriceSource::ItemDefault)
    );
    let net = 30 * 12_000 + 20 * 12_500;
    assert_eq!(det.booking.totals.net_c, net);
    assert_eq!(det.booking.totals.vat_c, net * 12 / 100);

    // No DRs before it is confirmed.
    let gl = det.lines[0].id.clone();
    let tl = det.lines[1].id.clone();
    let msg = invalid(request_dr(
        &a.db,
        &actor,
        &bk_id,
        vec![(gl.clone(), Some("2027-03-31"), 5)],
    ));
    assert!(msg.contains("confirmed"), "{msg}");
    set_status(&a.db, &actor, &bk_id, BookingAction::Confirm).unwrap();
    // A confirmed booking stays editable.
    let det = booking(&a.db, &bk_id);
    let mut edit = booking_input(&client, vec![]);
    edit.id = Some(bk_id.clone());
    edit.client_ref = "PO-78".into();
    edit.lines = det
        .lines
        .iter()
        .map(|l| BookingLineInput {
            id: Some(l.id.clone()),
            item_id: l.item_id.clone(),
            qty: l.qty,
            price_c: Some(l.price_c),
            notes: String::new(),
        })
        .collect();
    a.db.write(actor.clone(), move |tx| bookings::update(tx, &edit))
        .unwrap();
    assert_eq!(
        booking(&a.db, &bk_id).booking.client_ref.as_deref(),
        Some("PO-78")
    );

    // DR 1: 25 gloves from the 2027 lot and all the tape.
    let msg = invalid(request_dr(
        &a.db,
        &actor,
        &bk_id,
        vec![(gl.clone(), None, 5)],
    ));
    assert!(msg.contains("lot"), "{msg}");
    let msg = invalid(request_dr(
        &a.db,
        &actor,
        &bk_id,
        vec![(gl.clone(), Some("2026-12-31"), 15)],
    ));
    assert!(msg.contains("not enough GLV"), "{msg}");
    let dr1 = request_dr(
        &a.db,
        &actor,
        &bk_id,
        vec![(gl.clone(), Some("2027-03-31"), 25), (tl.clone(), None, 20)],
    )
    .unwrap();
    assert_eq!(dr(&a.db, &dr1).no, None);
    // Only 5 gloves are left to request.
    let msg = invalid(request_dr(
        &a.db,
        &actor,
        &bk_id,
        vec![(gl.clone(), Some("2026-12-31"), 6)],
    ));
    assert!(msg.contains("only 5 box of GLV"), "{msg}");
    let det = booking(&a.db, &bk_id);
    assert_eq!((det.lines[0].pending, det.lines[0].remaining), (25, 5));
    assert_eq!(det.booking.open_drs, 1);
    // A booking with DRs can't be cancelled or closed while one is open.
    invalid(set_status(&a.db, &actor, &bk_id, BookingAction::Cancel));
    invalid(set_status(&a.db, &actor, &bk_id, BookingAction::Close));

    // Approve without a number; delivering then needs one.
    invalid(deliver(&a.db, &actor, &dr1, TODAY));
    approve(&a.db, &actor, &[], &dr1, None).unwrap();
    assert!(invalid(deliver(&a.db, &actor, &dr1, TODAY)).contains("invoice number"));
    assert!(invalid(deliver(&a.db, &actor, &dr1, "2026-10-22")).contains("future"));
    let res = numbering::reserve_exact(&a.db, &[], series::INVOICE, " 0012345 ", &dr1).unwrap();
    assert!(res.manual);
    let id = dr1.clone();
    a.db.write(actor.clone(), move |tx| delivery::set_number(tx, &id, &res))
        .unwrap();
    // A typo is corrected in place; the old text is kept.
    let res = numbering::reserve_exact(&a.db, &[], series::INVOICE, "0012346", &dr1).unwrap();
    let id = dr1.clone();
    a.db.write(actor.clone(), move |tx| delivery::set_number(tx, &id, &res))
        .unwrap();
    assert_eq!(dr(&a.db, &dr1).no.as_deref(), Some("0012346"));
    a.clock.advance(60_000);
    deliver(&a.db, &actor, &dr1, TODAY).unwrap();

    let d = dr(&a.db, &dr1);
    assert_eq!(d.status, DrStatus::Delivered);
    assert!(d.no_manual && d.no_confirmed && !d.no_clash);
    assert_eq!(d.received_by.as_deref(), Some("Store clerk"));
    assert_eq!(d.totals.net_c, 25 * 12_000 + 20 * 12_500);
    assert_eq!(on_hand(&a.db, &glv), 35);
    assert_eq!(on_hand(&a.db, &tape), 80);
    let moves = a.db.read(|c| inventory::item_moves(c, &glv)).unwrap();
    assert_eq!(moves[0].kind, StockKind::Delivery);
    assert_eq!(
        (moves[0].qty, moves[0].no.as_deref()),
        (-25, Some("0012346"))
    );

    // DR 2 reuses the number: refused.
    let dr2 = request_dr(
        &a.db,
        &actor,
        &bk_id,
        vec![(gl.clone(), Some("2026-12-31"), 5)],
    )
    .unwrap();
    let msg = invalid(approve(&a.db, &actor, &[], &dr2, Some("12346")));
    assert!(msg.contains("already used"), "{msg}");
    invalid(approve(&a.db, &actor, &[], &dr2, Some("12-3")));
    approve(&a.db, &actor, &[], &dr2, Some("0012347")).unwrap();
    deliver(&a.db, &actor, &dr2, TODAY).unwrap();
    let det = booking(&a.db, &bk_id);
    assert_eq!(det.booking.progress, DeliveryProgress::Full);
    assert_eq!(det.drs.len(), 2);

    // Voiding a delivered DR puts its stock back; cancelling is for open ones.
    invalid(a.db.write(actor.clone(), {
        let id = dr2.clone();
        move |tx| delivery::cancel(tx, &id, "")
    }));
    invalid(a.db.write(actor.clone(), {
        let id = dr2.clone();
        move |tx| delivery::void(tx, &id, " ")
    }));
    a.db.write(actor.clone(), {
        let id = dr2.clone();
        move |tx| delivery::void(tx, &id, "wrong lot")
    })
    .unwrap();
    assert!(dr(&a.db, &dr2).voided);
    assert_eq!(on_hand(&a.db, &glv), 35);
    let det = booking(&a.db, &bk_id);
    assert_eq!(det.booking.progress, DeliveryProgress::Partial);
    assert_eq!(det.lines[0].remaining, 5);

    set_status(&a.db, &actor, &bk_id, BookingAction::Close).unwrap();
    assert_eq!(booking(&a.db, &bk_id).booking.status, BookingStatus::Closed);

    // Used master data can't be deleted.
    invalid(a.db.write(actor.clone(), {
        let id = glv.clone();
        move |tx| items::delete(tx, &id)
    }));
    invalid(a.db.write(actor.clone(), {
        let id = client.clone();
        move |tx| clients::delete(tx, &id)
    }));
}

#[test]
fn booking_prices_follow_resolution_and_keep_their_rule() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV", false);
    let tape = add_item(&a.db, &actor, "TAPE", false);
    let mask = add_item(&a.db, &actor, "MASK", false);
    // 50% tier: ₱100 → ₱200.
    let client = add_client(&a.db, &actor, "Watsons", Some(5_000));
    let rate = RateInput {
        client_id: client.clone(),
        item_id: glv.clone(),
        price_c: Some(15_000),
        vat: true,
        notes: String::new(),
    };
    a.db.write(actor.clone(), move |tx| pricing::save_rate(tx, &rate))
        .unwrap();

    let id = create_booking(
        &a.db,
        &actor,
        booking_input(
            &client,
            vec![
                booking_line(&glv, 2, None),
                booking_line(&tape, 3, None),
                booking_line(&mask, 4, Some(9_000)),
            ],
        ),
    );
    let det = booking(&a.db, &id);
    let got: Vec<_> = det
        .lines
        .iter()
        .map(|l| (l.price_c, l.price_rule))
        .collect();
    assert_eq!(
        got,
        [
            (15_000, PriceSource::Manual),
            (20_000, PriceSource::ClientTier),
            (9_000, PriceSource::Custom),
        ]
    );
    assert!(det.lines[2].below_cost);
    assert_eq!(det.booking.no.as_deref(), Some("BK-2026-0001"));

    // The rate changes; re-saving the draft with the same prices keeps
    // each line's rule, and a removed line goes away.
    let rate = RateInput {
        client_id: client.clone(),
        item_id: glv.clone(),
        price_c: Some(16_000),
        vat: true,
        notes: String::new(),
    };
    a.db.write(actor.clone(), move |tx| pricing::save_rate(tx, &rate))
        .unwrap();
    let mut edit = booking_input(&client, vec![]);
    edit.id = Some(id.clone());
    edit.vat = false;
    edit.lines = det.lines[..2]
        .iter()
        .map(|l| BookingLineInput {
            id: Some(l.id.clone()),
            item_id: l.item_id.clone(),
            qty: l.qty,
            price_c: Some(l.price_c),
            notes: String::new(),
        })
        .collect();
    a.db.write(actor.clone(), move |tx| bookings::update(tx, &edit))
        .unwrap();
    let det = booking(&a.db, &id);
    assert_eq!(det.lines.len(), 2);
    assert_eq!(
        (det.lines[0].price_c, det.lines[0].price_rule),
        (15_000, PriceSource::Manual)
    );
    assert_eq!(det.booking.totals.vat_c, 0);
    assert_eq!(det.booking.totals.gross_c, 2 * 15_000 + 3 * 20_000);

    // Drafts can be deleted; confirmed bookings only cancelled.
    set_status(&a.db, &actor, &id, BookingAction::Confirm).unwrap();
    invalid(a.db.write(actor.clone(), {
        let id = id.clone();
        move |tx| bookings::delete(tx, &id)
    }));
    set_status(&a.db, &actor, &id, BookingAction::BackToDraft).unwrap();
    a.db.write(actor.clone(), {
        let id = id.clone();
        move |tx| bookings::delete(tx, &id)
    })
    .unwrap();
    assert!(a.db.read(|c| bookings::get(c, &id)).unwrap().is_none());
}

#[test]
fn automatic_dr_numbers_after_switching_mode() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    assert!(a.db.read(|c| numbering::is_manual(c, series::DR)).unwrap());
    assert!(
        !a.db
            .read(|c| numbering::is_manual(c, series::BOOKING))
            .unwrap()
    );
    invalid(a.db.write(actor.clone(), |tx| {
        settings::set_number_mode(tx, series::BOOKING, true)
    }));
    a.db.write(actor.clone(), |tx| {
        settings::set_number_mode(tx, series::DR, false)
    })
    .unwrap();
    assert!(!a.db.read(|c| numbering::is_manual(c, series::DR)).unwrap());

    let tape = add_item(&a.db, &actor, "TAPE", false);
    receive(&a.db, &actor, &tape, None, 10);
    let client = add_client(&a.db, &actor, "Watsons", None);
    let mut input = booking_input(&client, vec![booking_line(&tape, 4, None)]);
    input.vat = false;
    let bk = create_booking(&a.db, &actor, input);
    set_status(&a.db, &actor, &bk, BookingAction::Confirm).unwrap();
    let line = booking(&a.db, &bk).lines[0].id.clone();
    let id = request_dr(&a.db, &actor, &bk, vec![(line, None, 4)]).unwrap();
    assert_eq!(dr(&a.db, &id).kind, ReceiptKind::Dr);
    let res = numbering::reserve(&a.db, &[], series::DR, &id).unwrap();
    let dr_id = id.clone();
    a.db.write(actor.clone(), move |tx| {
        delivery::approve(tx, &dr_id, Some(&res))
    })
    .unwrap();
    let d = dr(&a.db, &id);
    assert_eq!(d.no.as_deref(), Some("DR-2026-0001"));
    assert!(!d.no_manual);
}

#[test]
fn offline_typed_dr_numbers_are_flagged_not_renumbered() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let tape = add_item(&a.db, &actor, "TAPE", false);
    receive(&a.db, &actor, &tape, None, 100);
    let client = add_client(&a.db, &actor, "Watsons", None);
    let bk = create_booking(
        &a.db,
        &actor,
        booking_input(&client, vec![booking_line(&tape, 50, None)]),
    );
    set_status(&a.db, &actor, &bk, BookingAction::Confirm).unwrap();
    let line = booking(&a.db, &bk).lines[0].id.clone();

    let b = TestPeer::new(dir.path(), "b", 2);
    pull(&b.db, &a.db, 100);
    let ben = users::create_user(
        &a.db,
        actor.clone(),
        NewUser {
            username: "ben".into(),
            display_name: "Ben".into(),
            password: "secret1".into(),
            role_ids: vec![],
        },
    )
    .unwrap();
    pull(&b.db, &a.db, 100);
    let ben = Actor::user(ben);

    // Both PCs, offline from each other, type booklet number 500.
    let da = request_dr(&a.db, &actor, &bk, vec![(line.clone(), None, 10)]).unwrap();
    approve(&a.db, &actor, &[&Down], &da, Some("500")).unwrap();
    b.clock.advance(1_000);
    let db_ = request_dr(&b.db, &ben, &bk, vec![(line.clone(), None, 10)]).unwrap();
    approve(&b.db, &ben, &[&Down], &db_, Some("500")).unwrap();
    assert!(!dr(&a.db, &da).no_confirmed);

    pull(&a.db, &b.db, 100);
    pull(&b.db, &a.db, 100);
    // Nobody renumbers a typed number; B (the later one) flags it.
    assert!(
        numbering::resolve_clashes(&a.db, &[&b.db])
            .unwrap()
            .is_empty()
    );
    assert!(
        numbering::resolve_clashes(&b.db, &[&a.db])
            .unwrap()
            .is_empty()
    );
    assert!(
        numbering::resolve_clashes(&b.db, &[&a.db])
            .unwrap()
            .is_empty()
    );
    pull(&a.db, &b.db, 100);
    pull(&b.db, &a.db, 100);
    assert_eq!(a.db.dump().unwrap(), b.db.dump().unwrap());

    for id in [&da, &db_] {
        let d = dr(&a.db, id);
        assert_eq!(d.no.as_deref(), Some("500"));
        assert!(d.no_clash, "both show the clash");
    }
    let dump = a.db.dump().unwrap();
    let mut todos: Vec<_> = dump["todos"]
        .iter()
        .filter(|t| t["ref_id"] == json!(db_))
        .map(|t| t["title"].as_str().unwrap().to_string())
        .collect();
    todos.sort();
    assert_eq!(
        todos,
        ["Number 500 is used twice", "Number 500 is used twice"]
    );

    // Correcting one clears the clash.
    let res = numbering::reserve_exact(&b.db, &[&a.db], series::INVOICE, "501", &db_).unwrap();
    assert!(res.confirmed());
    let id = db_.clone();
    b.db.write(ben.clone(), move |tx| delivery::set_number(tx, &id, &res))
        .unwrap();
    pull(&a.db, &b.db, 100);
    assert!(!dr(&a.db, &da).no_clash);
    assert!(!dr(&a.db, &db_).no_clash);
}

#[test]
fn dr_lines_stay_editable_even_after_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV", true);
    let tape = add_item(&a.db, &actor, "TAPE", false);
    receive(&a.db, &actor, &glv, Some("2027-03-31"), 50);
    receive(&a.db, &actor, &glv, Some("2026-12-31"), 10);
    receive(&a.db, &actor, &tape, None, 100);
    let client = add_client(&a.db, &actor, "Watsons", None);
    let bk = create_booking(
        &a.db,
        &actor,
        booking_input(
            &client,
            vec![booking_line(&glv, 60, None), booking_line(&tape, 30, None)],
        ),
    );
    set_status(&a.db, &actor, &bk, BookingAction::Confirm).unwrap();
    let det = booking(&a.db, &bk);
    let (gl, tl) = (det.lines[0].id.clone(), det.lines[1].id.clone());
    let id = request_dr(
        &a.db,
        &actor,
        &bk,
        vec![(gl.clone(), Some("2027-03-31"), 20)],
    )
    .unwrap();

    let edit = |lines: Vec<(Option<String>, String, Option<&str>, i64)>| DrRequestInput {
        booking_id: bk.clone(),
        kind: None,
        date: TODAY.into(),
        from_location_id: warehouse(),
        deliver_to: "Makati".into(),
        notes: String::new(),
        lines: lines
            .into_iter()
            .map(|(id, booking_line_id, expiry, qty)| DrLineInput {
                id,
                booking_line_id,
                expiry: expiry.map(String::from),
                qty,
                boxes: vec![],
            })
            .collect(),
    };
    let save = |input: DrRequestInput, done: Option<DeliverInput>| {
        let id = id.clone();
        a.db.write(actor.clone(), move |tx| {
            delivery::update(tx, &id, &input, done.as_ref())
        })
    };
    let line_id = |n: usize| {
        a.db.read(|c| delivery::get(c, &id)).unwrap().unwrap().lines[n]
            .id
            .clone()
    };

    // Open DR: change qty and lot, add lines. Its own qty doesn't count
    // against what's left on the booking.
    let first = line_id(0);
    save(
        edit(vec![
            (Some(first.clone()), gl.clone(), Some("2026-12-31"), 10),
            (None, gl.clone(), Some("2027-03-31"), 30),
            (None, tl.clone(), None, 30),
        ]),
        None,
    )
    .unwrap();
    let det = a.db.read(|c| delivery::get(c, &id)).unwrap().unwrap();
    assert_eq!(det.lines.len(), 3);
    assert_eq!(det.lines[0].id, first, "kept lines keep their id");
    assert_eq!(det.dr.deliver_to.as_deref(), Some("Makati"));
    let msg = invalid(save(
        edit(vec![(None, gl.clone(), Some("2027-03-31"), 61)]),
        None,
    ));
    assert!(msg.contains("only 60"), "{msg}");

    approve(&a.db, &actor, &[], &id, Some("700")).unwrap();
    deliver(&a.db, &actor, &id, TODAY).unwrap();
    assert_eq!(on_hand(&a.db, &glv), 20);
    assert_eq!(on_hand(&a.db, &tape), 70);

    // Delivered DR: fewer gloves and no tape: that stock comes back.
    let (l0, l1) = (line_id(0), line_id(1));
    save(
        edit(vec![
            (Some(l0), gl.clone(), Some("2026-12-31"), 10),
            (Some(l1.clone()), gl.clone(), Some("2027-03-31"), 25),
        ]),
        Some(DeliverInput {
            date: "2026-10-20".into(),
            received_by: "Liza".into(),
        }),
    )
    .unwrap();
    assert_eq!(on_hand(&a.db, &glv), 25);
    assert_eq!(on_hand(&a.db, &tape), 100);
    let d = dr(&a.db, &id);
    assert_eq!(d.status, DrStatus::Delivered);
    assert_eq!(
        (d.delivered_date.as_deref(), d.received_by.as_deref()),
        (Some("2026-10-20"), Some("Liza"))
    );
    // The 2027 lot has 25 left plus the DR's own 25: 51 is too many.
    let msg = invalid(save(
        edit(vec![(Some(l1.clone()), gl.clone(), Some("2027-03-31"), 51)]),
        None,
    ));
    assert!(msg.contains("not enough GLV"), "{msg}");
    save(
        edit(vec![(Some(l1), gl.clone(), Some("2027-03-31"), 40)]),
        None,
    )
    .unwrap();
    assert_eq!(on_hand(&a.db, &glv), 20);

    // A booking line on a DR can't be removed from the booking.
    let det = booking(&a.db, &bk);
    let mut b = booking_input(&client, vec![]);
    b.id = Some(bk.clone());
    b.lines = vec![BookingLineInput {
        id: Some(tl.clone()),
        item_id: tape.clone(),
        qty: det.lines[1].qty,
        price_c: Some(det.lines[1].price_c),
        notes: String::new(),
    }];
    let msg = invalid(a.db.write(actor.clone(), move |tx| bookings::update(tx, &b)));
    assert!(msg.contains("GLV is on a DR"), "{msg}");
}

/// The receipt as an edit input, of another type.
fn as_kind(db: &Db, id: &str, kind: ReceiptKind) -> DrRequestInput {
    let d = db.read(|c| delivery::get(c, id)).unwrap().unwrap();
    DrRequestInput {
        booking_id: d.dr.booking_id.clone().unwrap(),
        kind: Some(kind),
        date: d.dr.date.clone(),
        from_location_id: d.dr.from_location_id.clone().unwrap(),
        deliver_to: String::new(),
        notes: String::new(),
        lines: d
            .lines
            .iter()
            .map(|l| DrLineInput {
                id: Some(l.id.clone()),
                booking_line_id: l.booking_line_id.clone().unwrap(),
                expiry: l.expiry.clone(),
                qty: l.qty,
                boxes: vec![],
            })
            .collect(),
    }
}

fn switch(db: &Db, actor: &Actor, id: &str, kind: ReceiptKind) -> Result<()> {
    let (id, input) = (id.to_string(), as_kind(db, id, kind));
    db.write(actor.clone(), move |tx| delivery::update(tx, &id, &input, None))
}

fn typed(db: &Db, actor: &Actor, id: &str, no: &str) -> Result<()> {
    let series = db.read(|c| delivery::number_series(c, id))?;
    let res = numbering::reserve_exact(db, &[], series, no, id)?;
    let id = id.to_string();
    db.write(actor.clone(), move |tx| delivery::set_number(tx, &id, &res))
}

#[test]
fn receipts_are_drs_or_invoices() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let tape = add_item(&a.db, &actor, "TAPE", false);
    receive(&a.db, &actor, &tape, None, 20);
    let client = add_client(&a.db, &actor, "Watsons", None);
    let bk = create_booking(
        &a.db,
        &actor,
        booking_input(&client, vec![booking_line(&tape, 10, None)]),
    );
    set_status(&a.db, &actor, &bk, BookingAction::Confirm).unwrap();
    let line = booking(&a.db, &bk).lines[0].id.clone();

    // The booking has VAT: an invoice by default. 2 × ₱125 + 12%.
    let inv = request_dr(&a.db, &actor, &bk, vec![(line.clone(), None, 2)]).unwrap();
    let d = dr(&a.db, &inv);
    assert_eq!((d.kind, d.vat, d.no_series.as_str()), (ReceiptKind::Invoice, true, "SI"));
    assert_eq!((d.totals.net_c, d.totals.gross_c), (25_000, 28_000));
    // Picked as a DR: no VAT.
    let mut input = as_kind(&a.db, &inv, ReceiptKind::Dr);
    input.lines[0].id = None;
    let drid = a
        .db
        .write(actor.clone(), move |tx| delivery::request(tx, &input))
        .unwrap();
    let d = dr(&a.db, &drid);
    assert_eq!((d.kind, d.vat, d.no_series.as_str()), (ReceiptKind::Dr, false, "DR"));
    assert_eq!(d.totals.gross_c, 25_000);
    assert!(a.db.read(|c| numbering::is_manual(c, series::INVOICE)).unwrap());

    // Each booklet has its own numbers: DR 700 and invoice 700 don't clash.
    approve(&a.db, &actor, &[], &inv, None).unwrap();
    typed(&a.db, &actor, &inv, "700").unwrap();
    typed(&a.db, &actor, &drid, "700").unwrap();
    for id in [&inv, &drid] {
        let d = dr(&a.db, id);
        assert_eq!(d.no.as_deref(), Some("700"));
        assert!(!d.no_clash);
    }
    let dump = a.db.dump().unwrap();
    let series_of = |id: &str| {
        dump["doc_numbers"]
            .iter()
            .find(|n| n["ref_id"] == json!(id))
            .unwrap()["series"]
            .clone()
    };
    assert_eq!(series_of(&inv), json!("SI"));
    assert_eq!(series_of(&drid), json!("DR"));

    // Switching type before delivery frees the number.
    switch(&a.db, &actor, &inv, ReceiptKind::Dr).unwrap();
    let d = dr(&a.db, &inv);
    assert_eq!((d.kind, d.no.as_deref(), d.totals.gross_c), (ReceiptKind::Dr, None, 25_000));
    assert_eq!(d.status, DrStatus::Approved);
    // Invoice 700 can go on another invoice now.
    let other = request_dr(&a.db, &actor, &bk, vec![(line.clone(), None, 1)]).unwrap();
    typed(&a.db, &actor, &other, "700").unwrap();
    assert!(!dr(&a.db, &other).no_clash);
    // DR 700 is still taken by the other DR.
    assert!(invalid(typed(&a.db, &actor, &inv, "700")).contains("already used"));
    // Back to an invoice: it takes a new invoice number in the same row.
    switch(&a.db, &actor, &inv, ReceiptKind::Invoice).unwrap();
    assert_eq!(dr(&a.db, &inv).no, None);
    typed(&a.db, &actor, &inv, "701").unwrap();
    assert_eq!(dr(&a.db, &inv).no.as_deref(), Some("701"));
    assert_eq!(dr(&a.db, &inv).totals.gross_c, 28_000);

    // Delivered: the type is fixed.
    deliver(&a.db, &actor, &inv, TODAY).unwrap();
    let msg = invalid(switch(&a.db, &actor, &inv, ReceiptKind::Dr));
    assert!(msg.contains("can't change type"), "{msg}");
    approve(&a.db, &actor, &[], &drid, None).unwrap();
    deliver(&a.db, &actor, &drid, TODAY).unwrap();

    // Receivables tell them apart.
    let cb = a
        .db
        .read(|c| sanco_core::domain::receivables::client_balance(c, &client, TODAY))
        .unwrap();
    assert_eq!((cb.open_drs, cb.open_invoices), (1, 1));
    assert_eq!(cb.outstanding_c, 28_000 + 25_000);
    let mut kinds: Vec<_> = a
        .db
        .read(|c| sanco_core::domain::receivables::client_drs(c, &client, TODAY))
        .unwrap()
        .into_iter()
        .map(|d| (d.kind, d.no))
        .collect();
    kinds.sort_by_key(|k| k.0 == ReceiptKind::Invoice);
    assert_eq!(
        kinds,
        [
            (ReceiptKind::Dr, Some("700".to_string())),
            (ReceiptKind::Invoice, Some("701".to_string()))
        ]
    );
    // The stock moves show which.
    let moves = a.db.read(|c| inventory::item_moves(c, &tape)).unwrap();
    let out: Vec<_> = moves
        .iter()
        .filter(|m| m.kind == StockKind::Delivery)
        .map(|m| (m.no.clone(), m.invoice))
        .collect();
    assert_eq!(out.len(), 2);
    assert!(out.contains(&(Some("701".into()), true)));
    assert!(out.contains(&(Some("700".into()), false)));
}

#[test]
fn invoices_from_before_m12_keep_their_dr_number() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let tape = add_item(&a.db, &actor, "TAPE", false);
    receive(&a.db, &actor, &tape, None, 20);
    let client = add_client(&a.db, &actor, "Watsons", None);
    let bk = create_booking(
        &a.db,
        &actor,
        booking_input(&client, vec![booking_line(&tape, 10, None)]),
    );
    set_status(&a.db, &actor, &bk, BookingAction::Confirm).unwrap();
    let line = booking(&a.db, &bk).lines[0].id.clone();
    let old = request_dr(&a.db, &actor, &bk, vec![(line, None, 2)]).unwrap();
    // A VAT DR numbered from the DR booklet before M12.
    let res = numbering::reserve_exact(&a.db, &[], series::DR, "300", &old).unwrap();
    a.db.write(actor.clone(), move |tx| {
        numbering::assign(tx, &res, "delivery_receipts").map(drop)
    })
    .unwrap();
    let d = dr(&a.db, &old);
    assert_eq!(d.kind, ReceiptKind::Invoice);
    assert_eq!((d.no.as_deref(), d.no_series.as_str()), (Some("300"), "DR"));
    // Corrections stay in the DR booklet.
    typed(&a.db, &actor, &old, "301").unwrap();
    assert_eq!(dr(&a.db, &old).no.as_deref(), Some("301"));
    // As a DR it keeps that number.
    switch(&a.db, &actor, &old, ReceiptKind::Dr).unwrap();
    let d = dr(&a.db, &old);
    assert_eq!((d.kind, d.no.as_deref()), (ReceiptKind::Dr, Some("301")));
}
