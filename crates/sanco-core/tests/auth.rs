mod common;

use common::{TestPeer, pull};
use sanco_core::domain::audit::{self, AuditFilter};
use sanco_core::domain::auth::{self, Action, Field, Module, Perms};
use sanco_core::domain::conflicts;
use sanco_core::domain::roles::{self, RoleInput};
use sanco_core::domain::users::{self, NewUser, UserUpdate};
use sanco_core::{Actor, Db, Error};
use serde_json::json;

fn new_user(username: &str, role_ids: Vec<String>) -> NewUser {
    NewUser {
        username: username.into(),
        display_name: String::new(),
        password: "secret1".into(),
        role_ids,
    }
}

fn admin_setup(db: &Db) -> auth::SessionUser {
    users::create_first_admin(
        db,
        NewUser {
            username: " Ana ".into(),
            display_name: "Ana Cruz".into(),
            password: "secret1".into(),
            role_ids: vec![],
        },
    )
    .unwrap()
}

#[test]
fn first_admin_then_login() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 1);
    let admin = admin_setup(&a.db);
    assert_eq!(admin.username, "ana");
    assert_eq!(admin.display_name, "Ana Cruz");
    assert_eq!(admin.roles, vec!["Admin"]);
    assert!(admin.perms.admin);

    // Only once.
    let again = users::create_first_admin(&a.db, new_user("bob", vec![]));
    assert!(matches!(again, Err(Error::Invalid(_))));

    // Usernames are case-insensitive.
    let user = auth::login(&a.db, "ANA", "secret1").unwrap();
    assert_eq!(user.id, admin.id);
    assert!(matches!(auth::login(&a.db, "ana", "wrong!"), Err(Error::BadLogin)));
    assert!(matches!(auth::login(&a.db, "nobody", "secret1"), Err(Error::BadLogin)));

    let logins = a
        .db
        .read(|c| {
            audit::list(
                c,
                &AuditFilter {
                    q: Some("login".into()),
                    ..AuditFilter::default()
                },
            )
        })
        .unwrap();
    assert_eq!(logins.len(), 1);
    assert_eq!(logins[0].user_name.as_deref(), Some("Ana Cruz"));

    // The password hash never reaches the audit log.
    let all = a.db.read(|c| audit::list(c, &AuditFilter::default())).unwrap();
    assert!(all.iter().all(|e| !e.changes_json.contains("argon2")));
}

