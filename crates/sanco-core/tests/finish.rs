//! M10: Dashboard KPIs and issues, reports, to-dos.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::TestPeer;
use sanco_core::domain::boxes::Serials;
use sanco_core::domain::auth::{Action, Module, Perms};
use sanco_core::domain::bd::{self, BdInput, BdLineInput, BdStatus};
use sanco_core::domain::bookings::{self, BookingAction, BookingInput, BookingLineInput};
use sanco_core::domain::clients::{self, ClientInput};
use sanco_core::domain::dashboard::{self, Tone};
use sanco_core::domain::delivery::{self, DeliverInput, DrLineInput, DrRequestInput};
use sanco_core::domain::inventory::{self, StockDocInput, StockKind, StockLineInput};
use sanco_core::domain::items::{self, CostSheetInput, ItemInput};
use sanco_core::domain::reports::{self, DateRange, SalesGroup};
use sanco_core::domain::todos::{self, TodoInput};
use sanco_core::domain::users::{self, NewUser};
use sanco_core::ids::derived_id;
use sanco_core::sync::numbering::{self, series};
use sanco_core::time::manila_date_string;
use sanco_core::{Actor, Db};
use serde_json::json;

const TODAY: &str = "2026-10-21";

fn admin(db: &Db) -> Actor {
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

/// ₱100 cost at a 20% default margin (₱125), reorder at 95, 100 in stock.
fn add_item(db: &Db, actor: &Actor, code: &str) -> String {
    let input = ItemInput {
        id: None,
        code: code.into(),
        name: format!("Item {code}"),
        unit: "box".into(),
        category: String::new(),
        supplier_id: None,
        tracks_expiry: false,
        reorder_level: Some(95),
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
            box_id: None,
            pcs_per_box: None,
        }],
        serial_year: String::new(),
    };
    let doc_id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], series::STOCK_RECEIPT, &doc_id).unwrap();
    db.write(actor.clone(), move |tx| inventory::create(tx, &doc, &res, &mut Serials::none()))
        .unwrap();
    id
}

