mod common;

use common::{T0, TestPeer, pull};
use sanco_core::domain::clients::{self, ClientInput};
use sanco_core::domain::conflicts;
use sanco_core::domain::delivery::{self, DeliverInput, DrInput, DrLineInput, ReceiptKind};
use sanco_core::domain::inventory::{self, StockDocInput, StockKind, StockLineInput};
use sanco_core::domain::items::{self, CostSheetInput, ItemInput};
use sanco_core::domain::lots;
use sanco_core::domain::payments::{
    self, AllocationInput, CheckInput, CheckStatus, CheckStep, CheckStepInput, PayMethod,
    PaymentInput,
};
use sanco_core::domain::receivables::{self, AgingBucket};
use sanco_core::domain::users::{self, NewUser};
use sanco_core::ids::derived_id;
use sanco_core::sync::numbering::{self, ClaimReply, NumberClaim, NumberPeer, series};
use sanco_core::time::manila_date_string;
use sanco_core::{Actor, Db, Error, Result};

const TODAY: &str = "2026-10-21";
const DAY_MS: i64 = 86_400_000;

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

fn add_client(db: &Db, actor: &Actor, name: &str) -> String {
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
        margin_tier_bp: None,
        credit_limit_c: Some(100_000),
        notes: String::new(),
        active: true,
    };
    db.write(actor.clone(), move |tx| {
        clients::create(tx, &input, &res, true)
    })
    .unwrap()
}

/// ₱100 cost at a 20% default margin: ₱125, with 100 in the warehouse.
fn add_item(db: &Db, actor: &Actor, code: &str) -> String {
    let input = ItemInput {
        id: None,
        code: code.into(),
        name: format!("Item {code}"),
        unit: "box".into(),
        category: String::new(),
        supplier_id: None,
        tracks_expiry: false,
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
    let doc = StockDocInput {
        kind: StockKind::Receipt,
        date: TODAY.into(),
        location_id: warehouse(),
        to_location_id: None,
        reason: String::new(),
        notes: String::new(),
        lines: vec![StockLineInput {
            item_id: id.clone(),
            expiry: None,
            qty: 100,
            lot_id: None,
            serial_from: None,
            serial_to: None,
        }],
    };
    let doc_id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::STOCK_RECEIPT, &doc_id).unwrap();
    db.write(actor.clone(), move |tx| inventory::create(tx, &doc, &res))
        .unwrap();
    id
}

/// What the tests order: an item for a client at a price.
struct Order {
    client: String,
    item: String,
    price_c: Option<i64>,
}

fn order(client: &str, item: &str, price_c: Option<i64>) -> Order {
    Order {
        client: client.into(),
        item: item.into(),
        price_c,
    }
}

/// An invoice of `qty` of the order's item, on 2026-07-01. `line_id`: the
/// line being edited.
fn receipt_input(order: &Order, qty: i64, line_id: Option<String>) -> DrInput {
    DrInput {
        client_id: order.client.clone(),
        kind: ReceiptKind::Invoice,
        date: "2026-07-01".into(),
        from_location_id: warehouse(),
        client_ref: String::new(),
        deliver_to: String::new(),
        notes: String::new(),
        lines: vec![DrLineInput {
            id: line_id,
            item_id: order.item.clone(),
            qty,
            price_c: order.price_c,
            lot_id: Some(lots::lot_id(&order.item, lots::month_of(TODAY))),
            notes: String::new(),
        }],
    }
}

/// Enters (typed number `no`) and delivers an invoice on `date`.
fn delivered_dr(db: &Db, actor: &Actor, order: &Order, qty: i64, no: &str, date: &str) -> String {
    let input = receipt_input(order, qty, None);
    let id = db.new_id().unwrap();
    let res = numbering::reserve_exact(db, &[], ReceiptKind::Invoice.series(), no, &id).unwrap();
    let new_id = id.clone();
    db.write(actor.clone(), move |tx| {
        delivery::create(tx, &new_id, &input, Some(&res))
    })
    .unwrap();
    let (rid, input) = (
        id.clone(),
        DeliverInput {
            date: date.into(),
            received_by: String::new(),
        },
    );
    db.write(actor.clone(), move |tx| {
        delivery::deliver(tx, &rid, &input, None)
    })
    .unwrap();
    id
}

