mod common;

use common::{TestPeer, pull};
use sanco_core::domain::clients::{self, ClientInput};
use sanco_core::domain::delivery::{
    self, DeliverInput, DrInput, DrLineInput, DrStatus, ReceiptKind,
};
use sanco_core::domain::inventory::{self, StockDocInput, StockKind, StockLineInput};
use sanco_core::domain::items::{self, CostSheetInput, ItemInput};
use sanco_core::domain::lots;
use sanco_core::domain::pricing::{self, PriceSource, RateInput};
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
    receive_on(db, actor, TODAY, item, expiry, qty);
}

/// A stock receipt on `date`, which picks its lot.
fn receive_on(db: &Db, actor: &Actor, date: &str, item: &str, expiry: Option<&str>, qty: i64) {
    let input = StockDocInput {
        kind: StockKind::Receipt,
        date: date.into(),
        location_id: warehouse(),
        to_location_id: None,
        reason: String::new(),
        notes: String::new(),
        lines: vec![StockLineInput {
            item_id: item.into(),
            expiry: expiry.map(String::from),
            qty,
            lot_id: None,
            serial_from: None,
            serial_to: None,
        }],
    };
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::STOCK_RECEIPT, &id).unwrap();
    db.write(actor.clone(), move |tx| inventory::create(tx, &input, &res))
        .unwrap();
}

/// The lot an item arrived in during `month` (`YYYY-MM`).
fn lot(item: &str, month: &str) -> Option<String> {
    Some(lots::lot_id(item, month))
}

/// This month's lot (what `receive` makes).
fn lot_now(item: &str) -> Option<String> {
    lot(item, lots::month_of(TODAY))
}

fn line(item: &str, qty: i64, price_c: Option<i64>, lot_id: Option<String>) -> DrLineInput {
    DrLineInput {
        id: None,
        item_id: item.into(),
        qty,
        price_c,
        lot_id,
        notes: String::new(),
    }
}

/// A receipt of `kind` for the client, from the warehouse, dated today.
fn input(client: &str, kind: ReceiptKind, lines: Vec<DrLineInput>) -> DrInput {
    DrInput {
        client_id: client.into(),
        kind,
        date: TODAY.into(),
        from_location_id: warehouse(),
        client_ref: "PO-77".into(),
        deliver_to: String::new(),
        notes: String::new(),
        lines,
    }
}

/// Enters a new receipt as the app does: checked first, then its typed
/// number (`no`) is claimed with `peers`, then written.
fn enter_with(
    db: &Db,
    actor: &Actor,
    peers: &[&dyn NumberPeer],
    input: DrInput,
    no: Option<&str>,
) -> Result<String> {
    db.read(|c| delivery::validate(c, &input))?;
    let id = db.new_id().unwrap();
    let res = match no {
        Some(no) => Some(numbering::reserve_exact(
            db,
            peers,
            input.kind.series(),
            no,
            &id,
        )?),
        None => None,
    };
    let new_id = id.clone();
    db.write(actor.clone(), move |tx| {
        delivery::create(tx, &new_id, &input, res.as_ref())
    })
}

fn enter(db: &Db, actor: &Actor, input: DrInput, no: Option<&str>) -> Result<String> {
    enter_with(db, actor, &[], input, no)
}

/// The receipt as an edit input.
fn as_input(db: &Db, id: &str) -> DrInput {
    let d = db.read(|c| delivery::get(c, id)).unwrap().unwrap();
    DrInput {
        client_id: d.dr.client_id.clone().unwrap(),
        kind: d.dr.kind,
        date: d.dr.date.clone(),
        from_location_id: d.dr.from_location_id.clone().unwrap(),
        client_ref: d.dr.client_ref.clone().unwrap_or_default(),
        deliver_to: d.dr.deliver_to.clone().unwrap_or_default(),
        notes: d.dr.notes.clone().unwrap_or_default(),
        lines: d
            .lines
            .iter()
            .map(|l| DrLineInput {
                id: Some(l.id.clone()),
                item_id: l.item_id.clone(),
                qty: l.qty,
                price_c: Some(l.price_c),
                lot_id: l.lot_id.clone(),
                notes: l.notes.clone().unwrap_or_default(),
            })
            .collect(),
    }
}

