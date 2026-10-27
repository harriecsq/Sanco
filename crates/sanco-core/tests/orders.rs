mod common;

use common::{TestPeer, pull};
use sanco_core::domain::conflicts;
use sanco_core::domain::inventory::{self, StockDocInput, StockKind, StockLineInput};
use sanco_core::domain::charge_types;
use sanco_core::domain::items::{self, CostLineInput, CostSheetInput, ItemInput};
use sanco_core::domain::pricing::ChargeMode;
use sanco_core::domain::purchasing::{
    self, PoAction, PoInput, PoLineInput, PoStatus, PoStatusInput, ReceiptInput, ReceiptLineInput,
    ReceiveProgress, TtInput,
};
use sanco_core::domain::suppliers::{self, SupplierInput};
use sanco_core::domain::users::{self, NewUser};
use sanco_core::ids::derived_id;
use sanco_core::sync::numbering::{self, series};
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

fn add_item(db: &Db, actor: &Actor, code: &str, supplier: &str, tracks_expiry: bool) -> String {
    let input = ItemInput {
        id: None,
        code: code.into(),
        name: format!("Item {code}"),
        unit: "box".into(),
        category: "Gloves".into(),
        supplier_id: Some(supplier.into()),
        tracks_expiry,
        reorder_level: None,
        default_margin_bp: Some(2000),
        notes: String::new(),
        active: true,
    };
    db.write(actor.clone(), move |tx| items::save(tx, &input, true))
        .unwrap()
}

fn po_line(item: &str, qty: i64, price: Option<i64>) -> PoLineInput {
    PoLineInput {
        id: None,
        item_id: item.into(),
        qty,
        price_minor: price,
        notes: String::new(),
    }
}

fn po_input(supplier: &str, lines: Vec<PoLineInput>) -> PoInput {
    PoInput {
        id: None,
        supplier_id: supplier.into(),
        date: TODAY.into(),
        currency: "USD".into(),
        fx_rate: "56.50".into(),
        expected_date: "2026-11-15".into(),
        supplier_ref: "PI-77".into(),
        notes: String::new(),
        lines,
    }
}

fn create_po(db: &Db, actor: &Actor, input: PoInput) -> Result<String, Error> {
    db.read(|c| purchasing::validate(c, &input, true))?;
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::PO, &id).unwrap();
    db.write(actor.clone(), move |tx| {
        purchasing::create(tx, &input, &res, true)
    })
}

fn update_po(db: &Db, actor: &Actor, input: PoInput, costs: bool) -> Result<(), Error> {
    db.write(actor.clone(), move |tx| {
        purchasing::update(tx, &input, costs)
    })
}

fn status(db: &Db, actor: &Actor, id: &str, action: PoAction) -> Result<(), Error> {
    let id = id.to_string();
    let input = PoStatusInput {
        action,
        date: String::new(),
        reason: "Supplier out of stock".into(),
    };
    db.write(actor.clone(), move |tx| {
        purchasing::set_status(tx, &id, &input)
    })
}

fn receive(db: &Db, actor: &Actor, input: ReceiptInput) -> Result<String, Error> {
    db.read(|c| purchasing::validate_receipt(c, &input, TODAY))?;
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::RECEIVING, &id).unwrap();
    db.write(actor.clone(), move |tx| {
        purchasing::receive(tx, &input, &res)
    })
}

fn receipt(po: &str, location: &str, lines: Vec<(&str, Option<&str>, i64)>) -> ReceiptInput {
    ReceiptInput {
        po_id: po.into(),
        date: TODAY.into(),
        location_id: location.into(),
        notes: String::new(),
        lines: lines
            .into_iter()
            .map(|(l, e, q)| ReceiptLineInput {
                po_line_id: l.into(),
                expiry: e.map(String::from),
                qty: q,
            })
            .collect(),
    }
}

fn detail(db: &Db, id: &str) -> purchasing::PoDetail {
    db.read(|c| purchasing::get(c, id)).unwrap().unwrap()
}

/// (location, expiry, qty) of an item's non-zero lots.
fn lots(db: &Db, item: &str) -> Vec<(String, Option<String>, i64)> {
    db.read(|c| inventory::lots(c, Some(item), TODAY))
        .unwrap()
        .into_iter()
        .map(|l| (l.location_id, l.expiry, l.qty))
        .collect()
}