fn pay_input(
    client: &str,
    method: PayMethod,
    amount_c: i64,
    allocs: &[(&str, i64)],
) -> PaymentInput {
    PaymentInput {
        id: None,
        client_id: client.into(),
        date: TODAY.into(),
        method,
        amount_c,
        reference: String::new(),
        or_no: String::new(),
        notes: String::new(),
        check: (method == PayMethod::Check).then(|| CheckInput {
            bank: "BPI".into(),
            check_no: "0012345".into(),
            check_date: "2026-11-15".into(),
        }),
        bank_account_id: None,
        replaces_id: None,
        allocations: allocs
            .iter()
            .map(|(dr, amount_c)| AllocationInput {
                dr_id: dr.to_string(),
                amount_c: *amount_c,
            })
            .collect(),
    }
}

fn today(db: &Db) -> String {
    manila_date_string(db.now_ms())
}

fn pay(db: &Db, actor: &Actor, peers: &[&dyn NumberPeer], input: PaymentInput) -> Result<String> {
    let t = today(db);
    db.read(|c| payments::validate(c, &input, &t))?;
    let id = db.new_id()?;
    let res = numbering::reserve(db, peers, series::PAYMENT, &id)?;
    db.write(actor.clone(), move |tx| payments::create(tx, &input, &res))
}

fn step(db: &Db, actor: &Actor, id: &str, step: CheckStep, date: &str, reason: &str) -> Result<()> {
    let (id, input) = (
        id.to_string(),
        CheckStepInput {
            step,
            date: date.into(),
            bank_account_id: None,
            reason: reason.into(),
        },
    );
    db.write(actor.clone(), move |tx| {
        payments::check_step(tx, &id, &input)
    })
}

fn balance(db: &Db, dr: &str) -> i64 {
    db.read(|c| delivery::get(c, dr))
        .unwrap()
        .unwrap()
        .dr
        .balance_c
}

fn payment(db: &Db, id: &str) -> payments::PaymentInfo {
    let t = today(db);
    db.read(|c| payments::get(c, id, &t))
        .unwrap()
        .unwrap()
        .payment
}

