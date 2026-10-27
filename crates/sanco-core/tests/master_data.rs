mod common;

use common::{TestPeer, pull};
use sanco_core::domain::auth::Perms;
use sanco_core::domain::charge_types;
use sanco_core::domain::clients::{self, ClientInput};
use sanco_core::domain::items::{self, CostLineInput, CostSheetInput, ItemInput};
use sanco_core::domain::locations::{self, LocationInput};
use sanco_core::domain::pricing::{self, ChargeMode, PriceRule, RateFilter, RateInput};
use sanco_core::domain::settings;
use sanco_core::domain::suppliers::{self, SupplierInput};
use sanco_core::domain::users::{self, NewUser};
use sanco_core::sync::numbering::{self, NumberFormat, series};
use sanco_core::{Actor, Db, Error};

fn setup(db: &Db) -> Actor {
    let admin = users::create_first_admin(
        db,
        NewUser {
            username: "ana".into(),
            display_name: "Ana".into(),
            password: "secret1".into(),
            role_ids: vec![],
        },
    )
    .unwrap();
    admin.actor()
}

fn client_input(name: &str, tier: Option<i64>) -> ClientInput {
    ClientInput {
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
    }
}

fn add_client(db: &Db, actor: &Actor, name: &str, tier: Option<i64>) -> String {
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::CLIENT, &id).unwrap();
    let input = client_input(name, tier);
    db.write(actor.clone(), move |tx| {
        clients::create(tx, &input, &res, true)
    })
    .unwrap()
}

fn supplier_input(name: &str) -> SupplierInput {
    SupplierInput {
        id: None,
        name: name.into(),
        country: "tw".into(),
        currency: "usd".into(),
        contact_person: String::new(),
        phone: String::new(),
        email: String::new(),
        address: String::new(),
        notes: String::new(),
        active: true,
    }
}

fn add_supplier(db: &Db, actor: &Actor, name: &str) -> String {
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::SUPPLIER, &id).unwrap();
    let input = supplier_input(name);
    db.write(actor.clone(), move |tx| suppliers::create(tx, &input, &res))
        .unwrap()
}

fn item_input(code: &str, supplier: Option<&str>, margin: Option<i64>) -> ItemInput {
    ItemInput {
        id: None,
        code: code.into(),
        name: format!("Item {code}"),
        unit: "pc".into(),
        category: "Gloves".into(),
        supplier_id: supplier.map(String::from),
        tracks_expiry: true,
        reorder_level: Some(10),
        default_margin_bp: margin,
        notes: String::new(),
        active: true,
        serial_code: String::new(),
        serial_digits: None,
        pcs_per_box: None,
    }
}

fn add_item(db: &Db, actor: &Actor, input: ItemInput) -> Result<String, Error> {
    db.write(actor.clone(), move |tx| items::save(tx, &input, true))
}

fn charge(db: &Db, name: &str) -> String {
    db.read(charge_types::list)
        .unwrap()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap()
        .id
}

/// US$10.00 at 56.50 = ₱565; freight ₱20; duty 10% = ₱56.50 → ₱641.50.
fn set_cost(db: &Db, actor: &Actor, item: &str) {
    let input = CostSheetInput {
        item_id: item.into(),
        currency: "USD".into(),
        supplier_price_minor: Some(1_000),
        fx_rate: "56.50".into(),
        shipment_qty: None,
        notes: String::new(),
        lines: vec![
            CostLineInput {
                charge_type_id: charge(db, "Freight"),
                mode: ChargeMode::PerUnit,
                value: Some(2_000),
            },
            CostLineInput {
                charge_type_id: charge(db, "Customs duty"),
                mode: ChargeMode::Pct,
                value: Some(1_000),
            },
        ],
    };
    db.write(actor.clone(), move |tx| items::save_cost_sheet(tx, &input))
        .unwrap();
}