fn invalid<T: std::fmt::Debug>(r: Result<T, Error>) -> String {
    match r {
        Err(Error::Invalid(msg)) => msg,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

/// The input that saves a PO as it is now, for edits.
fn as_input(d: &purchasing::PoDetail) -> PoInput {
    PoInput {
        id: Some(d.po.id.clone()),
        supplier_id: d.po.supplier_id.clone().unwrap(),
        date: d.po.date.clone(),
        currency: d.po.currency.clone(),
        fx_rate: d.po.fx_rate.clone().unwrap_or_default(),
        expected_date: d.po.expected_date.clone().unwrap_or_default(),
        supplier_ref: d.po.supplier_ref.clone().unwrap_or_default(),
        notes: String::new(),
        lines: d
            .lines
            .iter()
            .map(|l| PoLineInput {
                id: Some(l.id.clone()),
                item_id: l.item_id.clone(),
                qty: l.qty,
                price_minor: l.price_minor,
                notes: String::new(),
            })
            .collect(),
    }
}

#[test]
fn po_payments_and_receiving_into_stock() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let sup = add_supplier(&a.db, &actor);
    let glv = add_item(&a.db, &actor, "GLV-100", &sup, true);
    let tape = add_item(&a.db, &actor, "TAPE", &sup, false);

    // Validation.
    assert_eq!(
        invalid(create_po(&a.db, &actor, po_input(&sup, vec![]))),
        "add at least one item"
    );
    assert_eq!(
        invalid(create_po(
            &a.db,
            &actor,
            po_input(&sup, vec![po_line(&glv, 0, None)])
        )),
        "enter a quantity for GLV-100"
    );
    let mut bad_fx = po_input(&sup, vec![po_line(&glv, 1, None)]);
    bad_fx.fx_rate = "abc".into();
    assert!(invalid(create_po(&a.db, &actor, bad_fx)).contains("not a valid exchange rate"));

    // US$12.50 × 100 + US$3.00 × 40 = US$1,370.00; at 56.50 = ₱77,405.00.
    let po = create_po(
        &a.db,
        &actor,
        po_input(
            &sup,
            vec![
                po_line(&glv, 100, Some(1250)),
                po_line(&tape, 40, Some(300)),
            ],
        ),
    )
    .unwrap();
    let d = detail(&a.db, &po);
    assert_eq!(d.po.no.as_deref(), Some("PO-2026-0001"));
    assert_eq!(d.po.status, PoStatus::Draft);
    assert_eq!(d.po.supplier_code.as_deref(), Some("SUP-0001"));
    assert_eq!(d.po.total_minor, Some(137_000));
    assert_eq!(d.po.total_php_c, Some(7_740_500));
    assert_eq!(d.po.paid_minor, Some(0));
    let (gl, tl) = (d.lines[0].id.clone(), d.lines[1].id.clone());

    // A draft can't be received.
    assert_eq!(
        invalid(receive(
            &a.db,
            &actor,
            receipt(&po, &warehouse(), vec![(&gl, Some("2028-01-31"), 10)])
        )),
        "mark the purchase order as ordered first"
    );
    status(&a.db, &actor, &po, PoAction::Order).unwrap();
    status(&a.db, &actor, &po, PoAction::Ship).unwrap();
    let d = detail(&a.db, &po);
    assert_eq!(d.po.status, PoStatus::Shipped);
    assert_eq!(d.po.shipped_date.as_deref(), Some(TODAY));

    // TT payments: a deposit in USD and one in PHP.
    let tt = TtInput {
        id: None,
        po_id: po.clone(),
        date: TODAY.into(),
        currency: "USD".into(),
        amount_minor: 50_000,
        fx_rate: "56.00".into(),
        reference: "TT-1".into(),
        bank_account_id: None,
        notes: String::new(),
    };
    let mut no_fx = tt.clone();
    no_fx.fx_rate = String::new();
    assert_eq!(
        invalid(
            a.db.write(actor.clone(), move |tx| purchasing::save_payment(
                tx, &no_fx
            ))
        ),
        "enter the exchange rate of the transfer"
    );
    let input = tt.clone();
    let tt1 =
        a.db.write(actor.clone(), move |tx| {
            purchasing::save_payment(tx, &input)
        })
        .unwrap();
    let mut php = tt.clone();
    php.currency = "PHP".into();
    php.amount_minor = 150_000;
    php.fx_rate = String::new();
    a.db.write(actor.clone(), move |tx| purchasing::save_payment(tx, &php))
        .unwrap();
    let d = detail(&a.db, &po);
    assert_eq!(d.payments.len(), 2);
    assert_eq!(d.po.paid_minor, Some(50_000));
    assert_eq!(d.po.paid_php_c, Some(2_800_000 + 150_000));
    // Edit the USD payment.
    let mut edit = tt.clone();
    edit.id = Some(tt1.clone());
    edit.amount_minor = 60_000;
    a.db.write(actor.clone(), move |tx| purchasing::save_payment(tx, &edit))
        .unwrap();
    assert_eq!(detail(&a.db, &po).po.paid_minor, Some(60_000));

    // Receiving: expiry required for GLV, ignored for TAPE; no more than
    // ordered.
    assert_eq!(
        invalid(receive(
            &a.db,
            &actor,
            receipt(&po, &warehouse(), vec![(&gl, None, 10)])
        )),
        "enter the expiry date of GLV-100"
    );
    assert!(
        invalid(receive(
            &a.db,
            &actor,
            receipt(
                &po,
                &warehouse(),
                vec![(&gl, Some("2028-01-31"), 60), (&gl, Some("2028-02-28"), 41)]
            )
        ))
        .starts_with("only 100 of GLV-100 left to receive")
    );
    let rr1 = receive(
        &a.db,
        &actor,
        receipt(
            &po,
            &warehouse(),
            vec![(&gl, Some("2028-01-31"), 60), (&tl, Some("2030-01-01"), 40)],
        ),
    )
    .unwrap();
    let d = detail(&a.db, &po);
    assert_eq!(
        d.po.status,
        PoStatus::Arrived,
        "a receipt marks the PO arrived"
    );
    assert_eq!(d.po.arrived_date.as_deref(), Some(TODAY));
    assert_eq!(d.po.progress, ReceiveProgress::Partial);
    assert_eq!((d.lines[0].received, d.lines[0].remaining), (60, 40));
    assert_eq!((d.lines[1].received, d.lines[1].remaining), (40, 0));
    assert_eq!(d.receipts.len(), 1);
    assert_eq!(d.receipts[0].no.as_deref(), Some("RR-2026-0001"));
    assert_eq!(
        lots(&a.db, &glv),
        vec![(warehouse(), Some("2028-01-31".into()), 60)]
    );
    assert_eq!(lots(&a.db, &tape), vec![(warehouse(), None, 40)]);
    let moves = a.db.read(|c| inventory::item_moves(c, &glv)).unwrap();
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].kind, StockKind::PoReceipt);
    assert_eq!(moves[0].no.as_deref(), Some("RR-2026-0001"));
    assert_eq!(moves[0].po_id.as_deref(), Some(po.as_str()));

    // Edits keep received lines.
    let mut e = as_input(&detail(&a.db, &po));
    e.lines.remove(1);
    assert!(invalid(update_po(&a.db, &actor, e, true)).starts_with("TAPE has been received"));
    let mut e = as_input(&detail(&a.db, &po));
    e.lines[0].qty = 50;
    assert_eq!(
        invalid(update_po(&a.db, &actor, e, true)),
        "60 of GLV-100 were already received; the quantity can't be lower"
    );
    let mut e = as_input(&detail(&a.db, &po));
    e.lines[1].item_id = glv.clone();
    assert!(invalid(update_po(&a.db, &actor, e, true)).contains("can't change its item"));
    // Raising a quantity is fine; so is a user without costs saving: the
    // prices, currency and FX stay.
    let mut e = as_input(&detail(&a.db, &po));
    e.lines[0].qty = 120;
    e.currency = "PHP".into();
    e.fx_rate = String::new();
    for l in &mut e.lines {
        l.price_minor = None;
    }
    update_po(&a.db, &actor, e, false).unwrap();
    let d = detail(&a.db, &po);
    assert_eq!(d.lines[0].qty, 120);
    assert_eq!(d.lines[0].price_minor, Some(1250));
    assert_eq!(
        (d.po.currency.as_str(), d.po.fx_rate.as_deref()),
        ("USD", Some("56.5"))
    );

    // The rest goes into the Office in a second report.
    let rr2 = receive(
        &a.db,
        &actor,
        receipt(&po, &office(), vec![(&gl, Some("2028-03-31"), 60)]),
    )
    .unwrap();
    let d = detail(&a.db, &po);
    assert_eq!(d.po.progress, ReceiveProgress::Full);
    assert_eq!(d.po.received_qty, 160);

    // Cancel and back-to-draft are refused once goods came in.
    assert!(
        invalid(status(&a.db, &actor, &po, PoAction::Cancel))
            .starts_with("goods were already received")
    );
    assert_eq!(
        invalid(status(&a.db, &actor, &po, PoAction::Ship)),
        "this purchase order is arrived; it can't be changed that way"
    );

    // Voiding a report takes its stock back out, unless it was used.
    let adj = StockDocInput {
        kind: StockKind::Adjustment,
        date: TODAY.into(),
        location_id: office(),
        to_location_id: None,
        reason: "Damaged".into(),
        notes: String::new(),
        lines: vec![StockLineInput {
            item_id: glv.clone(),
            expiry: Some("2028-03-31".into()),
            qty: -5,
        }],
    };
    let id = a.db.new_id().unwrap();
    let res = numbering::reserve(&a.db, &[], series::ADJUSTMENT, &id).unwrap();
    a.db.write(actor.clone(), move |tx| inventory::create(tx, &adj, &res))
        .unwrap();
    let r = rr2.clone();
    assert!(
        invalid(
            a.db.write(actor.clone(), move |tx| purchasing::void_receipt(
                tx, &r, "Wrong"
            ))
        )
        .starts_with("can't void: not enough GLV-100")
    );
    let r = rr1.clone();
    assert_eq!(
        invalid(
            a.db.write(actor.clone(), move |tx| purchasing::void_receipt(
                tx, &r, " "
            ))
        ),
        "enter why the receiving report is voided"
    );
    let r = rr1.clone();
    a.db.write(actor.clone(), move |tx| {
        purchasing::void_receipt(tx, &r, "Wrong location")
    })
    .unwrap();
    assert_eq!(lots(&a.db, &tape), vec![]);
    let d = detail(&a.db, &po);
    assert_eq!((d.lines[0].received, d.lines[1].received), (60, 0));
    assert!(d.receipts.iter().any(|r| r.id == rr1 && r.voided));
    assert_eq!(d.po.progress, ReceiveProgress::Partial);
    // Entered again: a new number.
    let rr3 = receive(
        &a.db,
        &actor,
        receipt(
            &po,
            &office(),
            vec![(&tl, None, 40), (&gl, Some("2028-01-31"), 60)],
        ),
    )
    .unwrap();
    let r3 =
        a.db.read(|c| purchasing::get_receipt(c, &rr3))
            .unwrap()
            .unwrap();
    assert_eq!(r3.receipt.no.as_deref(), Some("RR-2026-0003"));
    assert_eq!(r3.lines.len(), 2);

    // Close and reopen.
    status(&a.db, &actor, &po, PoAction::Close).unwrap();
    assert_eq!(
        invalid(receive(
            &a.db,
            &actor,
            receipt(&po, &office(), vec![(&gl, Some("2028-01-31"), 1)])
        )),
        "this purchase order is closed; reopen it first"
    );
    status(&a.db, &actor, &po, PoAction::Reopen).unwrap();
    assert_eq!(detail(&a.db, &po).po.status, PoStatus::Arrived);

    // Cost sheets from the PO. The charge lines already on GLV's sheet
    // follow the new supplier price and ordered quantity.
    let charge = |name: &str| {
        a.db.read(charge_types::list)
            .unwrap()
            .into_iter()
            .find(|t| t.name == name)
            .unwrap()
            .id
    };
    let sheet = CostSheetInput {
        item_id: glv.clone(),
        currency: "PHP".into(),
        supplier_price_minor: Some(100),
        fx_rate: String::new(),
        shipment_qty: Some(1),
        notes: String::new(),
        lines: vec![
            CostLineInput {
                charge_type_id: charge("Freight"),
                mode: ChargeMode::Shipment,
                value: Some(1_200_000),
            },
            CostLineInput {
                charge_type_id: charge("Customs duty"),
                mode: ChargeMode::Pct,
                value: Some(1_000),
            },
        ],
    };
    a.db.write(actor.clone(), move |tx| items::save_cost_sheet(tx, &sheet))
        .unwrap();
    let p = po.clone();
    let codes =
        a.db.write(actor.clone(), move |tx| {
            purchasing::apply_to_cost_sheets(tx, &p)
        })
        .unwrap();
    assert_eq!(codes, vec!["GLV-100".to_string(), "TAPE".to_string()]);
    let landed =
        a.db.read(|c| items::landed_cost_of(c, &glv))
            .unwrap()
            .unwrap();
    assert_eq!(landed.base_c, 70_625); // US$12.50 × 56.50
    let sheet = a.db.read(|c| items::cost_sheet(c, &glv)).unwrap().unwrap();
    assert_eq!(sheet.shipment_qty, Some(120)); // the ordered quantity
    assert_eq!(sheet.currency, "USD");
    let amount = |name: &str| sheet.lines.iter().find(|l| l.name == name).unwrap().amount_c;
    assert_eq!(amount("Freight"), Some(10_000)); // ₱12,000 ÷ 120
    assert_eq!(amount("Customs duty"), Some(7_063)); // 10% of ₱706.25
    assert_eq!(landed.total_c, 70_625 + 10_000 + 7_063);
    assert!(!sheet.price_pending);
    assert_eq!(sheet.changed_by.as_deref(), Some("Ana"));
    let sp = sheet.supplier_price.clone().unwrap();
    assert_eq!((sp.po_id.as_str(), sp.price_minor, sp.qty), (po.as_str(), 1250, Some(120)));

    // A typed price leaves the PO price pending until the button copies it
    // back. A draft PO's price never counts.
    let mut typed = CostSheetInput {
        item_id: glv.clone(),
        currency: "USD".into(),
        supplier_price_minor: Some(1_300),
        fx_rate: "56.5".into(),
        shipment_qty: Some(120),
        notes: String::new(),
        lines: vec![],
    };
    let t = typed.clone();
    a.db.write(actor.clone(), move |tx| items::save_cost_sheet(tx, &t))
        .unwrap();
    let sheet = a.db.read(|c| items::cost_sheet(c, &glv)).unwrap().unwrap();
    assert!(sheet.price_pending);
    assert_eq!(sheet.lines.iter().filter(|l| l.value.is_some()).count(), 2);
    let draft = create_po(&a.db, &actor, po_input(&sup, vec![po_line(&glv, 5, Some(9_999))])).unwrap();
    let g = glv.clone();
    let used =
        a.db.write(actor.clone(), move |tx| purchasing::apply_latest_price(tx, &g))
            .unwrap();
    assert_eq!(used.po_id, po);
    a.db.write(actor.clone(), move |tx| purchasing::delete(tx, &draft))
        .unwrap();
    let sheet = a.db.read(|c| items::cost_sheet(c, &glv)).unwrap().unwrap();
    assert!(!sheet.price_pending);
    assert_eq!(sheet.supplier_price_minor, Some(1_250));
    assert_eq!(sheet.landed.unwrap().total_c, 70_625 + 10_000 + 7_063);
    // Same FX written differently is not a new rate.
    typed.supplier_price_minor = Some(1_250);
    typed.fx_rate = "56.500".into();
    typed.lines = sheet
        .lines
        .iter()
        .map(|l| CostLineInput {
            charge_type_id: l.charge_type_id.clone(),
            mode: l.mode,
            value: l.value,
        })
        .collect();
    a.db.write(actor.clone(), move |tx| items::save_cost_sheet(tx, &typed))
        .unwrap();
    assert!(!a.db.read(|c| items::cost_sheet(c, &glv)).unwrap().unwrap().price_pending);
    // An item never ordered has nothing to apply.
    let gown = add_item(&a.db, &actor, "GOWN", &sup, true);
    let sheet = a.db.read(|c| items::cost_sheet(c, &gown)).unwrap().unwrap();
    assert_eq!((sheet.supplier_price, sheet.price_pending), (None, false));
    assert!(
        invalid(a.db.write(actor.clone(), move |tx| purchasing::apply_latest_price(tx, &gown)))
            .starts_with("no placed purchase order")
    );

    // Delete guards.
    let s = sup.clone();
    assert!(
        invalid(a.db.write(actor.clone(), move |tx| suppliers::delete(tx, &s)))
            .contains("item(s) still use this supplier")
    );
    let w = warehouse();
    assert!(a.db.read(|c| inventory::location_in_use(c, &w)).unwrap());
    let p = po.clone();
    assert!(
        invalid(a.db.write(actor.clone(), move |tx| purchasing::delete(tx, &p)))
            .starts_with("only a draft purchase order can be deleted")
    );

    // A draft with a payment can't be deleted; without one it can.
    let draft = create_po(&a.db, &actor, po_input(&sup, vec![po_line(&tape, 5, None)])).unwrap();
    let d = detail(&a.db, &draft);
    assert_eq!(d.po.unpriced_lines, 1);
    let mut pay = tt.clone();
    pay.po_id = draft.clone();
    let tt_draft =
        a.db.write(actor.clone(), move |tx| purchasing::save_payment(tx, &pay))
            .unwrap();
    let p = draft.clone();
    assert!(
        invalid(a.db.write(actor.clone(), move |tx| purchasing::delete(tx, &p)))
            .starts_with("this purchase order has TT payments")
    );
    a.db.write(actor.clone(), move |tx| {
        purchasing::delete_payment(tx, &tt_draft)
    })
    .unwrap();
    let p = draft.clone();
    a.db.write(actor.clone(), move |tx| purchasing::delete(tx, &p))
        .unwrap();
    assert!(a.db.read(|c| purchasing::get(c, &draft)).unwrap().is_none());
    assert_eq!(a.db.read(purchasing::list).unwrap().len(), 1);
    assert_eq!(a.db.read(purchasing::list_payments).unwrap().len(), 2);
    assert_eq!(a.db.read(purchasing::list_receipts).unwrap().len(), 3);

    // Stock documents can't pretend to be PO receipts.
    let fake = StockDocInput {
        kind: StockKind::PoReceipt,
        date: TODAY.into(),
        location_id: office(),
        to_location_id: None,
        reason: String::new(),
        notes: String::new(),
        lines: vec![],
    };
    assert!(a.db.read(|c| inventory::validate(c, &fake, TODAY)).is_err());
}