fn invalid<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(Error::Invalid(msg)) => msg,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[test]
fn checks_payments_and_receivables() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let tape = add_item(&a.db, &actor, "TAPE");
    let client = add_client(&a.db, &actor, "Watsons");
    let bk = order(&client, &tape, Some(14_000));

    // 4 × ₱140 = ₱560 (the typed price is final, no VAT on top), delivered Aug 1 (30 days' terms: due Aug
    // 31, 51 days overdue). 6 more today: ₱840, not due.
    let dr1 = delivered_dr(&a.db, &actor, &bk, 4, "101", "2026-08-01");
    let dr2 = delivered_dr(&a.db, &actor, &bk, 6, "102", TODAY);
    let d1 = a.db.read(|c| delivery::get(c, &dr1)).unwrap().unwrap().dr;
    assert_eq!(d1.totals.gross_c, 56_000);
    assert_eq!(d1.balance_c, 56_000);
    assert_eq!(d1.due_date.as_deref(), Some("2026-08-31"));

    let report = a.db.read(|c| receivables::report(c, TODAY)).unwrap();
    assert_eq!(report.outstanding_c, 140_000);
    assert_eq!(report.totals.d31_60_c, 56_000);
    assert_eq!(report.totals.not_due_c, 84_000);
    assert_eq!(report.drs.len(), 2);
    assert_eq!(report.drs[0].days_overdue, 51);
    assert_eq!(report.drs[0].bucket, AgingBucket::Days31To60);
    let cb = &report.clients[0];
    assert_eq!(
        (cb.balance_c, cb.overdue_c, cb.open_drs, cb.open_invoices),
        (140_000, 56_000, 0, 2)
    );
    assert_eq!(cb.oldest_overdue_days, Some(51));
    assert_eq!(cb.credit_limit_c, Some(100_000));

    // A PDC for DR 101 counts at once.
    let pdc = pay(
        &a.db,
        &actor,
        &[],
        pay_input(&client, PayMethod::Check, 56_000, &[(&dr1, 56_000)]),
    )
    .unwrap();
    assert_eq!(balance(&a.db, &dr1), 0);
    let p = payment(&a.db, &pdc);
    assert_eq!(p.no.as_deref(), Some("PAY-2026-0001"));
    assert!(p.counts);
    let check = p.check.unwrap();
    assert!(check.pdc);
    assert_eq!(check.status, CheckStatus::OnHand);
    let cb =
        a.db.read(|c| receivables::client_balance(c, &client, TODAY))
            .unwrap();
    assert_eq!(
        (cb.balance_c, cb.overdue_c, cb.uncleared_c),
        (84_000, 0, 56_000)
    );

    // Guards: no more than the payment, no more than the DR owes, only
    // delivered DRs; the DR can't be voided or cut below what's paid.
    assert!(
        invalid(pay(
            &a.db,
            &actor,
            &[],
            pay_input(&client, PayMethod::Cash, 50_000, &[(&dr2, 60_000)])
        ))
        .contains("more than the amount")
    );
    assert!(
        invalid(pay(
            &a.db,
            &actor,
            &[],
            pay_input(&client, PayMethod::Cash, 100_000, &[(&dr2, 90_000)])
        ))
        .contains("only ₱840.00 left")
    );
    assert!(
        invalid(pay(
            &a.db,
            &actor,
            &[],
            pay_input(&client, PayMethod::Cash, 100, &[(&dr1, 100)])
        ))
        .contains("only ₱0.00 left")
    );
    let id = dr1.clone();
    assert!(
        invalid(a.db.write(actor.clone(), move |tx| delivery::void(tx, &id, "wrong")))
            .contains("payments of ₱560.00")
    );
    let dl =
        a.db.read(|c| delivery::get(c, &dr1))
            .unwrap()
            .unwrap()
            .lines[0]
            .id
            .clone();
    let (id, input) = (dr1.clone(), receipt_input(&bk, 3, Some(dl)));
    assert!(
        invalid(a.db.write(actor.clone(), move |tx| delivery::update(
            tx, &id, &input, None, None
        )))
        .contains("can't go below")
    );
    let other = add_client(&a.db, &actor, "Mercury");
    assert!(
        invalid(pay(
            &a.db,
            &actor,
            &[],
            pay_input(&other, PayMethod::Cash, 100, &[(&dr2, 100)])
        ))
        .contains("another client's")
    );

    // Not before its check date; then deposited and bounced: DR 101 owes again.
    assert!(
        invalid(step(&a.db, &actor, &pdc, CheckStep::Deposit, TODAY, ""))
            .contains("before its check date")
    );
    assert!(
        invalid(step(&a.db, &actor, &pdc, CheckStep::Clear, TODAY, ""))
            .contains("can't be marked cleared")
    );
    a.clock.set(T0 + 26 * DAY_MS);
    let later = today(&a.db);
    assert_eq!(later, "2026-11-16");
    step(&a.db, &actor, &pdc, CheckStep::Deposit, &later, "").unwrap();
    assert!(invalid(step(&a.db, &actor, &pdc, CheckStep::Bounce, &later, "")).contains("why"));
    step(&a.db, &actor, &pdc, CheckStep::Bounce, &later, "DAIF").unwrap();
    assert_eq!(balance(&a.db, &dr1), 56_000);
    let p = payment(&a.db, &pdc);
    assert!(!p.counts);
    assert_eq!(p.check.as_ref().unwrap().status, CheckStatus::Bounced);
    // Undo and bounce again.
    step(&a.db, &actor, &pdc, CheckStep::Undo, "", "").unwrap();
    assert_eq!(balance(&a.db, &dr1), 0);
    step(&a.db, &actor, &pdc, CheckStep::Bounce, &later, "DAIF").unwrap();

    // Cash replaces the bounced check, paying the same DR.
    let mut cash = pay_input(&client, PayMethod::Cash, 56_000, &[(&dr1, 56_000)]);
    cash.date = later.clone();
    cash.replaces_id = Some(pdc.clone());
    let open =
        a.db.read(|c| payments::open_drs(c, &client, None, Some(&pdc)))
            .unwrap();
    assert_eq!(
        open.iter().map(|d| d.open_c).collect::<Vec<_>>(),
        [56_000, 84_000]
    );
    let repl = pay(&a.db, &actor, &[], cash.clone()).unwrap();
    assert_eq!(balance(&a.db, &dr1), 0);
    let check = payment(&a.db, &pdc).check.unwrap();
    assert_eq!(check.status, CheckStatus::Replaced);
    assert_eq!(check.replaced_by_no.as_deref(), Some("PAY-2026-0002"));
    assert!(
        invalid(step(&a.db, &actor, &pdc, CheckStep::Deposit, &later, "")).contains("replaced")
    );
    assert!(invalid(pay(&a.db, &actor, &[], cash)).contains("already replaced"));
    // Voiding the replacement brings the bounced check (and the debt) back.
    let id = repl.clone();
    a.db.write(actor.clone(), move |tx| {
        payments::void(tx, &id, "entered twice")
    })
    .unwrap();
    assert_eq!(
        payment(&a.db, &pdc).check.unwrap().status,
        CheckStatus::Bounced
    );
    assert_eq!(balance(&a.db, &dr1), 56_000);

    // ₱1,000 cash for DR 102 (₱840): ₱160 stays as credit.
    let mut cash = pay_input(&client, PayMethod::Cash, 100_000, &[(&dr2, 84_000)]);
    cash.date = later.clone();
    let c1 = pay(&a.db, &actor, &[], cash.clone()).unwrap();
    let p = payment(&a.db, &c1);
    assert_eq!((p.applied_c, p.unapplied_c), (84_000, 16_000));
    assert_eq!(p.receipts, ["Invoice 102"]);
    let cb =
        a.db.read(|c| receivables::client_balance(c, &client, &later))
            .unwrap();
    assert_eq!(
        (cb.outstanding_c, cb.credit_c, cb.balance_c),
        (56_000, 16_000, 40_000)
    );

    // Edit: amount below its allocations is refused; moving ₱560 of it to
    // DR 101 and lowering DR 102's share works.
    cash.id = Some(c1.clone());
    cash.amount_c = 50_000;
    assert!(invalid(pay_update(&a.db, &actor, cash.clone())).contains("more than the amount"));
    cash.amount_c = 100_000;
    cash.allocations = vec![
        AllocationInput {
            dr_id: dr1.clone(),
            amount_c: 56_000,
        },
        AllocationInput {
            dr_id: dr2.clone(),
            amount_c: 44_000,
        },
    ];
    pay_update(&a.db, &actor, cash.clone()).unwrap();
    assert_eq!((balance(&a.db, &dr1), balance(&a.db, &dr2)), (0, 40_000));
    // Taking DR 102 off clears its allocation row (never deleted).
    cash.allocations.truncate(1);
    pay_update(&a.db, &actor, cash.clone()).unwrap();
    assert_eq!(balance(&a.db, &dr2), 84_000);
    let row = a.db.dump().unwrap()["payment_allocations"]
        .iter()
        .find(|r| r["id"] == serde_json::json!(payments::allocation_id(&c1, &dr2)))
        .cloned()
        .unwrap();
    assert_eq!(row["amount_c"], serde_json::Value::Null);
    assert_eq!(row["_deleted"], serde_json::json!(0));
    let mut wrong = cash.clone();
    // The method can change now; a check then needs its details.
    wrong.method = PayMethod::Check;
    assert!(invalid(pay_update(&a.db, &actor, wrong)).contains("check's details"));

    // DR 102 has nothing applied now, so it can be voided.
    let id = dr2.clone();
    a.db.write(actor.clone(), move |tx| {
        delivery::void(tx, &id, "wrong client")
    })
    .unwrap();
    let report = a.db.read(|c| receivables::report(c, &later)).unwrap();
    assert_eq!(report.outstanding_c, 0);
    assert_eq!(report.credit_c, 44_000);
    let dr_payments =
        a.db.read(|c| delivery::get(c, &dr1))
            .unwrap()
            .unwrap()
            .payments;
    assert_eq!(dr_payments.len(), 3, "bounced check, void cash, cash");
    assert_eq!(dr_payments.iter().filter(|p| p.counts).count(), 1);
}