/// A confirmed VAT booking of 10 at the default price; returns (booking, line).
fn booking(db: &Db, actor: &Actor, client: &str, item: &str) -> (String, String) {
    let input = BookingInput {
        id: None,
        client_id: client.into(),
        date: "2026-07-01".into(),
        vat: true,
        client_ref: String::new(),
        deliver_to: String::new(),
        notes: String::new(),
        lines: vec![BookingLineInput {
            id: None,
            item_id: item.into(),
            qty: 10,
            price_c: None,
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

fn request_dr(db: &Db, actor: &Actor, booking: &str, line: &str, qty: i64) -> String {
    let input = DrRequestInput {
        booking_id: booking.into(),
        date: "2026-07-01".into(),
        from_location_id: warehouse(),
        deliver_to: String::new(),
        notes: String::new(),
        lines: vec![DrLineInput {
            id: None,
            booking_line_id: line.into(),
            expiry: None,
            qty,
            boxes: vec![],
        }],
    };
    db.write(actor.clone(), move |tx| delivery::request(tx, &input))
        .unwrap()
}

fn deliver(db: &Db, actor: &Actor, id: &str, no: &str, date: &str) {
    let res = numbering::reserve_exact(db, &[], series::DR, no, id).unwrap();
    let rid = id.to_string();
    db.write(actor.clone(), move |tx| {
        delivery::approve(tx, &rid, Some(&res))
    })
    .unwrap();
    let (rid, input) = (
        id.to_string(),
        DeliverInput {
            date: date.into(),
            received_by: String::new(),
        },
    );
    db.write(actor.clone(), move |tx| {
        delivery::deliver(tx, &rid, &input, None)
    })
    .unwrap();
}

fn perms(modules: &[Module]) -> Perms {
    Perms {
        admin: false,
        modules: modules
            .iter()
            .map(|m| (*m, BTreeSet::from([Action::View])))
            .collect::<BTreeMap<_, _>>(),
        fields: BTreeSet::new(),
    }
}

fn all() -> Perms {
    Perms {
        admin: true,
        ..Perms::default()
    }
}

#[test]
fn dashboard_kpis_and_issues() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let db = &a.db;
    let ana = admin(db);
    let today = manila_date_string(db.now_ms());
    assert_eq!(today, TODAY);

    let client = add_client(db, &ana, "Mercury Drug");
    let item = add_item(db, &ana, "GLV-100");
    let (bk, line) = booking(db, &ana, &client, &item);
    let dr = request_dr(db, &ana, &bk, &line, 4);
    deliver(db, &ana, &dr, "1001", "2026-07-01");
    request_dr(db, &ana, &bk, &line, 2);

    let d = db.read(|c| dashboard::summary(c, &all(), TODAY)).unwrap();
    let kpi = |key: &str| {
        d.kpis
            .iter()
            .find(|k| k.key == key)
            .unwrap_or_else(|| panic!("no KPI {key}"))
            .clone()
    };
    assert_eq!(kpi("bookings_to_deliver").value, 1);
    assert_eq!(kpi("drs_open").value, 1);
    assert_eq!(kpi("drs_open").href, "/accounting/?status=requested");
    // 4 × ₱125 + 12% VAT, due 30 days after 2026-07-01: overdue.
    assert_eq!(kpi("receivables").value, 56_000);
    assert_eq!(kpi("overdue").value, 56_000);
    assert_eq!(kpi("overdue").tone, Tone::Warn);
    // 96 left, reorder at 95: not low yet.
    assert_eq!(kpi("low_stock").value, 0);
    assert_eq!(kpi("po_transit").value, 0);
    assert!(d.issues.is_empty(), "{:?}", d.issues);

    // Only the modules the user may view.
    let d = db
        .read(|c| dashboard::summary(c, &perms(&[Module::Inventory]), TODAY))
        .unwrap();
    let keys: Vec<_> = d.kpis.iter().map(|k| k.key.as_str()).collect();
    assert_eq!(keys, ["low_stock", "near_expiry"]);

    // An item code typed on two PCs offline (written directly here).
    db.write(ana.clone(), |tx| {
        tx.insert("items", json!({ "code": "glv-100", "name": "Gloves again" }))
    })
    .unwrap();
    let d = db.read(|c| dashboard::summary(c, &all(), TODAY)).unwrap();
    assert_eq!(d.issues.len(), 1);
    assert_eq!(d.issues[0].key, "item_duplicate");
    assert_eq!(d.issues[0].detail, "GLV-100");
    let d = db
        .read(|c| dashboard::summary(c, &perms(&[Module::Clients]), TODAY))
        .unwrap();
    assert!(d.issues.is_empty());
}

#[test]
fn reports_sales_margin_and_bd() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 2);
    let db = &a.db;
    let ana = admin(db);
    let client = add_client(db, &ana, "Mercury Drug");
    let item = add_item(db, &ana, "GLV-100");
    let (bk, line) = booking(db, &ana, &client, &item);
    let dr = request_dr(db, &ana, &bk, &line, 4);
    deliver(db, &ana, &dr, "1001", "2026-07-01");
    let dr = request_dr(db, &ana, &bk, &line, 3);
    deliver(db, &ana, &dr, "1002", "2026-08-15");

    let any = DateRange::default();
    let by_client = db
        .read(|c| reports::sales(c, &any, SalesGroup::Client))
        .unwrap();
    assert_eq!(by_client.len(), 1);
    assert_eq!(by_client[0].label, "Mercury Drug");
    assert_eq!(by_client[0].code.as_deref(), Some("CL-0001"));
    assert_eq!(by_client[0].drs, 2);
    assert_eq!(
        (by_client[0].net_c, by_client[0].vat_c, by_client[0].gross_c),
        (87_500, 10_500, 98_000)
    );

    let by_month = db
        .read(|c| reports::sales(c, &any, SalesGroup::Month))
        .unwrap();
    let months: Vec<_> = by_month.iter().map(|r| (r.key.as_str(), r.net_c)).collect();
    assert_eq!(months, [("2026-07", 50_000), ("2026-08", 37_500)]);

    let august = DateRange::new("2026-08-01", "2026-08-31");
    let by_item = db
        .read(|c| reports::sales(c, &august, SalesGroup::Item))
        .unwrap();
    assert_eq!(by_item.len(), 1);
    assert_eq!(by_item[0].qty, Some(3));
    assert_eq!(by_item[0].gross_c, 42_000);
    let none = db
        .read(|c| reports::sales(c, &DateRange::new("2026-09-01", ""), SalesGroup::Client))
        .unwrap();
    assert!(none.is_empty());

    // ₱125 price on a ₱100 landed cost: 20% margin on price.
    let margin = db.read(|c| reports::item_margin(c, &any)).unwrap();
    assert_eq!(margin.len(), 1);
    assert_eq!(margin[0].qty, 7);
    assert_eq!(margin[0].cost_c, Some(70_000));
    assert_eq!(margin[0].margin_c, Some(17_500));
    assert_eq!(margin[0].margin_bp, Some(2_000));

    // BD: two requests in October, one lost.
    for status in [BdStatus::Open, BdStatus::Lost] {
        let input = BdInput {
            id: None,
            client_id: None,
            prospect_name: "St. Luke's".into(),
            contact: String::new(),
            date: "2026-10-02".into(),
            status,
            follow_up: None,
            vat: false,
            notes: String::new(),
            lost_reason: if status == BdStatus::Lost {
                "price".into()
            } else {
                String::new()
            },
            lines: vec![BdLineInput {
                id: None,
                item_id: Some(item.clone()),
                description: String::new(),
                qty: 2,
                price_c: Some(12_500),
            }],
        };
        let id = db.new_id().unwrap();
        let res = numbering::reserve(db, &[], series::BD, &id).unwrap();
        db.write(ana.clone(), move |tx| bd::create(tx, &input, &res))
            .unwrap();
    }
    let conv = db.read(|c| reports::bd_conversion(c, &any)).unwrap();
    assert_eq!(conv.len(), 1);
    assert_eq!(conv[0].month, "2026-10");
    assert_eq!((conv[0].total, conv[0].open, conv[0].lost), (2, 1, 1));
    assert_eq!(conv[0].lost_net_c, 25_000);
    assert_eq!(conv[0].win_rate_bp, Some(0));
    assert_eq!(reports::win_rate(2, 1), Some(6_667));
    assert_eq!(reports::win_rate(0, 0), None);

    let spend = db.read(|c| reports::po_spend(c, &any)).unwrap();
    assert!(spend.is_empty());
}

#[test]
fn todos_visibility_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 3);
    let db = &a.db;
    let ana = admin(db);
    let ana_id = ana.user_id.clone().unwrap();
    let ben_id = users::create_user(
        db,
        ana.clone(),
        NewUser {
            username: "ben".into(),
            display_name: "Ben".into(),
            password: "secret1".into(),
            role_ids: vec![derived_id("role", "staff")],
        },
    )
    .unwrap();
    let ben = Actor::user(ben_id.clone());
    let cy_id = users::create_user(
        db,
        ana.clone(),
        NewUser {
            username: "cy".into(),
            display_name: "Cy".into(),
            password: "secret1".into(),
            role_ids: vec![derived_id("role", "staff")],
        },
    )
    .unwrap();

    let save = |actor: &Actor, title: &str, assignee: Option<&str>, due: Option<&str>| {
        let input = TodoInput {
            id: None,
            title: title.into(),
            notes: String::new(),
            due: due.map(String::from),
            assignee_id: assignee.map(String::from),
        };
        db.write(actor.clone(), move |tx| todos::save(tx, &input))
    };
    let shared = save(&ana, "Count the warehouse", None, Some("2026-10-25")).unwrap();
    let for_ben = save(&ana, "Call Mercury", Some(&ben_id), Some("2026-10-22")).unwrap();
    save(&ana, "Ana only", Some(&ana_id), None).unwrap();
    assert!(save(&ana, "  ", None, None).is_err());
    assert!(save(&ana, "Bad date", None, Some("2026-13-01")).is_err());
    assert!(save(&ana, "Nobody", Some("missing"), None).is_err());

    let titles = |user: &str| -> Vec<String> {
        db.read(|c| todos::list(c, user, false))
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect()
    };
    // Due dates first (soonest first), then undated.
    assert_eq!(titles(&ana_id), ["Call Mercury", "Count the warehouse", "Ana only"]);
    assert_eq!(titles(&ben_id), ["Call Mercury", "Count the warehouse"]);
    assert_eq!(titles(&cy_id), ["Count the warehouse"]);
    assert_eq!(db.read(|c| todos::open_count(c, &ben_id)).unwrap(), 2);

    let id = for_ben.clone();
    db.write(ben.clone(), move |tx| todos::set_done(tx, &id, true))
        .unwrap();
    assert_eq!(titles(&ben_id), ["Count the warehouse"]);
    let with_done = db.read(|c| todos::list(c, &ben_id, true)).unwrap();
    assert!(with_done.iter().any(|t| t.id == for_ben && t.done));

    // Cy can't delete Ana's shared to-do; Ben (assignee) and admins can.
    let id = shared.clone();
    let cy = cy_id.clone();
    assert!(
        db.write(Actor::user(cy_id.clone()), move |tx| todos::delete(tx, &id, &cy, false))
            .is_err()
    );
    let (id, b) = (for_ben.clone(), ben_id.clone());
    db.write(ben, move |tx| todos::delete(tx, &id, &b, false))
        .unwrap();
    let (id, c) = (shared.clone(), cy_id.clone());
    db.write(Actor::user(cy_id), move |tx| todos::delete(tx, &id, &c, true))
        .unwrap();
    assert_eq!(titles(&ana_id), ["Ana only"]);
}
