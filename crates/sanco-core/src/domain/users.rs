//! User accounts. Users are deactivated, never deleted, because rows keep
//! pointing at them (`created_by`, the audit log).

use std::collections::BTreeSet;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::Db;
use crate::domain::auth::{self, SessionUser};
use crate::domain::roles::{self, ROLES, USER_ROLES};
use crate::error::{Error, Result};
use crate::repo::{Actor, WriteTx};

pub const USERS: &str = "users";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub active: bool,
    pub role_ids: Vec<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct NewUser {
    pub username: String,
    pub display_name: String,
    pub password: String,
    pub role_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct UserUpdate {
    pub id: String,
    pub display_name: String,
    pub active: bool,
    pub role_ids: Vec<String>,
}

pub fn list(conn: &Connection) -> Result<Vec<UserInfo>> {
    let mut stmt = conn.prepare(
        "SELECT u.id, u.username, u.display_name, u.active, u.created_at,
                (SELECT json_group_array(role_id) FROM
                   (SELECT DISTINCT ur.role_id FROM user_roles ur
                    JOIN roles r ON r.id = ur.role_id AND r._deleted = 0
                    WHERE ur.user_id = u.id AND ur._deleted = 0 ORDER BY ur.role_id))
         FROM users u WHERE u._deleted = 0
         ORDER BY u.active DESC, u.display_name COLLATE NOCASE, u.username",
    )?;
    let rows = stmt.query_map([], |r| {
        let username: String = r.get::<_, Option<String>>(1)?.unwrap_or_default();
        let roles: String = r.get(5)?;
        Ok(UserInfo {
            id: r.get(0)?,
            display_name: r.get::<_, Option<String>>(2)?.unwrap_or_else(|| username.clone()),
            username,
            active: r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
            created_at: r.get(4)?,
            role_ids: serde_json::from_str(&roles).unwrap_or_default(),
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Whether any live user exists. A new workspace has none until its first
/// admin is created.
pub fn any_user(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM users WHERE _deleted = 0)",
        [],
        |r| r.get(0),
    )?)
}

/// Creates the workspace's first user with the Admin role, and seeds the
/// default roles. Fails once any user exists.
pub fn create_first_admin(db: &Db, input: NewUser) -> Result<SessionUser> {
    let hash = auth::hash_password(&input.password)?;
    let id = db.write(Actor::system(), move |tx| {
        if any_user(tx.conn())? {
            return Err(Error::Invalid("this workspace already has users".into()));
        }
        roles::seed(tx)?;
        crate::domain::seed_defaults_tx(tx)?;
        let input = NewUser {
            role_ids: vec![roles::admin_role_id()],
            ..input
        };
        create(tx, &input, hash)
    })?;
    db.read(|c| auth::load_user(c, &id))?
        .ok_or(Error::NotSignedIn)
}

/// Hashes the password and creates the user.
pub fn create_user(db: &Db, actor: Actor, input: NewUser) -> Result<String> {
    let hash = auth::hash_password(&input.password)?;
    db.write(actor, move |tx| create(tx, &input, hash))
}

/// Sets a new password, hashed here.
pub fn set_password(db: &Db, actor: Actor, user_id: &str, password: &str) -> Result<()> {
    let hash = auth::hash_password(password)?;
    let id = user_id.to_string();
    db.write(actor, move |tx| {
        if tx.get(USERS, &id)?.is_none() {
            return Err(Error::Invalid("that user no longer exists".into()));
        }
        tx.update(USERS, &id, json!({ "pw_hash": hash })).map(drop)
    })
}

/// Changes the signed-in user's own password after checking the old one.
pub fn change_own_password(db: &Db, user_id: &str, old: &str, new: &str) -> Result<()> {
    let hash: Option<String> = db.read(|c| {
        Ok(c.query_row(
            "SELECT pw_hash FROM users WHERE id = ?1 AND _deleted = 0",
            [user_id],
            |r| r.get(0),
        )?)
    })?;
    if !hash.is_some_and(|h| auth::verify_password(old, &h)) {
        return Err(Error::Invalid("the current password is wrong".into()));
    }
    set_password(db, Actor::user(user_id), user_id, new)
}

pub fn create(tx: &mut WriteTx, input: &NewUser, pw_hash: String) -> Result<String> {
    let username = check_username(&input.username)?;
    let taken: bool = tx.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM users WHERE _deleted = 0 AND username = ?1)",
        [&username],
        |r| r.get(0),
    )?;
    if taken {
        return Err(Error::Invalid(format!("the username {username} is taken")));
    }
    let display_name = display_name(&input.display_name, &username);
    let id = tx.insert(
        USERS,
        json!({
            "username": username,
            "display_name": display_name,
            "pw_hash": pw_hash,
            "active": 1,
        }),
    )?;
    set_roles(tx, &id, &input.role_ids)?;
    Ok(id)
}

pub fn update(tx: &mut WriteTx, input: &UserUpdate) -> Result<()> {
    let Some(row) = tx.get(USERS, &input.id)? else {
        return Err(Error::Invalid("that user no longer exists".into()));
    };
    let username = row
        .get("username")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if !input.active && tx.actor().user_id.as_deref() == Some(input.id.as_str()) {
        return Err(Error::Invalid("you can't deactivate your own account".into()));
    }
    tx.update(
        USERS,
        &input.id,
        json!({
            "display_name": display_name(&input.display_name, &username),
            "active": input.active,
        }),
    )?;
    set_roles(tx, &input.id, &input.role_ids)?;
    ensure_admin_remains(tx)
}

/// Gives the user exactly these roles.
fn set_roles(tx: &mut WriteTx, user_id: &str, role_ids: &[String]) -> Result<()> {
    let wanted: BTreeSet<&str> = role_ids.iter().map(String::as_str).collect();
    for role in &wanted {
        if tx.get(ROLES, role)?.is_none() {
            return Err(Error::Invalid("one of the roles no longer exists".into()));
        }
    }
    let current: Vec<(String, String)> = {
        let mut stmt = tx.conn().prepare(
            "SELECT id, role_id FROM user_roles WHERE user_id = ?1 AND _deleted = 0",
        )?;
        let rows = stmt.query_map([user_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut have = BTreeSet::new();
    for (link, role) in &current {
        // Removes links to unwanted roles and duplicates from offline edits.
        if !wanted.contains(role.as_str()) || !have.insert(role.as_str()) {
            tx.delete(USER_ROLES, link)?;
        }
    }
    for role in wanted.difference(&have) {
        tx.insert(USER_ROLES, json!({ "user_id": user_id, "role_id": role }))?;
    }
    Ok(())
}

/// Refuses a change that leaves no active user with an admin role.
pub fn ensure_admin_remains(tx: &WriteTx) -> Result<()> {
    let ok: bool = tx.conn().query_row(
        "SELECT EXISTS (
           SELECT 1 FROM users u
           JOIN user_roles ur ON ur.user_id = u.id AND ur._deleted = 0
           JOIN roles r ON r.id = ur.role_id AND r._deleted = 0
           WHERE u._deleted = 0 AND u.active = 1
             AND json_valid(r.perms_json) AND json_extract(r.perms_json, '$.admin') = 1)",
        [],
        |r| r.get(0),
    )?;
    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(
            "at least one active user must keep an admin role".into(),
        ))
    }
}

pub fn check_username(username: &str) -> Result<String> {
    let u = auth::normalize_username(username);
    let valid_chars = u
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if !(2..=32).contains(&u.len()) || !valid_chars {
        return Err(Error::Invalid(
            "a username needs 2–32 letters, digits, dots, dashes or underscores".into(),
        ));
    }
    Ok(u)
}

fn display_name(name: &str, username: &str) -> String {
    let name = name.trim();
    if name.is_empty() { username } else { name }.to_string()
}