/// Saves an edit as the app does (a typed number is claimed first).
fn save(
    db: &Db,
    actor: &Actor,
    id: &str,
    input: DrInput,
    done: Option<DeliverInput>,
    no: Option<&str>,
) -> Result<()> {
    let today = sanco_core::time::manila_date_string(db.now_ms());
    db.read(|c| delivery::check_update(c, id, &input, done.as_ref(), no.is_some(), &today))?;
    let plan = db.read(|c| delivery::number_plan(c, Some(id), input.kind))?;
    let res = match no {
        Some(no) => Some(numbering::reserve_exact(db, &[], plan.series, no, id)?),
        None => None,
    };
    let id = id.to_string();
    db.write(actor.clone(), move |tx| {
        delivery::update(tx, &id, &input, done.as_ref(), res.as_ref())
    })
}

/// Marks a receipt delivered on `date`, giving it `no` if typed.
fn deliver(db: &Db, actor: &Actor, id: &str, date: &str, no: Option<&str>) -> Result<()> {
    let input = DeliverInput {
        date: date.into(),
        received_by: "Store clerk".into(),
    };
    db.read(|c| delivery::check_deliver(c, id, &input, TODAY))?;
    let res = match no {
        Some(no) => Some(numbering::reserve_exact(
            db,
            &[],
            db.read(|c| delivery::number_series(c, id))?,
            no,
            id,
        )?),
        None => None,
    };
    let id = id.to_string();
    db.write(actor.clone(), move |tx| {
        delivery::deliver(tx, &id, &input, res.as_ref())
    })
}

fn act(
    db: &Db,
    actor: &Actor,
    f: impl FnOnce(&mut sanco_core::repo::WriteTx) -> Result<()> + Send + 'static,
) -> Result<()> {
    db.write(actor.clone(), f)
}

fn dr(db: &Db, id: &str) -> delivery::DrInfo {
    db.read(|c| delivery::get(c, id)).unwrap().unwrap().dr
}

fn lines_of(db: &Db, id: &str) -> Vec<delivery::DrLine> {
    db.read(|c| delivery::get(c, id)).unwrap().unwrap().lines
}

fn typed(db: &Db, actor: &Actor, id: &str, no: &str) -> Result<()> {
    let series = db.read(|c| delivery::number_series(c, id))?;
    let res = numbering::reserve_exact(db, &[], series, no, id)?;
    let id = id.to_string();
    db.write(actor.clone(), move |tx| delivery::set_number(tx, &id, &res))
}

fn on_hand(db: &Db, item: &str) -> i64 {
    db.read(|c| inventory::lots(c, Some(item), TODAY))
        .unwrap()
        .iter()
        .map(|l| l.qty)
        .sum()
}