fn pay_update(db: &Db, actor: &Actor, input: PaymentInput) -> Result<()> {
    let t = today(db);
    db.read(|c| payments::validate(c, &input, &t))?;
    db.write(actor.clone(), move |tx| payments::update(tx, &input))
}

#[test]
fn invoice_price_is_final_and_matches_receivables() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let tape = add_item(&a.db, &actor, "TAPE");
    let client = add_client(&a.db, &actor, "Watsons");
    // 3 × ₱33.33 = ₱99.99 on a Sales Invoice: no VAT is added on top.
    let bk = order(&client, &tape, Some(3_333));
    let dr = delivered_dr(&a.db, &actor, &bk, 3, "7", TODAY);
    let d = a.db.read(|c| delivery::get(c, &dr)).unwrap().unwrap().dr;
    assert_eq!(
        (d.totals.net_c, d.totals.vat_c, d.totals.gross_c),
        (9_999, 0, 9_999)
    );
    assert!(d.vat, "the booking asked for a Sales Invoice");
    let report = a.db.read(|c| receivables::report(c, TODAY)).unwrap();
    assert_eq!(report.drs[0].gross_c, 9_999);
    assert_eq!(report.outstanding_c, 9_999);
}

#[test]
fn offline_payments_for_the_same_dr_show_as_overpaid() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let tape = add_item(&a.db, &actor, "TAPE");
    let client = add_client(&a.db, &actor, "Watsons");
    let bk = order(&client, &tape, Some(14_000));
    let dr = delivered_dr(&a.db, &actor, &bk, 4, "101", TODAY);
    let b = TestPeer::new(dir.path(), "b", 2);
    pull(&b.db, &a.db, 100);

    // Both PCs record the client's ₱560 while they can't reach each other.
    let pa = pay(
        &a.db,
        &actor,
        &[&Down],
        pay_input(&client, PayMethod::Cash, 56_000, &[(&dr, 56_000)]),
    )
    .unwrap();
    b.clock.advance(1_000);
    let pb = pay(
        &b.db,
        &actor,
        &[&Down],
        pay_input(&client, PayMethod::BankTransfer, 56_000, &[(&dr, 56_000)]),
    )
    .unwrap();
    pull(&a.db, &b.db, 100);
    pull(&b.db, &a.db, 100);
    numbering::resolve_clashes(&a.db, &[&b.db]).unwrap();
    numbering::resolve_clashes(&b.db, &[&a.db]).unwrap();
    pull(&a.db, &b.db, 100);
    pull(&b.db, &a.db, 100);
    assert_eq!(a.db.dump().unwrap(), b.db.dump().unwrap());
    assert!(a.db.read(|c| conflicts::list(c, true)).unwrap().is_empty());

    assert_eq!(balance(&a.db, &dr), -56_000);
    let report = a.db.read(|c| receivables::report(c, TODAY)).unwrap();
    let cb = &report.clients[0];
    assert_eq!(
        (cb.outstanding_c, cb.credit_c, cb.overpaid_drs),
        (0, 56_000, 1)
    );
    let mut nos = vec![
        payment(&a.db, &pa).no.unwrap(),
        payment(&a.db, &pb).no.unwrap(),
    ];
    nos.sort();
    assert_eq!(nos, ["PAY-2026-0001", "PAY-2026-0002"]);
}

