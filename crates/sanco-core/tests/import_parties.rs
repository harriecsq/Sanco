//! The one-time client and supplier import.

mod common;

use common::TestPeer;
use sanco_core::domain::clients;
use sanco_core::domain::code_groups::CodeGroup;
use sanco_core::domain::import_parties::{
    self, ImportClient, ImportSupplier, PartiesPlan, series_list,
};
use sanco_core::domain::suppliers;
use sanco_core::domain::users::{self, NewUser};
use sanco_core::sync::numbering::{self, Reservation};
use sanco_core::{Actor, Db, Error};

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

fn group(key: &str, prefix: &str, pad: u32) -> CodeGroup {
    CodeGroup {
        key: key.into(),
        prefix: prefix.into(),
        pad,
        active: true,
    }
}

fn client(group: &str, name: &str) -> ImportClient {
    ImportClient {
        group: group.into(),
        name: name.into(),
    }
}

fn supplier(group: &str, name: &str, country: &str, currency: &str) -> ImportSupplier {
    ImportSupplier {
        group: group.into(),
        name: name.into(),
        country: country.into(),
        currency: currency.into(),
        address: String::new(),
        phone: String::new(),
    }
}

fn plan() -> PartiesPlan {
    PartiesPlan {
        client_groups: vec![group("NCR", "NCR", 4), group("REG", "REG", 4)],
        supplier_groups: vec![group("TWN", "SUP-TWN", 3), group("PHP", "SUP-PHP", 3)],
        clients: vec![
            client("NCR", "Zanoson"),
            client("REG", "Sanvicare"),
            client("NCR", "Medshoppe"),
        ],
        suppliers: vec![
            supplier("TWN", "Gentry Way", "TW", "USD"),
            supplier("PHP", "Nutex", "OTHER", "PHP"),
            supplier("TWN", "Sturdy", "tw", "usd"),
        ],
    }
}

/// Prepares the groups and one reservation per record, as the command does.
fn reservations(db: &Db, actor: &Actor, plan: &PartiesPlan) -> Vec<Reservation> {
    let p = plan.clone();
    db.write(actor.clone(), move |tx| {
        import_parties::apply_groups(tx, &p)
    })
    .unwrap();
    series_list(plan)
        .iter()
        .map(|s| {
            let id = db.new_id().unwrap();
            numbering::reserve(db, &[], s, &id).unwrap()
        })
        .collect()
}

#[test]
fn imports_clients_and_suppliers_into_their_groups() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let plan = plan();

    let sum = a.db.read(|c| import_parties::validate(c, &plan)).unwrap();
    assert_eq!((sum.clients, sum.suppliers), (3, 3));

    let res = reservations(&a.db, &actor, &plan);
    let p = plan.clone();
    a.db.write(actor, move |tx| import_parties::apply(tx, &p, &res))
        .unwrap();

    let codes: Vec<(String, String)> =
        a.db.read(clients::list)
            .unwrap()
            .into_iter()
            .map(|c| (c.name, c.code.unwrap()))
            .collect();
    assert!(codes.contains(&("Zanoson".into(), "NCR-0001".into())));
    assert!(codes.contains(&("Sanvicare".into(), "REG-0001".into())));
    assert!(codes.contains(&("Medshoppe".into(), "NCR-0002".into())));

    let sup = a.db.read(suppliers::list).unwrap();
    let code = |name: &str| {
        sup.iter()
            .find(|s| s.name == name)
            .and_then(|s| s.code.clone())
            .unwrap()
    };
    assert_eq!(code("Gentry Way"), "SUP-TWN-001");
    assert_eq!(code("Nutex"), "SUP-PHP-001");
    assert_eq!(code("Sturdy"), "SUP-TWN-002");
    let sturdy = sup.iter().find(|s| s.name == "Sturdy").unwrap();
    assert_eq!(
        (sturdy.country.as_str(), sturdy.currency.as_str()),
        ("TW", "USD")
    );
}

#[test]
fn refuses_a_second_run_and_bad_plans() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let plan = plan();

    // Bad plans, before anything is written.
    let mut unknown_group = plan.clone();
    unknown_group.clients.push(client("XYZ", "Nobody"));
    let mut bad_country = plan.clone();
    bad_country.suppliers[0].country = "MARS".into();
    let mut no_name = plan.clone();
    no_name.clients[0].name = "  ".into();
    let mut repeated_prefix = plan.clone();
    repeated_prefix.client_groups[1].prefix = "ncr".into();
    for bad in [unknown_group, bad_country, no_name, repeated_prefix] {
        let r = a.db.read(|c| import_parties::validate(c, &bad));
        assert!(matches!(r, Err(Error::Invalid(_))));
    }

    let res = reservations(&a.db, &actor, &plan);
    let p = plan.clone();
    a.db.write(actor.clone(), move |tx| import_parties::apply(tx, &p, &res))
        .unwrap();
    // Now the database isn't empty.
    let r = a.db.read(|c| import_parties::validate(c, &plan));
    assert!(matches!(r, Err(Error::Invalid(_))));
}

#[test]
fn a_wrong_group_for_a_reservation_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let plan = plan();
    let mut res = reservations(&a.db, &actor, &plan);
    res.swap(0, 1); // a NCR client given a REG code
    let p = plan.clone();
    let r =
        a.db.write(actor, move |tx| import_parties::apply(tx, &p, &res));
    assert!(matches!(r, Err(Error::Invalid(_))));
    assert!(a.db.read(clients::list).unwrap().is_empty());
}

/// Dry run of a real import file: `PARTIES_JSON=path cargo test -p sanco-core
/// --test import_parties dry_run -- --ignored --nocapture`.
#[test]
#[ignore]
fn dry_run_of_a_real_file() {
    let Ok(path) = std::env::var("PARTIES_JSON") else {
        return;
    };
    let json = std::fs::read_to_string(path).unwrap();
    let plan: PartiesPlan = serde_json::from_str(&json).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let sum = a.db.read(|c| import_parties::validate(c, &plan)).unwrap();
    let res = reservations(&a.db, &actor, &plan);
    let p = plan.clone();
    a.db.write(actor, move |tx| import_parties::apply(tx, &p, &res))
        .unwrap();
    println!("{sum:?}");
    let list = a.db.read(clients::list).unwrap();
    let sup = a.db.read(suppliers::list).unwrap();
    let mut codes: Vec<String> = list.iter().filter_map(|c| c.code.clone()).collect();
    codes.sort();
    println!(
        "clients {} first {:?} last {:?}",
        codes.len(),
        codes.first(),
        codes.last()
    );
    for g in ["NCR", "REG"] {
        let n = codes.iter().filter(|c| c.starts_with(g)).count();
        println!(
            "  {g}: {n}, highest {:?}",
            codes.iter().filter(|c| c.starts_with(g)).last()
        );
    }
    let mut scodes: Vec<String> = sup.iter().filter_map(|s| s.code.clone()).collect();
    scodes.sort();
    println!("suppliers {}", scodes.len());
    for g in ["SUP-CHN", "SUP-PHP", "SUP-TWN"] {
        let own: Vec<&String> = scodes.iter().filter(|c| c.starts_with(g)).collect();
        println!("  {g}: {}, {:?}..{:?}", own.len(), own.first(), own.last());
    }
    assert!(list.iter().all(|c| !c.duplicate_name));
    let dup: Vec<_> = sup
        .iter()
        .filter(|s| s.duplicate_name)
        .map(|s| &s.name)
        .collect();
    println!("duplicate supplier names: {dup:?}");
}