fn invalid<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(Error::Invalid(msg)) => msg,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[test]
fn receipt_from_entry_to_delivered() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV", true);
    let tape = add_item(&a.db, &actor, "TAPE", false);
    receive(&a.db, &actor, &glv, Some("2027-03-31"), 50);
    receive_on(&a.db, &actor, "2026-09-15", &glv, Some("2026-12-31"), 10);
    receive(&a.db, &actor, &tape, None, 100);
    let client = add_client(&a.db, &actor, "Mercury Drug Cubao", None);

    // Gloves at a typed ₱120, tape at the item default ₱125. The order
    // starts as a receipt: no booking, no approval.
    let id = enter(
        &a.db,
        &actor,
        input(
            &client,
            ReceiptKind::Invoice,
            vec![
                line(&glv, 25, Some(12_000), lot_now(&glv)),
                line(&tape, 20, None, lot_now(&tape)),
            ],
        ),
        None,
    )
    .unwrap();
    let d = dr(&a.db, &id);
    assert_eq!(d.status, DrStatus::New);
    assert_eq!((d.kind, d.no.clone()), (ReceiptKind::Invoice, None));
    assert_eq!(d.client_ref.as_deref(), Some("PO-77"));
    assert_eq!(d.booking_id, None);
    let ls = lines_of(&a.db, &id);
    assert_eq!(
        (ls[0].price_c, ls[0].price_rule),
        (12_000, PriceSource::Custom)
    );
    assert_eq!(
        (ls[1].price_c, ls[1].price_rule),
        (12_500, PriceSource::ItemDefault)
    );
    // The price typed is final: no VAT on top, even for an invoice.
    let net = 25 * 12_000 + 20 * 12_500;
    assert_eq!(
        (d.totals.net_c, d.totals.vat_c, d.totals.gross_c),
        (net, 0, net)
    );
    // Entering moves no stock.
    assert_eq!(on_hand(&a.db, &glv), 60);

    // Delivering needs the number, the lots, and a date that isn't future.
    assert!(invalid(deliver(&a.db, &actor, &id, TODAY, None)).contains("invoice number"));
    assert!(invalid(deliver(&a.db, &actor, &id, "2026-10-22", Some("12345"))).contains("future"));
    let res = numbering::reserve_exact(&a.db, &[], series::INVOICE, " 0012345 ", &id).unwrap();
    assert!(res.manual);
    let rid = id.clone();
    a.db.write(actor.clone(), move |tx| {
        delivery::set_number(tx, &rid, &res)
    })
    .unwrap();
    // A typo is corrected in place.
    typed(&a.db, &actor, &id, "0012346").unwrap();
    assert_eq!(dr(&a.db, &id).no.as_deref(), Some("0012346"));
    a.clock.advance(60_000);
    deliver(&a.db, &actor, &id, TODAY, None).unwrap();
    let d = dr(&a.db, &id);
    assert_eq!(d.status, DrStatus::Delivered);
    assert!(d.no_manual && d.no_confirmed && !d.no_clash);
    assert_eq!(d.received_by.as_deref(), Some("Store clerk"));
    assert_eq!(on_hand(&a.db, &glv), 35);
    assert_eq!(on_hand(&a.db, &tape), 80);
    let moves = a.db.read(|c| inventory::item_moves(c, &glv)).unwrap();
    assert_eq!(moves[0].kind, StockKind::Delivery);
    assert_eq!(
        (moves[0].qty, moves[0].no.as_deref()),
        (-25, Some("0012346"))
    );
    assert_eq!(
        deliver(&a.db, &actor, &id, TODAY, None).map_err(|e| e.to_string()),
        Err("this invoice is already delivered".to_string())
    );

    // A second receipt can't reuse the number, nor type a bad one.
    let second = enter(
        &a.db,
        &actor,
        input(
            &client,
            ReceiptKind::Invoice,
            vec![line(&glv, 15, None, lot(&glv, "2026-09"))],
        ),
        Some("12346"),
    );
    assert!(invalid(second).contains("already used"));
    let bad = enter(
        &a.db,
        &actor,
        input(
            &client,
            ReceiptKind::Invoice,
            vec![line(&glv, 1, None, lot_now(&glv))],
        ),
        Some("12-3"),
    );
    invalid(bad);
    // It can be entered with the lot still open; the stock is checked when
    // it is delivered (the September lot holds 10, not 15).
    let second = enter(
        &a.db,
        &actor,
        input(
            &client,
            ReceiptKind::Invoice,
            vec![line(&glv, 15, None, lot(&glv, "2026-09"))],
        ),
        Some("0012347"),
    )
    .unwrap();
    assert!(invalid(deliver(&a.db, &actor, &second, TODAY, None)).contains("not enough GLV"));
    let mut open = as_input(&a.db, &second);
    open.lines[0].lot_id = None;
    save(&a.db, &actor, &second, open, None, None).unwrap();
    assert!(invalid(deliver(&a.db, &actor, &second, TODAY, None)).contains("choose the lot"));
    // A lot of another item is refused.
    let mut wrong = as_input(&a.db, &second);
    wrong.lines[0].lot_id = lot_now(&tape);
    assert!(invalid(save(&a.db, &actor, &second, wrong, None, None)).contains("doesn't hold GLV"));
    let mut fixed = as_input(&a.db, &second);
    fixed.lines[0].qty = 10;
    fixed.lines[0].lot_id = lot(&glv, "2026-09");
    save(&a.db, &actor, &second, fixed, None, None).unwrap();
    deliver(&a.db, &actor, &second, TODAY, None).unwrap();
    assert_eq!(on_hand(&a.db, &glv), 25);

    // Voiding puts the stock back; cancelling is for new ones.
    invalid(act(&a.db, &actor, {
        let id = second.clone();
        move |tx| delivery::cancel(tx, &id, "")
    }));
    invalid(act(&a.db, &actor, {
        let id = second.clone();
        move |tx| delivery::void(tx, &id, " ")
    }));
    act(&a.db, &actor, {
        let id = second.clone();
        move |tx| delivery::void(tx, &id, "wrong lot")
    })
    .unwrap();
    assert!(dr(&a.db, &second).voided);
    assert_eq!(on_hand(&a.db, &glv), 35);

    // Used master data can't be deleted.
    invalid(act(&a.db, &actor, {
        let id = glv.clone();
        move |tx| items::delete(tx, &id)
    }));
    invalid(act(&a.db, &actor, {
        let id = client.clone();
        move |tx| clients::delete(tx, &id)
    }));
}