#[test]
fn payments_edit_in_any_status_and_void_ones_restore() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let tape = add_item(&a.db, &actor, "TAPE");
    let client = add_client(&a.db, &actor, "Watsons");
    let other = add_client(&a.db, &actor, "Mercury");
    let bk = order(&client, &tape, Some(10_000));
    let dr1 = delivered_dr(&a.db, &actor, &bk, 4, "101", "2026-08-01"); // ₱400
    assert_eq!(balance(&a.db, &dr1), 40_000);

    let id = pay(
        &a.db,
        &actor,
        &[],
        pay_input(&client, PayMethod::Cash, 40_000, &[(&dr1, 40_000)]),
    )
    .unwrap();
    assert_eq!(balance(&a.db, &dr1), 0);

    // Void, then edit while void (it still doesn't count).
    let vid = id.clone();
    a.db.write(actor.clone(), move |tx| payments::void(tx, &vid, "wrong"))
        .unwrap();
    assert_eq!(balance(&a.db, &dr1), 40_000);
    let mut edit = pay_input(&client, PayMethod::Cash, 30_000, &[(&dr1, 30_000)]);
    edit.id = Some(id.clone());
    pay_update(&a.db, &actor, edit).unwrap();
    assert_eq!(balance(&a.db, &dr1), 40_000);
    assert_eq!(payment(&a.db, &id).amount_c, 30_000);

    // Restoring counts it again.
    let rid = id.clone();
    a.db.write(actor.clone(), move |tx| payments::restore(tx, &rid))
        .unwrap();
    assert_eq!(balance(&a.db, &dr1), 10_000);
    invalid(a.db.write(actor.clone(), {
        let rid = id.clone();
        move |tx| payments::restore(tx, &rid)
    }));

    // The method changes: cash becomes a check, with its details.
    let mut edit = pay_input(&client, PayMethod::Check, 30_000, &[(&dr1, 30_000)]);
    edit.id = Some(id.clone());
    pay_update(&a.db, &actor, edit).unwrap();
    let p = payment(&a.db, &id);
    assert_eq!(p.method, PayMethod::Check);
    assert_eq!(
        p.check.as_ref().map(|c| c.status),
        Some(CheckStatus::OnHand)
    );
    // And back to cash: the check details go.
    let mut edit = pay_input(&client, PayMethod::Cash, 30_000, &[(&dr1, 30_000)]);
    edit.id = Some(id.clone());
    pay_update(&a.db, &actor, edit).unwrap();
    let p = payment(&a.db, &id);
    assert_eq!((p.method, p.check.is_none()), (PayMethod::Cash, true));

    // A payment can move to another client once it has no allocations.
    let mut edit = pay_input(&other, PayMethod::Cash, 30_000, &[]);
    edit.id = Some(id.clone());
    pay_update(&a.db, &actor, edit).unwrap();
    assert_eq!(
        payment(&a.db, &id).client_id.as_deref(),
        Some(other.as_str())
    );
    assert_eq!(balance(&a.db, &dr1), 40_000);

    // A void payment can't be restored over a receipt that was paid since.
    let mut back = pay_input(&client, PayMethod::Cash, 30_000, &[(&dr1, 30_000)]);
    back.id = Some(id.clone());
    pay_update(&a.db, &actor, back).unwrap();
    let vid = id.clone();
    a.db.write(actor.clone(), move |tx| payments::void(tx, &vid, "again"))
        .unwrap();
    pay(
        &a.db,
        &actor,
        &[],
        pay_input(&client, PayMethod::Cash, 40_000, &[(&dr1, 40_000)]),
    )
    .unwrap();
    let msg = invalid(a.db.write(actor.clone(), {
        let rid = id.clone();
        move |tx| payments::restore(tx, &rid)
    }));
    assert!(msg.contains("left to pay"), "{msg}");
}