#[test]
fn staff_permissions_and_admin_guard() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 2);
    let admin = admin_setup(&a.db);

    let staff_id =
        users::create_user(&a.db, admin.actor(), new_user("ben", vec![roles::staff_role_id()]))
            .unwrap();
    let ben = auth::login(&a.db, "ben", "secret1").unwrap();
    assert_eq!(ben.id, staff_id);
    assert!(ben.perms.can(Module::Clients, Action::Create));
    assert!(!ben.perms.can(Module::Settings, Action::View));
    assert!(!ben.perms.sees(Field::Costs));
    assert!(matches!(
        ben.require(Module::Settings, Action::View),
        Err(Error::Forbidden(_))
    ));

    // Duplicate usernames are refused.
    let dup = users::create_user(&a.db, admin.actor(), new_user("BEN", vec![]));
    assert!(matches!(dup, Err(Error::Invalid(_))));

    // A custom role adds cost visibility on top of Staff.
    let costs = a
        .db
        .write(admin.actor(), |tx| {
            roles::save(
                tx,
                RoleInput {
                    id: None,
                    name: "Costing".into(),
                    perms: Perms {
                        modules: [(Module::Items, [Action::Edit].into())].into(),
                        fields: [Field::Costs].into(),
                        ..Perms::default()
                    },
                },
            )
        })
        .unwrap();
    let update = UserUpdate {
        id: staff_id.clone(),
        display_name: "Ben".into(),
        active: true,
        role_ids: vec![roles::staff_role_id(), costs.clone()],
    };
    a.db.write(admin.actor(), {
        let u = update.clone();
        move |tx| users::update(tx, &u)
    })
    .unwrap();
    let ben = a.db.read(|c| auth::load_user(c, &staff_id)).unwrap().unwrap();
    assert!(ben.perms.sees(Field::Costs));
    assert_eq!(ben.roles, vec!["Costing", "Staff"]);

    // Deleting the role takes it away again.
    a.db.write(admin.actor(), move |tx| roles::delete(tx, &costs))
        .unwrap();
    let ben = a.db.read(|c| auth::load_user(c, &staff_id)).unwrap().unwrap();
    assert!(!ben.perms.sees(Field::Costs));

    // The last admin can't lose the role, be deactivated, or lose admin rights.
    let admin_id = admin.id.clone();
    let res = a.db.write(Actor::user(staff_id.clone()), move |tx| {
        users::update(
            tx,
            &UserUpdate {
                id: admin_id,
                display_name: "Ana".into(),
                active: true,
                role_ids: vec![roles::staff_role_id()],
            },
        )
    });
    assert!(matches!(res, Err(Error::Invalid(_))));
    let admin_id = admin.id.clone();
    let res = a.db.write(admin.actor(), move |tx| {
        users::update(
            tx,
            &UserUpdate {
                id: admin_id,
                display_name: "Ana".into(),
                active: false,
                role_ids: vec![roles::admin_role_id()],
            },
        )
    });
    assert!(matches!(res, Err(Error::Invalid(_))));
    let res = a.db.write(admin.actor(), |tx| {
        roles::save(
            tx,
            RoleInput {
                id: Some(roles::admin_role_id()),
                name: "Admin".into(),
                perms: Perms::staff(),
            },
        )
    });
    assert!(matches!(res, Err(Error::Invalid(_))));
    let res = a
        .db
        .write(admin.actor(), |tx| roles::delete(tx, &roles::admin_role_id()));
    assert!(matches!(res, Err(Error::Invalid(_))));
    // The failed writes left everything as it was.
    let admin_now = a.db.read(|c| auth::load_user(c, &admin.id)).unwrap().unwrap();
    assert!(admin_now.perms.admin);

    // Deactivated users can't sign in.
    let u = UserUpdate {
        active: false,
        role_ids: vec![roles::staff_role_id()],
        ..update
    };
    a.db.write(admin.actor(), move |tx| users::update(tx, &u))
        .unwrap();
    assert!(matches!(auth::login(&a.db, "ben", "secret1"), Err(Error::BadLogin)));
    assert!(a.db.read(|c| auth::load_user(c, &staff_id)).unwrap().is_none());
}

#[test]
fn passwords_change() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 3);
    let admin = admin_setup(&a.db);
    let res = users::change_own_password(&a.db, &admin.id, "wrong!", "newpass1");
    assert!(matches!(res, Err(Error::Invalid(_))));
    let res = users::change_own_password(&a.db, &admin.id, "secret1", "new");
    assert!(matches!(res, Err(Error::Invalid(_))));
    users::change_own_password(&a.db, &admin.id, "secret1", "newpass1").unwrap();
    assert!(auth::login(&a.db, "ana", "secret1").is_err());
    assert!(auth::login(&a.db, "ana", "newpass1").is_ok());
    users::set_password(&a.db, admin.actor(), &admin.id, "reset99").unwrap();
    assert!(auth::login(&a.db, "ana", "reset99").is_ok());
}

#[test]
fn users_sync_and_log_in_on_another_pc() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 4);
    let b = TestPeer::new(dir.path(), "b", 5);
    let admin = admin_setup(&a.db);
    pull(&b.db, &a.db, 100);
    let on_b = auth::login(&b.db, "ana", "secret1").unwrap();
    assert_eq!(on_b.id, admin.id);
    assert!(on_b.perms.admin);
}