#[test]
fn receipt_prices_follow_resolution_and_keep_their_rule() {
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

    let id = enter(
        &a.db,
        &actor,
        input(
            &client,
            ReceiptKind::Dr,
            vec![
                line(&glv, 2, None, None),
                line(&tape, 3, None, None),
                line(&mask, 4, Some(9_000), None),
            ],
        ),
        None,
    )
    .unwrap();
    let got: Vec<_> = lines_of(&a.db, &id)
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
    // Below cost shows on the detail (₱100 cost).
    assert!(lines_of(&a.db, &id)[2].below_cost);
    // No price and nothing to work it out from is refused.
    let none = a.db.read(|c| {
        delivery::validate(
            c,
            &input(
                &client,
                ReceiptKind::Dr,
                vec![line("missing", 1, None, None)],
            ),
        )
    });
    invalid(none);

    // The rate changes; re-saving with the same prices keeps each line's
    // rule, and a removed line goes away.
    let rate = RateInput {
        client_id: client.clone(),
        item_id: glv.clone(),
        price_c: Some(16_000),
        vat: true,
        notes: String::new(),
    };
    a.db.write(actor.clone(), move |tx| pricing::save_rate(tx, &rate))
        .unwrap();
    let mut edit = as_input(&a.db, &id);
    edit.lines.truncate(2);
    save(&a.db, &actor, &id, edit, None, None).unwrap();
    let ls = lines_of(&a.db, &id);
    assert_eq!(ls.len(), 2);
    assert_eq!(
        (ls[0].price_c, ls[0].price_rule),
        (15_000, PriceSource::Manual)
    );
    // Blank price on an edited line takes the new rate.
    let mut edit = as_input(&a.db, &id);
    edit.lines[0].price_c = None;
    save(&a.db, &actor, &id, edit, None, None).unwrap();
    assert_eq!(lines_of(&a.db, &id)[0].price_c, 16_000);
    assert_eq!(dr(&a.db, &id).totals.gross_c, 2 * 16_000 + 3 * 20_000);
}

#[test]
fn automatic_receipt_numbers_after_switching_mode() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    assert!(a.db.read(|c| numbering::is_manual(c, series::DR)).unwrap());
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
    let inp = input(
        &client,
        ReceiptKind::Dr,
        vec![line(&tape, 4, None, lot_now(&tape))],
    );
    // Given when it is entered.
    let id = a.db.new_id().unwrap();
    let res = numbering::reserve(&a.db, &[], series::DR, &id).unwrap();
    let (rid, inp2) = (id.clone(), inp.clone());
    a.db.write(actor.clone(), move |tx| {
        delivery::create(tx, &rid, &inp2, Some(&res))
    })
    .unwrap();
    let d = dr(&a.db, &id);
    assert_eq!(d.no.as_deref(), Some("DR-2026-0001"));
    assert!(!d.no_manual);
    deliver(&a.db, &actor, &id, TODAY, None).unwrap();
    assert_eq!(dr(&a.db, &id).status, DrStatus::Delivered);
}