#[test]
fn seeds_are_shared_and_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    setup(&a.db);
    // B seeds on its own (e.g. both PCs upgraded while apart).
    sanco_core::domain::seed_defaults(&b.db).unwrap();
    sanco_core::domain::seed_defaults(&a.db).unwrap();

    let names: Vec<String> =
        a.db.read(locations::list)
            .unwrap()
            .into_iter()
            .map(|l| l.name)
            .collect();
    assert_eq!(names, ["Office", "Warehouse"]);
    assert_eq!(a.db.read(charge_types::list).unwrap().len(), 5);

    pull(&a.db, &b.db, 100);
    pull(&b.db, &a.db, 100);
    assert_eq!(
        a.db.read(locations::list).unwrap(),
        b.db.read(locations::list).unwrap()
    );
    let conflicts: i64 =
        a.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM conflicts", [], |r| r.get(0))?))
            .unwrap();
    assert_eq!(conflicts, 0, "identical seeds are not conflicts");

    // A deleted seed stays deleted.
    let office = a.db.read(locations::list).unwrap()[0].id.clone();
    a.db.write(Actor::system(), move |tx| locations::delete(tx, &office))
        .unwrap();
    sanco_core::domain::seed_defaults(&a.db).unwrap();
    assert_eq!(a.db.read(locations::list).unwrap().len(), 1);
}

#[test]
fn locations_have_unique_names() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let input = LocationInput {
        id: None,
        name: " office ".into(),
        notes: String::new(),
        sort: 3,
        active: true,
    };
    let dup =
        a.db.write(actor.clone(), move |tx| locations::save(tx, &input));
    assert!(matches!(dup, Err(Error::Invalid(_))));
    let input = LocationInput {
        id: None,
        name: "Store".into(),
        notes: String::new(),
        sort: 3,
        active: true,
    };
    a.db.write(actor, move |tx| locations::save(tx, &input))
        .unwrap();
    assert_eq!(a.db.read(locations::list).unwrap()[2].name, "Store");
}

#[test]
fn clients_get_codes_and_keep_hidden_margins() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let c1 = add_client(&a.db, &actor, "Mercy Hospital", Some(2_000));
    let c2 = add_client(&a.db, &actor, "St. Luke's Pharmacy", None);

    let list = a.db.read(clients::list).unwrap();
    assert_eq!(list.len(), 2);
    let one = a.db.read(|c| clients::get(c, &c1)).unwrap().unwrap();
    assert_eq!(one.code.as_deref(), Some("CL-0001"));
    assert!(one.code_confirmed);
    assert_eq!(one.margin_tier_bp, Some(2_000));
    let two = a.db.read(|c| clients::get(c, &c2)).unwrap().unwrap();
    assert_eq!(two.code.as_deref(), Some("CL-0002"));

    // Someone who can't see margins edits the client: the tier stays.
    let mut input = client_input("Mercy Hospital Inc.", None);
    input.id = Some(c1.clone());
    a.db.write(actor.clone(), move |tx| clients::update(tx, &input, false))
        .unwrap();
    let mut one = a.db.read(|c| clients::get(c, &c1)).unwrap().unwrap();
    assert_eq!(one.name, "Mercy Hospital Inc.");
    assert_eq!(one.margin_tier_bp, Some(2_000));
    one.strip(&Perms::staff());
    assert_eq!(one.margin_tier_bp, None);

    // Validation.
    let bad = client_input("  ", None);
    let id = a.db.new_id().unwrap();
    let res = numbering::reserve(&a.db, &[], series::CLIENT, &id).unwrap();
    let r = a.db.write(actor.clone(), move |tx| {
        clients::create(tx, &bad, &res, true)
    });
    assert!(matches!(r, Err(Error::Invalid(_))));
    let bad = client_input("X", Some(10_000));
    let id = a.db.new_id().unwrap();
    let res = numbering::reserve(&a.db, &[], series::CLIENT, &id).unwrap();
    let r =
        a.db.write(actor, move |tx| clients::create(tx, &bad, &res, true));
    assert!(matches!(r, Err(Error::Invalid(_))));
    assert_eq!(a.db.read(clients::list).unwrap().len(), 2);
}