#[test]
fn conflicts_keep_or_restore() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 6);
    let b = TestPeer::new(dir.path(), "b", 7);
    let admin = admin_setup(&a.db);
    let id = a
        .db
        .write(admin.actor(), |tx| {
            tx.insert("todos", json!({ "title": "Call client", "done": 0 }))
        })
        .unwrap();
    pull(&b.db, &a.db, 100);

    // Concurrent edits: A's later edit wins, B's is recorded as lost.
    let edit = |db: &Db, title: &'static str| {
        let id = id.clone();
        db.write(Actor::user("someone"), move |tx| {
            tx.update("todos", &id, json!({ "title": title }))
        })
        .unwrap();
    };
    edit(&b.db, "Call client at 3pm");
    b.clock.advance(1);
    a.clock.advance(5);
    edit(&a.db, "Call client tomorrow");
    pull(&a.db, &b.db, 100);

    let list = a.db.read(|c| conflicts::list(c, false)).unwrap();
    assert_eq!(list.len(), 1);
    let c = &list[0];
    assert_eq!((c.tbl.as_str(), c.row_id.as_str()), ("todos", id.as_str()));
    assert!(!c.row_deleted);
    assert_eq!(c.fields.len(), 1);
    assert_eq!(c.fields[0].lost, "Call client at 3pm");
    assert_eq!(c.fields[0].current, "Call client tomorrow");

    // Restoring puts the lost values back and marks the conflict resolved.
    let cid = c.id.clone();
    a.db.write(admin.actor(), move |tx| conflicts::resolve(tx, &cid, true))
        .unwrap();
    let title: String = a
        .db
        .read(|c| Ok(c.query_row("SELECT title FROM todos WHERE id = ?1", [&id], |r| r.get(0))?))
        .unwrap();
    assert_eq!(title, "Call client at 3pm");
    assert!(a.db.read(|c| conflicts::list(c, false)).unwrap().is_empty());
    let all = a.db.read(|c| conflicts::list(c, true)).unwrap();
    assert!(all[0].resolved);
    assert_eq!(all[0].resolved_by.as_deref(), Some("Ana Cruz"));

    // The resolution reaches B, and B converges.
    pull(&b.db, &a.db, 100);
    assert!(b.db.read(|c| conflicts::list(c, false)).unwrap().is_empty());
}

#[test]
fn audit_filters() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 8);
    let admin = admin_setup(&a.db);
    a.db.write(admin.actor(), |tx| {
        tx.insert("todos", json!({ "title": "100% done_ok", "done": 0 }))
    })
    .unwrap();
    let list = |f: AuditFilter| a.db.read(|c| audit::list(c, &f)).unwrap();

    let todos = list(AuditFilter {
        tbl: Some("todos".into()),
        ..AuditFilter::default()
    });
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0].action, "insert");
    assert_eq!(todos[0].user_id.as_deref(), Some(admin.id.as_str()));

    // LIKE wildcards in the search are literal.
    assert_eq!(list(AuditFilter { q: Some("100%".into()), ..AuditFilter::default() }).len(), 1);
    assert_eq!(list(AuditFilter { q: Some("0%".into()), ..AuditFilter::default() }).len(), 1);
    assert_eq!(list(AuditFilter { q: Some("1_0".into()), ..AuditFilter::default() }).len(), 0);

    // Dates are Manila days; the test clock is 2026-10-21 08:00 Manila.
    let day = |from: &str, to: &str| {
        list(AuditFilter {
            from: Some(from.into()),
            to: Some(to.into()),
            ..AuditFilter::default()
        })
        .len()
    };
    assert!(day("2026-10-21", "2026-10-21") > 0);
    assert_eq!(day("2026-10-22", "2026-10-30"), 0);
    assert_eq!(day("2026-10-01", "2026-10-20"), 0);

    let mine = list(AuditFilter {
        user_id: Some(admin.id.clone()),
        limit: Some(1),
        ..AuditFilter::default()
    });
    assert_eq!(mine.len(), 1);
}

#[test]
fn removing_a_device() {
    let dir = tempfile::tempdir().unwrap();
    let a = TestPeer::new(dir.path(), "a", 9);
    let b = TestPeer::new(dir.path(), "b", 10);
    let info = |name: &str| sanco_core::sync::peer::DeviceInfo {
        name: name.into(),
        host: Some("127.0.0.1".into()),
        port: 7470,
        app_version: "test".into(),
    };
    a.db.register_device(&info("A")).unwrap();
    b.db.register_device(&info("B")).unwrap();
    pull(&a.db, &b.db, 100);

    let own = a.db.site_id().to_string();
    assert!(matches!(a.db.remove_device(Actor::system(), &own), Err(Error::Invalid(_))));
    a.db.remove_device(Actor::system(), b.db.site_id()).unwrap();
    assert_eq!(a.db.devices().unwrap().len(), 1);

    assert!(!b.db.is_removed().unwrap());
    pull(&b.db, &a.db, 100);
    assert!(b.db.is_removed().unwrap());
    assert!(matches!(b.db.register_device(&info("B")), Err(Error::Deleted { .. })));
}
