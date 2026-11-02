mod common;

use common::TestPeer;
use sanco_core::domain::clients::{self, ClientInput};
use sanco_core::domain::code_groups::{self, CodeGroup, Kind};
use sanco_core::domain::suppliers::{self, SupplierInput};
use sanco_core::domain::users::{self, NewUser};
use sanco_core::sync::numbering;
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

fn save(db: &Db, actor: &Actor, kind: Kind, g: CodeGroup) -> Result<(), Error> {
    db.write(actor.clone(), move |tx| code_groups::save(tx, kind, &g))
}

fn client(name: &str) -> ClientInput {
    ClientInput {
        id: None,
        name: name.into(),
        contact_person: String::new(),
        phone: String::new(),
        email: String::new(),
        tin: String::new(),
        address: String::new(),
        terms_days: None,
        margin_tier_bp: None,
        credit_limit_c: None,
        notes: String::new(),
        active: true,
    }
}

fn supplier(name: &str) -> SupplierInput {
    SupplierInput {
        id: None,
        name: name.into(),
        country: "TW".into(),
        currency: "USD".into(),
        contact_person: String::new(),
        phone: String::new(),
        email: String::new(),
        address: String::new(),
        notes: String::new(),
        active: true,
    }
}

fn add_client(db: &Db, actor: &Actor, key: &str, name: &str) -> String {
    let series = db
        .read(|c| code_groups::series_for(c, Kind::Client, key))
        .unwrap();
    let id = db.new_id().unwrap();
    let res = numbering::reserve(db, &[], &series, &id).unwrap();
    let input = client(name);
    db.write(actor.clone(), move |tx| {
        clients::create(tx, &input, &res, true)
    })
    .unwrap()
}

fn code_of(db: &Db, id: &str) -> (Option<String>, Option<String>) {
    let c = db.read(|c| clients::get(c, id)).unwrap().unwrap();
    (c.code, c.code_group)
}

#[test]
fn groups_count_separately() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    save(&a.db, &actor, Kind::Client, group("ncr", "ncr", 4)).unwrap();
    save(&a.db, &actor, Kind::Client, group("REG", "REG", 4)).unwrap();

    let n1 = add_client(&a.db, &actor, "NCR", "A");
    let r1 = add_client(&a.db, &actor, "reg", "B");
    let n2 = add_client(&a.db, &actor, "NCR", "C");
    assert_eq!(
        code_of(&a.db, &n1),
        (Some("NCR-0001".into()), Some("NCR".into()))
    );
    assert_eq!(
        code_of(&a.db, &r1),
        (Some("REG-0001".into()), Some("REG".into()))
    );
    assert_eq!(code_of(&a.db, &n2).0.as_deref(), Some("NCR-0002"));

    let infos = a.db.read(|c| code_groups::infos(c, Kind::Client)).unwrap();
    assert_eq!(infos.len(), 2);
    assert_eq!(infos[0].used, 2);
    assert_eq!(infos[0].example, "NCR-0001");
}

#[test]
fn supplier_prefix_may_have_a_dash() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    save(&a.db, &actor, Kind::Supplier, group("TWN", "SUP-TWN", 3)).unwrap();
    let series =
        a.db.read(|c| code_groups::series_for(c, Kind::Supplier, "TWN"))
            .unwrap();
    let id = a.db.new_id().unwrap();
    let res = numbering::reserve(&a.db, &[], &series, &id).unwrap();
    let input = supplier("Gentry");
    let id =
        a.db.write(actor, move |tx| suppliers::create(tx, &input, &res))
            .unwrap();
    let got = a.db.read(|c| suppliers::get(c, &id)).unwrap().unwrap();
    assert_eq!(got.code.as_deref(), Some("SUP-TWN-001"));
    assert_eq!(got.code_group.as_deref(), Some("TWN"));
}

#[test]
fn unknown_retired_and_bad_groups_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    let r =
        a.db.read(|c| code_groups::series_for(c, Kind::Client, "NCR"));
    assert!(matches!(r, Err(Error::Invalid(_))));

    save(&a.db, &actor, Kind::Client, group("NCR", "NCR", 4)).unwrap();
    // Same prefix in another group, bad characters, bad digits.
    for bad in [
        group("X", "ncr", 4),
        group("A B", "AB", 4),
        group("Y", "-Y", 4),
        group("Z", "Z", 0),
    ] {
        let r = save(&a.db, &actor, Kind::Client, bad);
        assert!(matches!(r, Err(Error::Invalid(_))));
    }
    // The same prefix is fine for the other kind.
    save(&a.db, &actor, Kind::Supplier, group("NCR", "NCR", 4)).unwrap();

    let mut retired = group("NCR", "NCR", 4);
    retired.active = false;
    save(&a.db, &actor, Kind::Client, retired).unwrap();
    let r =
        a.db.read(|c| code_groups::series_for(c, Kind::Client, "NCR"));
    assert!(matches!(r, Err(Error::Invalid(_))));
}

#[test]
fn changing_group_frees_the_old_code() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let actor = setup(&a.db);
    save(&a.db, &actor, Kind::Client, group("NCR", "NCR", 4)).unwrap();
    save(&a.db, &actor, Kind::Client, group("REG", "REG", 4)).unwrap();
    let id = add_client(&a.db, &actor, "NCR", "A");

    let res = numbering::reserve(&a.db, &[], "CL.REG", &id).unwrap();
    a.db.write(actor.clone(), move |tx| clients::change_group(tx, &res))
        .unwrap();
    assert_eq!(
        code_of(&a.db, &id),
        (Some("REG-0001".into()), Some("REG".into()))
    );
    // The freed NCR-0001 is given out again; the client is listed once.
    let other = add_client(&a.db, &actor, "NCR", "B");
    assert_eq!(code_of(&a.db, &other).0.as_deref(), Some("NCR-0001"));
    assert_eq!(a.db.read(clients::list).unwrap().len(), 2);

    // And back again.
    let res = numbering::reserve(&a.db, &[], "CL.NCR", &id).unwrap();
    a.db.write(actor, move |tx| clients::change_group(tx, &res))
        .unwrap();
    assert_eq!(code_of(&a.db, &id).1.as_deref(), Some("NCR"));
}