#[test]
fn number_formats_apply_to_new_codes() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let fmt = NumberFormat {
        prefix: "c".into(),
        year: false,
        reset_yearly: false,
        pad: 3,
    };
    a.db.write(actor.clone(), move |tx| {
        settings::set_number_format(tx, series::CLIENT, &fmt)
    })
    .unwrap();
    let id = add_client(&a.db, &actor, "A", None);
    let got = a.db.read(|c| clients::get(c, &id)).unwrap().unwrap();
    assert_eq!(got.code.as_deref(), Some("C-001"));

    // The yearly reset is fixed once numbers exist.
    let fmt = NumberFormat {
        prefix: "C".into(),
        year: true,
        reset_yearly: true,
        pad: 3,
    };
    let r = a.db.write(actor.clone(), move |tx| {
        settings::set_number_format(tx, series::CLIENT, &fmt)
    });
    assert!(matches!(r, Err(Error::Invalid(_))));
    // Restarting each year needs the year shown.
    let fmt = NumberFormat {
        prefix: "BK".into(),
        year: false,
        reset_yearly: true,
        pad: 4,
    };
    let r = a.db.write(actor, move |tx| {
        settings::set_number_format(tx, series::BOOKING, &fmt)
    });
    assert!(matches!(r, Err(Error::Invalid(_))));
    let formats =
        a.db.read(|c| settings::number_formats(c, common::T0))
            .unwrap();
    let dr = formats.iter().find(|f| f.series == "DR").unwrap();
    assert_eq!(dr.example, "DR-2026-0001");
}

#[test]
fn items_flag_codes_and_block_supplier_delete() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let b = TestPeer::new(dir.path(), "b", 2);
    let actor = setup(&a.db);
    pull(&b.db, &a.db, 500);
    let sup = add_supplier(&a.db, &actor, "Formosa Medical");
    let s = a.db.read(|c| suppliers::get(c, &sup)).unwrap().unwrap();
    assert_eq!(s.code.as_deref(), Some("SUP-0001"));
    assert_eq!((s.country.as_str(), s.currency.as_str()), ("TW", "USD"));

    add_item(&a.db, &actor, item_input("GLV-100", Some(&sup), None)).unwrap();
    let dup = add_item(&a.db, &actor, item_input("glv-100", None, None));
    assert!(matches!(dup, Err(Error::Invalid(_))));

    // The same code added offline on B is flagged after sync, not lost.
    add_item(&b.db, &actor, item_input("GLV-100", None, None)).unwrap();
    pull(&a.db, &b.db, 500);
    let list = a.db.read(items::list).unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().all(|i| i.duplicate_code));

    let r = a.db.write(actor.clone(), {
        let sup = sup.clone();
        move |tx| suppliers::delete(tx, &sup)
    });
    assert!(matches!(r, Err(Error::Invalid(_))));
    assert_eq!(
        a.db.read(|c| suppliers::get(c, &sup))
            .unwrap()
            .unwrap()
            .item_count,
        1
    );
}

