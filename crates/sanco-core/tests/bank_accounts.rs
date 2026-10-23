mod common;

use common::TestPeer;
use sanco_core::domain::bank_accounts::{self, BankAccountInput, BankMoveKind};
use sanco_core::domain::clients::{self, ClientInput};
use sanco_core::domain::items::{self, ItemInput};
use sanco_core::domain::payments::{
    self, CheckInput, CheckStatus, CheckStep, CheckStepInput, PayMethod, PaymentInput,
};
use sanco_core::domain::purchasing::{self, PoInput, PoLineInput, TtInput};
use sanco_core::domain::suppliers::{self, SupplierInput};
use sanco_core::domain::users::{self, NewUser};
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

fn invalid<T: std::fmt::Debug>(r: Result<T, Error>) -> String {
    match r {
        Err(Error::Invalid(m)) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

fn account_input(name: &str) -> BankAccountInput {
    BankAccountInput {
        id: None,
        name: name.into(),
        bank: "BDO".into(),
        account_no: "0012-3456-78".into(),
        currency: "php".into(),
        notes: String::new(),
        active: true,
    }
}

fn add_account(db: &Db, actor: &Actor, name: &str) -> String {
    let input = account_input(name);
    db.write(actor.clone(), move |tx| bank_accounts::save(tx, &input))
        .unwrap()
}

fn add_client(db: &Db, actor: &Actor) -> String {
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::CLIENT, &id).unwrap();
    let input = ClientInput {
        id: None,
        name: "Mercury Drug".into(),
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

fn payment(
    client: &str,
    method: PayMethod,
    amount_c: i64,
    account: Option<&str>,
) -> PaymentInput {
    PaymentInput {
        id: None,
        client_id: client.into(),
        date: TODAY.into(),
        method,
        amount_c,
        reference: "REF-9".into(),
        or_no: String::new(),
        notes: String::new(),
        check: (method == PayMethod::Check).then(|| CheckInput {
            bank: "BPI".into(),
            check_no: "0012345".into(),
            check_date: TODAY.into(),
        }),
        bank_account_id: account.map(String::from),
        replaces_id: None,
        allocations: vec![],
    }
}

fn pay(db: &Db, actor: &Actor, input: PaymentInput) -> Result<String, Error> {
    db.read(|c| payments::validate(c, &input, TODAY))?;
    let id = db.new_id()?;
    let res = numbering::reserve(db, &[], series::PAYMENT, &id)?;
    db.write(actor.clone(), move |tx| payments::create(tx, &input, &res))
}

fn step(db: &Db, actor: &Actor, id: &str, step: CheckStep, account: Option<&str>) -> Result<(), Error> {
    let (id, input) = (
        id.to_string(),
        CheckStepInput {
            step,
            date: TODAY.into(),
            bank_account_id: account.map(String::from),
            reason: "DAIF".into(),
        },
    );
    db.write(actor.clone(), move |tx| {
        payments::check_step(tx, &id, &input)
    })
}

fn add_po(db: &Db, actor: &Actor) -> String {
    let supplier_input = SupplierInput {
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
    let sid = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::SUPPLIER, &sid).unwrap();
    let supplier = db
        .write(actor.clone(), move |tx| {
            suppliers::create(tx, &supplier_input, &res)
        })
        .unwrap();
    let item_input = ItemInput {
        id: None,
        code: "GL-1".into(),
        name: "Gloves".into(),
        unit: "box".into(),
        category: "Gloves".into(),
        supplier_id: Some(supplier.clone()),
        tracks_expiry: false,
        reorder_level: None,
        default_margin_bp: None,
        notes: String::new(),
        active: true,
    };
    let item = db
        .write(actor.clone(), move |tx| items::save(tx, &item_input, true))
        .unwrap();
    let input = PoInput {
        id: None,
        supplier_id: supplier,
        date: TODAY.into(),
        currency: "USD".into(),
        fx_rate: "56.50".into(),
        expected_date: String::new(),
        supplier_ref: String::new(),
        notes: String::new(),
        lines: vec![PoLineInput {
            id: None,
            item_id: item,
            qty: 10,
            price_minor: Some(1_000),
            notes: String::new(),
        }],
    };
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::PO, &id).unwrap();
    db.write(actor.clone(), move |tx| {
        purchasing::create(tx, &input, &res, true)
    })
    .unwrap()
}

fn tt(po: &str, account: Option<&str>) -> TtInput {
    TtInput {
        id: None,
        po_id: po.into(),
        date: TODAY.into(),
        currency: "USD".into(),
        amount_minor: 10_000,
        fx_rate: "56.00".into(),
        reference: "TT-1".into(),
        bank_account_id: account.map(String::from),
        notes: String::new(),
    }
}

#[test]
fn accounts_collect_deposits_transfers_and_tts() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let bdo = add_account(&a.db, &actor, "BDO Savings");
    let bpi = add_account(&a.db, &actor, "BPI Current");
    let dup = account_input("bdo savings");
    assert_eq!(
        invalid(a.db.write(actor.clone(), move |tx| bank_accounts::save(tx, &dup))),
        "a bank account named bdo savings already exists"
    );
    let client = add_client(&a.db, &actor);

    // Two checks deposited to BDO: one clears, one bounces. One on hand.
    let c1 = pay(&a.db, &actor, payment(&client, PayMethod::Check, 10_000, None)).unwrap();
    let c2 = pay(&a.db, &actor, payment(&client, PayMethod::Check, 5_000, None)).unwrap();
    pay(&a.db, &actor, payment(&client, PayMethod::Check, 7_000, None)).unwrap();
    step(&a.db, &actor, &c1, CheckStep::Deposit, Some(&bdo)).unwrap();
    step(&a.db, &actor, &c1, CheckStep::Clear, None).unwrap();
    step(&a.db, &actor, &c2, CheckStep::Deposit, Some(&bdo)).unwrap();
    // Transfers: one into BPI, one without an account.
    let t1 = pay(&a.db, &actor, payment(&client, PayMethod::BankTransfer, 20_000, Some(&bpi))).unwrap();
    pay(&a.db, &actor, payment(&client, PayMethod::BankTransfer, 3_000, None)).unwrap();
    // A TT of US$100.00 at 56.00 from BPI.
    let po = add_po(&a.db, &actor);
    let input = tt(&po, Some(&bpi));
    a.db.write(actor.clone(), move |tx| purchasing::save_payment(tx, &input))
        .unwrap();

    let list = a.db.read(bank_accounts::list).unwrap();
    let get = |id: &str| list.iter().find(|x| x.id == id).unwrap().clone();
    let b = get(&bdo);
    assert_eq!(
        (b.checks_cleared_c, b.checks_pending_c, b.checks_pending, b.transfers_in_c),
        (10_000, 5_000, 1, 0)
    );
    assert_eq!(b.tt_count, Some(0));
    let p = get(&bpi);
    assert_eq!(p.transfers_in_c, 20_000);
    assert_eq!((p.tt_count, p.tt_out_php_c), (Some(1), Some(560_000)));
    assert_eq!(p.last_date.as_deref(), Some(TODAY));

    let moves = a.db.read(|c| bank_accounts::moves(c, &bdo, true)).unwrap();
    assert_eq!(moves.len(), 2);
    assert!(moves.iter().all(|m| m.kind == BankMoveKind::CheckDeposit));
    assert_eq!(moves.iter().find(|m| m.id == c1).unwrap().check_status, Some(CheckStatus::Cleared));
    assert_eq!(moves[0].detail.as_deref(), Some("BPI 0012345"));
    let kinds = |costs| {
        a.db.read(|c| bank_accounts::moves(c, &bpi, costs))
            .unwrap()
            .into_iter()
            .map(|m| m.kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(true).len(), 2);
    assert_eq!(kinds(false), vec![BankMoveKind::TransferIn]);

    // The payment and check show the account name.
    let shown = a.db.read(|c| payments::get(c, &c2, TODAY)).unwrap().unwrap().payment;
    assert_eq!(shown.check.unwrap().deposit_account.as_deref(), Some("BDO Savings"));
    let shown = a.db.read(|c| payments::get(c, &t1, TODAY)).unwrap().unwrap().payment;
    assert_eq!(shown.bank_account_name.as_deref(), Some("BPI Current"));

    // Bounced: no longer pending. Undoing a deposit clears the account.
    step(&a.db, &actor, &c2, CheckStep::Bounce, None).unwrap();
    step(&a.db, &actor, &c1, CheckStep::Undo, None).unwrap();
    step(&a.db, &actor, &c1, CheckStep::Undo, None).unwrap();
    let b = a.db.read(|c| bank_accounts::get(c, &bdo)).unwrap().unwrap();
    assert_eq!(
        (b.checks_cleared_c, b.checks_pending_c, b.checks_bounced),
        (0, 0, 1)
    );
    let row = a.db.read(|c| payments::get(c, &c1, TODAY)).unwrap().unwrap().payment;
    assert_eq!(row.bank_account_id, None);
}

#[test]
fn inactive_and_used_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let old = add_account(&a.db, &actor, "Old account");
    let unused = add_account(&a.db, &actor, "Unused");
    let client = add_client(&a.db, &actor);
    let t = pay(&a.db, &actor, payment(&client, PayMethod::BankTransfer, 1_000, Some(&old))).unwrap();

    // Used: can't be deleted, only made inactive.
    let id = old.clone();
    assert_eq!(
        invalid(a.db.write(actor.clone(), move |tx| bank_accounts::delete(tx, &id))),
        "payments name this account; mark it inactive instead"
    );
    let id = unused.clone();
    a.db.write(actor.clone(), move |tx| bank_accounts::delete(tx, &id))
        .unwrap();
    let mut input = account_input("Old account");
    input.id = Some(old.clone());
    input.active = false;
    a.db.write(actor.clone(), move |tx| bank_accounts::save(tx, &input))
        .unwrap();

    // An inactive account can't be chosen anew, but a payment that already
    // names it keeps it when edited.
    assert_eq!(
        invalid(pay(&a.db, &actor, payment(&client, PayMethod::BankTransfer, 500, Some(&old)))),
        "that bank account is inactive; choose another"
    );
    let mut edit = payment(&client, PayMethod::BankTransfer, 1_200, Some(&old));
    edit.id = Some(t.clone());
    a.db.write(actor.clone(), move |tx| payments::update(tx, &edit))
        .unwrap();
    let c = pay(&a.db, &actor, payment(&client, PayMethod::Check, 900, None)).unwrap();
    assert_eq!(
        invalid(step(&a.db, &actor, &c, CheckStep::Deposit, Some(&old))),
        "that bank account is inactive; choose another"
    );
    assert_eq!(
        invalid(step(&a.db, &actor, &c, CheckStep::Deposit, Some("nope"))),
        "that bank account no longer exists"
    );
    // Cash ignores the account.
    let cash = pay(&a.db, &actor, payment(&client, PayMethod::Cash, 100, Some("nope"))).unwrap();
    let row = a.db.read(|c| payments::get(c, &cash, TODAY)).unwrap().unwrap().payment;
    assert_eq!(row.bank_account_id, None);
}