#[test]
fn offline_typed_receipt_numbers_are_flagged_not_renumbered() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let tape = add_item(&a.db, &actor, "TAPE", false);
    receive(&a.db, &actor, &tape, None, 100);
    let client = add_client(&a.db, &actor, "Watsons", None);

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
    let mk = || {
        input(
            &client,
            ReceiptKind::Invoice,
            vec![line(&tape, 10, None, lot_now(&tape))],
        )
    };
    let da = enter_with(&a.db, &actor, &[&Down], mk(), Some("500")).unwrap();
    b.clock.advance(1_000);
    let db_ = enter_with(&b.db, &ben, &[&Down], mk(), Some("500")).unwrap();
    assert!(!dr(&a.db, &da).no_confirmed);

    pull(&a.db, &b.db, 100);
    pull(&b.db, &a.db, 100);
    // Nobody renumbers a typed number; both show the clash.
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
fn receipts_can_be_edited_in_any_status() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let glv = add_item(&a.db, &actor, "GLV", true);
    let tape = add_item(&a.db, &actor, "TAPE", false);
    receive(&a.db, &actor, &glv, Some("2027-03-31"), 50);
    receive_on(&a.db, &actor, "2026-09-15", &glv, Some("2026-12-31"), 10);
    receive(&a.db, &actor, &tape, None, 100);
    let client = add_client(&a.db, &actor, "Watsons", None);
    let other = add_client(&a.db, &actor, "Mercury", None);
    let oct = lot_now(&glv);
    let sep = lot(&glv, "2026-09");

    let id = enter(
        &a.db,
        &actor,
        input(
            &client,
            ReceiptKind::Dr,
            vec![line(&glv, 20, None, oct.clone())],
        ),
        None,
    )
    .unwrap();

    // New: change the quantity and lot, add a line, change the header and
    // the client.
    let first = lines_of(&a.db, &id)[0].id.clone();
    let mut edit = as_input(&a.db, &id);
    edit.client_id = other.clone();
    edit.deliver_to = "Makati".into();
    edit.lines[0].qty = 10;
    edit.lines[0].lot_id = sep.clone();
    edit.lines.push(line(&glv, 30, None, oct.clone()));
    edit.lines
        .push(line(&tape, 30, Some(10_000), lot_now(&tape)));
    save(&a.db, &actor, &id, edit, None, None).unwrap();
    let d = dr(&a.db, &id);
    assert_eq!(d.client_id.as_deref(), Some(other.as_str()));
    assert_eq!(d.deliver_to.as_deref(), Some("Makati"));
    let ls = lines_of(&a.db, &id);
    assert_eq!(ls.len(), 3);
    assert_eq!(ls[0].id, first, "kept lines keep their id");

    deliver(&a.db, &actor, &id, TODAY, Some("700")).unwrap();
    assert_eq!(on_hand(&a.db, &glv), 20);
    assert_eq!(on_hand(&a.db, &tape), 70);

    // Delivered: fewer gloves and no tape: that stock comes back. The
    // delivery date and receiver change too.
    let ls = lines_of(&a.db, &id);
    let mut edit = as_input(&a.db, &id);
    edit.lines.truncate(2);
    edit.lines[1].qty = 25;
    save(
        &a.db,
        &actor,
        &id,
        edit,
        Some(DeliverInput {
            date: "2026-10-20".into(),
            received_by: "Liza".into(),
        }),
        None,
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
    // The October lot has 25 left plus the receipt's own 25: 51 is too many.
    let mut edit = as_input(&a.db, &id);
    edit.lines[1].qty = 51;
    assert!(invalid(save(&a.db, &actor, &id, edit, None, None)).contains("not enough GLV"));
    // A delivered receipt keeps a lot on every line.
    let mut edit = as_input(&a.db, &id);
    edit.lines[1].lot_id = None;
    assert!(invalid(save(&a.db, &actor, &id, edit, None, None)).contains("choose the lot"));

    // Delivered: switch to an invoice. That frees DR 700, so the invoice
    // number is needed in the same save.
    let mut edit = as_input(&a.db, &id);
    edit.kind = ReceiptKind::Invoice;
    assert!(invalid(save(&a.db, &actor, &id, edit.clone(), None, None)).contains("invoice number"));
    save(&a.db, &actor, &id, edit, None, Some("900")).unwrap();
    let d = dr(&a.db, &id);
    assert_eq!(
        (d.kind, d.no.as_deref(), d.no_series.as_str()),
        (ReceiptKind::Invoice, Some("900"), "SI")
    );
    assert_eq!(d.status, DrStatus::Delivered);

    // Voided: still editable, no stock effect; restoring takes the stock
    // out again.
    act(&a.db, &actor, {
        let id = id.clone();
        move |tx| delivery::void(tx, &id, "wrong client")
    })
    .unwrap();
    assert_eq!(on_hand(&a.db, &glv), 60);
    let mut edit = as_input(&a.db, &id);
    edit.notes = "re-entered".into();
    edit.lines[1].qty = 5;
    save(&a.db, &actor, &id, edit, None, None).unwrap();
    assert_eq!(on_hand(&a.db, &glv), 60);
    act(&a.db, &actor, {
        let id = id.clone();
        move |tx| delivery::restore(tx, &id)
    })
    .unwrap();
    let d = dr(&a.db, &id);
    assert_eq!((d.status, d.voided), (DrStatus::Delivered, false));
    assert_eq!(on_hand(&a.db, &glv), 45);
    assert_eq!(on_hand(&a.db, &tape), 100);
    let _ = ls;

    // Cancelled: editable and restorable to New. A restore that would take
    // the stock below zero is refused.
    let c = enter(
        &a.db,
        &actor,
        input(
            &client,
            ReceiptKind::Dr,
            vec![line(&tape, 40, None, lot_now(&tape))],
        ),
        None,
    )
    .unwrap();
    act(&a.db, &actor, {
        let id = c.clone();
        move |tx| delivery::cancel(tx, &id, "client changed mind")
    })
    .unwrap();
    assert_eq!(dr(&a.db, &c).status, DrStatus::Cancelled);
    invalid(deliver(&a.db, &actor, &c, TODAY, Some("1")));
    let mut edit = as_input(&a.db, &c);
    edit.lines[0].qty = 41;
    save(&a.db, &actor, &c, edit, None, None).unwrap();
    act(&a.db, &actor, {
        let id = c.clone();
        move |tx| delivery::restore(tx, &id)
    })
    .unwrap();
    assert_eq!(dr(&a.db, &c).status, DrStatus::New);
    invalid(act(&a.db, &actor, {
        let id = c.clone();
        move |tx| delivery::restore(tx, &id)
    }));
    // Void with too little stock left: restore is refused.
    deliver(&a.db, &actor, &c, TODAY, Some("1")).unwrap();
    act(&a.db, &actor, {
        let id = c.clone();
        move |tx| delivery::void(tx, &id, "returned")
    })
    .unwrap();
    let t = enter(
        &a.db,
        &actor,
        input(
            &client,
            ReceiptKind::Dr,
            vec![line(&tape, 90, None, lot_now(&tape))],
        ),
        None,
    )
    .unwrap();
    deliver(&a.db, &actor, &t, TODAY, Some("2")).unwrap();
    let msg = invalid(act(&a.db, &actor, {
        let id = c.clone();
        move |tx| delivery::restore(tx, &id)
    }));
    assert!(msg.contains("not enough TAPE"), "{msg}");
}