#[test]
fn landed_cost_and_price_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let sup = add_supplier(&a.db, &actor, "Formosa Medical");
    let item = add_item(
        &a.db,
        &actor,
        item_input("GLV-100", Some(&sup), Some(2_000)),
    )
    .unwrap();
    let tiered = add_client(&a.db, &actor, "Tiered", Some(3_000));
    let plain = add_client(&a.db, &actor, "Plain", None);

    // No cost sheet yet: no price at all; the sheet defaults to USD.
    let sheet = a.db.read(|c| items::cost_sheet(c, &item)).unwrap().unwrap();
    assert_eq!(sheet.currency, "USD");
    assert_eq!(sheet.lines.len(), 5);
    assert_eq!(sheet.landed, None);
    let q = a.db.read(|c| pricing::quote(c, &plain, &item)).unwrap();
    assert_eq!((q.rule, q.net_c), (PriceRule::None, None));

    set_cost(&a.db, &actor, &item);
    let sheet = a.db.read(|c| items::cost_sheet(c, &item)).unwrap().unwrap();
    let landed = sheet.landed.unwrap();
    assert_eq!(
        (landed.base_c, landed.charges_c, landed.total_c),
        (56_500, 7_650, 64_150)
    );
    let duty = sheet
        .lines
        .iter()
        .find(|l| l.name == "Customs duty")
        .unwrap();
    assert_eq!(duty.amount_c, Some(5_650));
    assert_eq!(sheet.fx_rate.as_deref(), Some("56.5"));

    let info = a.db.read(|c| items::get(c, &item)).unwrap().unwrap();
    assert_eq!(info.landed_cost_c, Some(64_150));
    // ₱641.50 ÷ 0.8 = ₱801.875 → ₱801.88.
    assert_eq!(info.base_price_c, Some(80_188));

    // Rule 3: item default margin.
    let q = a.db.read(|c| pricing::quote(c, &plain, &item)).unwrap();
    assert_eq!(
        (q.rule, q.net_c, q.vat),
        (PriceRule::ItemDefault, Some(80_188), None)
    );
    let v = q.vat_amounts.unwrap();
    assert_eq!((v.vat_c, v.gross_c), (9_623, 89_811));
    assert_eq!(q.margin_bp, Some(2_000));

    // Rule 2: client tier. ₱641.50 ÷ 0.7 = ₱916.43.
    let q = a.db.read(|c| pricing::quote(c, &tiered, &item)).unwrap();
    assert_eq!((q.rule, q.net_c), (PriceRule::ClientTier, Some(91_643)));

    // Rule 1: manual rate, here below cost and without VAT.
    let rate = RateInput {
        client_id: tiered.clone(),
        item_id: item.clone(),
        price_c: Some(60_000),
        vat: false,
        notes: "promo".into(),
    };
    a.db.write(actor.clone(), move |tx| pricing::save_rate(tx, &rate))
        .unwrap();
    let mut q = a.db.read(|c| pricing::quote(c, &tiered, &item)).unwrap();
    assert_eq!(
        (q.rule, q.net_c, q.vat),
        (PriceRule::Manual, Some(60_000), Some(false))
    );
    assert_eq!(q.vat_amounts, None);
    assert!(q.below_cost);
    q.strip(&Perms::staff());
    assert_eq!((q.cost_c, q.margin_bp), (None, None));
    assert!(q.below_cost, "the warning stays for everyone");

    let rates =
        a.db.read(|c| {
            pricing::list_rates(
                c,
                &RateFilter {
                    client_id: Some(tiered.clone()),
                    item_id: None,
                },
            )
        })
        .unwrap();
    assert_eq!(rates.len(), 1);
    assert_eq!(rates[0].client_code.as_deref(), Some("CL-0001"));
    assert_eq!(rates[0].vat_amounts.gross_c, 60_000);
    assert!(rates[0].below_cost);

    // VAT rate from settings applies to VAT-able rates.
    a.db.write(actor.clone(), |tx| settings::set_vat_rate_bp(tx, 1_000))
        .unwrap();
    let rate = RateInput {
        client_id: plain.clone(),
        item_id: item.clone(),
        price_c: Some(100_000),
        vat: true,
        notes: String::new(),
    };
    a.db.write(actor.clone(), move |tx| pricing::save_rate(tx, &rate))
        .unwrap();
    let q = a.db.read(|c| pricing::quote(c, &plain, &item)).unwrap();
    assert_eq!(q.vat_amounts.unwrap().gross_c, 110_000);
    assert!(!q.below_cost);

    // Clearing a rate falls back to the tier; the row stays, price null.
    let clear = RateInput {
        client_id: tiered.clone(),
        item_id: item.clone(),
        price_c: None,
        vat: false,
        notes: String::new(),
    };
    a.db.write(actor.clone(), move |tx| pricing::save_rate(tx, &clear))
        .unwrap();
    let q = a.db.read(|c| pricing::quote(c, &tiered, &item)).unwrap();
    assert_eq!(q.rule, PriceRule::ClientTier);
    assert_eq!(
        a.db.read(clients::list)
            .unwrap()
            .iter()
            .map(|c| c.rate_count)
            .sum::<i64>(),
        1
    );

    // Deleting the item takes its sheet and rates along.
    let it = item.clone();
    a.db.write(actor, move |tx| items::delete(tx, &it)).unwrap();
    assert!(
        a.db.read(|c| items::cost_sheet(c, &item))
            .unwrap()
            .is_none()
    );
    assert!(
        a.db.read(|c| pricing::list_rates(c, &RateFilter::default()))
            .unwrap()
            .is_empty()
    );
    let left: i64 =
        a.db.read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM item_cost_charges WHERE _deleted = 0",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn cost_sheet_validation_and_php() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let item = add_item(&a.db, &actor, item_input("MSK-1", None, None)).unwrap();
    let sheet = a.db.read(|c| items::cost_sheet(c, &item)).unwrap().unwrap();
    assert_eq!(sheet.currency, "PHP");

    let mut input = CostSheetInput {
        item_id: item.clone(),
        currency: "USD".into(),
        supplier_price_minor: Some(100),
        fx_rate: "abc".into(),
        shipment_qty: None,
        notes: String::new(),
        lines: vec![],
    };
    let i2 = input.clone();
    let r =
        a.db.write(actor.clone(), move |tx| items::save_cost_sheet(tx, &i2));
    assert!(matches!(r, Err(Error::Invalid(_))));
    input.currency = "EUR".into();
    let i2 = input.clone();
    let r =
        a.db.write(actor.clone(), move |tx| items::save_cost_sheet(tx, &i2));
    assert!(matches!(r, Err(Error::Invalid(_))));

    // PHP ignores the rate; a pct charge on ₱1.00 base.
    input.currency = "php".into();
    input.lines = vec![CostLineInput {
        charge_type_id: charge(&a.db, "Import VAT"),
        mode: ChargeMode::Pct,
        value: Some(1_200),
    }];
    a.db.write(actor.clone(), move |tx| items::save_cost_sheet(tx, &input))
        .unwrap();
    let sheet = a.db.read(|c| items::cost_sheet(c, &item)).unwrap().unwrap();
    assert_eq!(sheet.fx_rate, None);
    assert_eq!(sheet.landed.unwrap().total_c, 112);

    // A deactivated charge type with a value still shows and counts.
    let vat_type = charge(&a.db, "Import VAT");
    let t =
        a.db.read(charge_types::list)
            .unwrap()
            .into_iter()
            .find(|t| t.id == vat_type)
            .unwrap();
    assert_eq!(t.used_by, 1);
    let input = charge_types::ChargeTypeInput {
        id: Some(vat_type.clone()),
        name: t.name,
        mode: t.mode,
        sort: t.sort,
        active: false,
    };
    a.db.write(actor.clone(), move |tx| {
        charge_types::save(tx, &input).map(drop)
    })
    .unwrap();
    let sheet = a.db.read(|c| items::cost_sheet(c, &item)).unwrap().unwrap();
    assert_eq!(sheet.lines.len(), 5);
    assert!(
        sheet
            .lines
            .iter()
            .any(|l| l.charge_type_id == vat_type && !l.active)
    );
    let r =
        a.db.write(actor, move |tx| charge_types::delete(tx, &vat_type));
    assert!(matches!(r, Err(Error::Invalid(_))));
}