#[test]
fn two_offline_pcs_receive_the_same_po_line() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    let actor = setup(&a.db);
    let sup = add_supplier(&a.db, &actor);
    let glv = add_item(&a.db, &actor, "GLV-100", &sup, true);
    let po = create_po(
        &a.db,
        &actor,
        po_input(&sup, vec![po_line(&glv, 10, Some(100))]),
    )
    .unwrap();
    status(&a.db, &actor, &po, PoAction::Order).unwrap();
    pull(&b.db, &a.db, 500);
    let line = detail(&b.db, &po).lines[0].id.clone();

    // Offline from each other, both receive the full quantity.
    receive(
        &a.db,
        &actor,
        receipt(&po, &warehouse(), vec![(&line, Some("2028-01-31"), 10)]),
    )
    .unwrap();
    receive(
        &b.db,
        &actor,
        receipt(&po, &office(), vec![(&line, Some("2028-01-31"), 10)]),
    )
    .unwrap();
    pull(&a.db, &b.db, 500);
    pull(&b.db, &a.db, 500);

    for db in [&a.db, &b.db] {
        let d = detail(db, &po);
        assert_eq!((d.lines[0].received, d.lines[0].remaining), (20, -10));
        assert_eq!(d.po.status, PoStatus::Arrived);
        assert_eq!(d.receipts.len(), 2);
        assert_eq!(lots(db, &glv).iter().map(|l| l.2).sum::<i64>(), 20);
        assert!(db.read(|c| conflicts::list(c, true)).unwrap().is_empty());
    }
}