#[test]
fn receipts_are_drs_or_invoices() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let tape = add_item(&a.db, &actor, "TAPE", false);
    receive(&a.db, &actor, &tape, None, 20);
    let client = add_client(&a.db, &actor, "Watsons", None);
    let mk = |kind, qty| input(&client, kind, vec![line(&tape, qty, None, lot_now(&tape))]);

    // 2 × ₱125, no VAT added on top, either way.
    let inv = enter(&a.db, &actor, mk(ReceiptKind::Invoice, 2), None).unwrap();
    let d = dr(&a.db, &inv);
    assert_eq!(
        (d.kind, d.vat, d.no_series.as_str()),
        (ReceiptKind::Invoice, true, "SI")
    );
    assert_eq!((d.totals.net_c, d.totals.gross_c), (25_000, 25_000));
    let drid = enter(&a.db, &actor, mk(ReceiptKind::Dr, 2), None).unwrap();
    let d = dr(&a.db, &drid);
    assert_eq!(
        (d.kind, d.vat, d.no_series.as_str()),
        (ReceiptKind::Dr, false, "DR")
    );
    assert!(
        a.db.read(|c| numbering::is_manual(c, series::INVOICE))
            .unwrap()
    );

    // Each booklet has its own numbers: DR 700 and invoice 700 don't clash.
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

    // Switching type frees the number.
    let mut sw = as_input(&a.db, &inv);
    sw.kind = ReceiptKind::Dr;
    save(&a.db, &actor, &inv, sw, None, None).unwrap();
    let d = dr(&a.db, &inv);
    assert_eq!(
        (d.kind, d.no.as_deref(), d.totals.gross_c),
        (ReceiptKind::Dr, None, 25_000)
    );
    assert_eq!(d.status, DrStatus::New);
    // Invoice 700 can go on another invoice now.
    let other = enter(&a.db, &actor, mk(ReceiptKind::Invoice, 1), Some("700")).unwrap();
    assert!(!dr(&a.db, &other).no_clash);
    // DR 700 is still taken by the other DR.
    assert!(invalid(typed(&a.db, &actor, &inv, "700")).contains("already used"));
    // Back to an invoice: it takes a new invoice number in the same row.
    let mut sw = as_input(&a.db, &inv);
    sw.kind = ReceiptKind::Invoice;
    save(&a.db, &actor, &inv, sw, None, None).unwrap();
    assert_eq!(dr(&a.db, &inv).no, None);
    typed(&a.db, &actor, &inv, "701").unwrap();
    assert_eq!(dr(&a.db, &inv).no.as_deref(), Some("701"));

    deliver(&a.db, &actor, &inv, TODAY, None).unwrap();
    deliver(&a.db, &actor, &drid, TODAY, None).unwrap();

    // Receivables tell them apart.
    let cb =
        a.db.read(|c| sanco_core::domain::receivables::client_balance(c, &client, TODAY))
            .unwrap();
    assert_eq!((cb.open_drs, cb.open_invoices), (1, 1));
    assert_eq!(cb.outstanding_c, 25_000 + 25_000);
    let mut kinds: Vec<_> =
        a.db.read(|c| sanco_core::domain::receivables::client_drs(c, &client, TODAY))
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
    let old = enter(
        &a.db,
        &actor,
        input(
            &client,
            ReceiptKind::Invoice,
            vec![line(&tape, 2, None, lot_now(&tape))],
        ),
        None,
    )
    .unwrap();
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
    let mut sw = as_input(&a.db, &old);
    sw.kind = ReceiptKind::Dr;
    save(&a.db, &actor, &old, sw, None, None).unwrap();
    let d = dr(&a.db, &old);
    assert_eq!((d.kind, d.no.as_deref()), (ReceiptKind::Dr, Some("301")));
}