#[test]
fn shipment_charges_need_a_quantity() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let item = add_item(&a.db, &actor, item_input("MSK-1", None, None)).unwrap();

    // ₱100 base; freight ₱10,000 for the shipment; brokerage ₱5 per unit.
    let mut input = CostSheetInput {
        item_id: item.clone(),
        currency: "PHP".into(),
        supplier_price_minor: Some(10_000),
        fx_rate: String::new(),
        shipment_qty: None,
        notes: String::new(),
        lines: vec![
            CostLineInput {
                charge_type_id: charge(&a.db, "Freight"),
                mode: ChargeMode::Shipment,
                value: Some(1_000_000),
            },
            CostLineInput {
                charge_type_id: charge(&a.db, "Brokerage"),
                mode: ChargeMode::PerUnit,
                value: Some(500),
            },
        ],
    };
    for qty in [None, Some(0), Some(-5)] {
        input.shipment_qty = qty;
        let i2 = input.clone();
        let r =
            a.db.write(actor.clone(), move |tx| items::save_cost_sheet(tx, &i2));
        assert!(matches!(r, Err(Error::Invalid(_))), "{qty:?}");
    }

    input.shipment_qty = Some(400);
    a.db.write(actor.clone(), move |tx| items::save_cost_sheet(tx, &input))
        .unwrap();
    let sheet = a.db.read(|c| items::cost_sheet(c, &item)).unwrap().unwrap();
    assert_eq!(sheet.shipment_qty, Some(400));
    let freight = sheet.lines.iter().find(|l| l.name == "Freight").unwrap();
    assert_eq!(freight.mode, ChargeMode::Shipment);
    assert_eq!(freight.value, Some(1_000_000));
    assert_eq!(freight.amount_c, Some(2_500)); // ₱10,000 ÷ 400
    assert_eq!(sheet.landed.unwrap().total_c, 10_000 + 2_500 + 500);
    assert_eq!(
        a.db.read(|c| items::landed_cost_of(c, &item)).unwrap(),
        sheet.landed
    );
}