#[test]
fn receipts_from_before_this_change_read_as_new() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let tape = add_item(&a.db, &actor, "TAPE", false);
    receive(&a.db, &actor, &tape, None, 20);
    let client = add_client(&a.db, &actor, "Watsons", None);
    let id = enter(
        &a.db,
        &actor,
        input(
            &client,
            ReceiptKind::Dr,
            vec![line(&tape, 2, None, lot_now(&tape))],
        ),
        Some("5"),
    )
    .unwrap();
    // Requested and approved were the two steps before delivery.
    for status in ["requested", "approved"] {
        let rid = id.clone();
        a.db.write(actor.clone(), move |tx| {
            tx.update("delivery_receipts", &rid, json!({ "status": status }))
                .map(drop)
        })
        .unwrap();
        assert_eq!(dr(&a.db, &id).status, DrStatus::New);
        assert_eq!(
            a.db.read(|c| sanco_core::domain::dashboard::summary(
                c,
                &sanco_core::domain::auth::Perms {
                    admin: true,
                    ..Default::default()
                },
                TODAY
            ))
            .unwrap()
            .kpis
            .iter()
            .find(|k| k.key == "drs_open")
            .unwrap()
            .value,
            1
        );
    }
    deliver(&a.db, &actor, &id, TODAY, None).unwrap();
}
