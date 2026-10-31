//! Sign-in and permissions.
//!
//! Users get one or more roles. A role grants actions per module plus
//! sensitive fields; a user's permissions are the union of their roles.
//! Permissions are checked in the shell on every command, and loaded fresh
//! each time, so a role edited (or a user deactivated) on another PC takes
//! effect as soon as it syncs.

use std::collections::{BTreeMap, BTreeSet};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::Db;
use crate::error::{Error, Result};
use crate::repo::Actor;

pub const MIN_PASSWORD_LEN: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Module {
    Dashboard,
    Bookings,
    Accounting,
    Items,
    Inventory,
    Orders,
    Clients,
    Reports,
    Hr,
    Settings,
}

impl Module {
    pub const ALL: [Module; 10] = [
        Module::Dashboard,
        Module::Bookings,
        Module::Accounting,
        Module::Items,
        Module::Inventory,
        Module::Orders,
        Module::Clients,
        Module::Reports,
        Module::Hr,
        Module::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Module::Dashboard => "Dashboard",
            Module::Bookings => "Booking",
            Module::Accounting => "Accounting",
            Module::Items => "Items",
            Module::Inventory => "Inventory",
            Module::Orders => "Order",
            Module::Clients => "Clients",
            Module::Reports => "Reports",
            Module::Hr => "HR",
            Module::Settings => "Settings",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Action {
    View,
    Create,
    Edit,
    Delete,
    Approve,
}

impl Action {
    pub const ALL: [Action; 5] = [
        Action::View,
        Action::Create,
        Action::Edit,
        Action::Delete,
        Action::Approve,
    ];

    pub fn verb(self) -> &'static str {
        match self {
            Action::View => "view",
            Action::Create => "create in",
            Action::Edit => "edit in",
            Action::Delete => "delete in",
            Action::Approve => "approve in",
        }
    }
}

/// Values only some roles may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// Supplier prices, FX, charges and landed cost.
    Costs,
    /// Margin percentages and margin amounts.
    Margins,
}

impl Field {
    pub const ALL: [Field; 2] = [Field::Costs, Field::Margins];

    pub fn label(self) -> &'static str {
        match self {
            Field::Costs => "Item costs",
            Field::Margins => "Margins",
        }
    }
}

/// What a role grants, stored as `roles.perms_json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Perms {
    /// Everything, including modules added later. Also required for users,
    /// roles, join codes and restores.
    #[serde(default)]
    pub admin: bool,
    /// Grants for modules this version doesn't know (a removed one, such as
    /// Business Development, or one added later) are skipped when read.
    /// `parse` skips them; a plain serde read of an unknown one fails.
    #[serde(default)]
    pub modules: BTreeMap<Module, BTreeSet<Action>>,
    #[serde(default)]
    pub fields: BTreeSet<Field>,
}

impl Perms {
    pub fn admin() -> Self {
        Self {
            admin: true,
            ..Self::default()
        }
    }

    /// Every action on every module except Settings and HR (salaries), no
    /// sensitive fields.
    pub fn staff() -> Self {
        let modules = Module::ALL
            .into_iter()
            .filter(|m| !matches!(m, Module::Settings | Module::Hr))
            .map(|m| (m, Action::ALL.into_iter().collect()))
            .collect();
        Self {
            admin: false,
            modules,
            fields: BTreeSet::new(),
        }
    }

    pub fn can(&self, module: Module, action: Action) -> bool {
        self.admin
            || self
                .modules
                .get(&module)
                .is_some_and(|a| a.contains(&action))
    }

    pub fn sees(&self, field: Field) -> bool {
        self.admin || self.fields.contains(&field)
    }

    /// Adds everything `other` grants.
    pub fn merge(&mut self, other: &Perms) {
        self.admin |= other.admin;
        for (m, actions) in &other.modules {
            self.modules.entry(*m).or_default().extend(actions);
        }
        self.fields.extend(&other.fields);
    }

    /// Drops empty module entries, and implied grants: any action needs view.
    pub fn normalized(mut self) -> Self {
        for actions in self.modules.values_mut() {
            if !actions.is_empty() {
                actions.insert(Action::View);
            }
        }
        self.modules.retain(|_, a| !a.is_empty());
        self
    }

    /// Parses `perms_json`; anything unreadable grants nothing.
    pub fn parse(json: Option<&str>) -> Self {
        let Some(mut v) = json.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        else {
            return Self::default();
        };
        if let Some(modules) = v.get_mut("modules").and_then(|m| m.as_object_mut()) {
            modules.retain(|k, _| serde_json::from_value::<Module>(json!(k)).is_ok());
        }
        serde_json::from_value(v).unwrap_or_default()
    }
}

/// The signed-in user, with permissions as of now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SessionUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub roles: Vec<String>,
    pub perms: Perms,
}

impl SessionUser {
    pub fn actor(&self) -> Actor {
        Actor::user(self.id.clone())
    }

    pub fn require(&self, module: Module, action: Action) -> Result<()> {
        if self.perms.can(module, action) {
            Ok(())
        } else {
            Err(Error::Forbidden(format!(
                "{} {}",
                action.verb(),
                module.label()
            )))
        }
    }

    pub fn require_admin(&self) -> Result<()> {
        if self.perms.admin {
            Ok(())
        } else {
            Err(Error::Forbidden("do this (admins only)".into()))
        }
    }
}

/// An active, live user and their combined permissions.
pub fn load_user(conn: &Connection, user_id: &str) -> Result<Option<SessionUser>> {
    let user: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT username, display_name FROM users
             WHERE id = ?1 AND _deleted = 0 AND active = 1",
            [user_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((username, display_name)) = user else {
        return Ok(None);
    };
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT r.name, r.perms_json FROM user_roles ur
         JOIN roles r ON r.id = ur.role_id AND r._deleted = 0
         WHERE ur.user_id = ?1 AND ur._deleted = 0
         ORDER BY r.name",
    )?;
    let mut roles = Vec::new();
    let mut perms = Perms::default();
    let rows = stmt.query_map([user_id], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
        ))
    })?;
    for row in rows {
        let (name, json) = row?;
        perms.merge(&Perms::parse(json.as_deref()));
        roles.push(name.unwrap_or_default());
    }
    Ok(Some(SessionUser {
        id: user_id.to_string(),
        display_name: display_name.unwrap_or_else(|| username.clone()),
        username,
        roles,
        perms,
    }))
}

/// Checks a username and password and records the sign-in in the audit log.
pub fn login(db: &Db, username: &str, password: &str) -> Result<SessionUser> {
    let username = normalize_username(username);
    let candidates: Vec<(String, Option<String>)> = db.read(|c| {
        let mut stmt = c.prepare(
            "SELECT id, pw_hash FROM users
             WHERE username = ?1 AND _deleted = 0 AND active = 1 ORDER BY id",
        )?;
        let rows = stmt.query_map([&username], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    })?;
    // Usernames can repeat after an offline merge; take the one that matches.
    let id = candidates
        .into_iter()
        .find(|(_, hash)| {
            hash.as_deref()
                .is_some_and(|h| verify_password(password, h))
        })
        .map(|(id, _)| id)
        .ok_or(Error::BadLogin)?;
    let user = db.read(|c| load_user(c, &id))?.ok_or(Error::BadLogin)?;
    let row_id = id.clone();
    db.write(user.actor(), move |tx| {
        tx.log_event("login", "users", &row_id, json!({}))
    })?;
    Ok(user)
}

/// Records a sign-out in the audit log.
pub fn logout(db: &Db, user_id: &str) -> Result<()> {
    let id = user_id.to_string();
    db.write(Actor::user(id.clone()), move |tx| {
        tx.log_event("logout", "users", &id, json!({}))
    })
}

/// Argon2id PHC string. Slow on purpose (tens of ms): call it before
/// `Db::write`, not inside, so the writer thread isn't held up.
pub fn hash_password(password: &str) -> Result<String> {
    check_password(password)?;
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| Error::Invalid(format!("could not hash the password: {e}")))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    Argon2::default()
        .verify_password(password.as_bytes(), hash)
        .is_ok()
}

pub fn check_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(Error::Invalid(format!(
            "the password needs at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

pub fn normalize_username(username: &str) -> String {
    username.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perms_union_and_checks() {
        let mut p = Perms::default();
        assert!(!p.can(Module::Clients, Action::View));
        p.merge(&Perms::staff());
        assert!(p.can(Module::Clients, Action::Delete));
        assert!(!p.can(Module::Settings, Action::View));
        assert!(!p.can(Module::Hr, Action::View));
        assert!(!p.sees(Field::Costs));
        p.merge(&Perms {
            fields: [Field::Costs].into(),
            ..Perms::default()
        });
        assert!(p.sees(Field::Costs));
        assert!(Perms::admin().can(Module::Settings, Action::Approve));
        assert!(Perms::admin().sees(Field::Margins));
    }

    #[test]
    fn perms_json_round_trip_and_normalize() {
        let p = Perms {
            admin: false,
            modules: [
                (Module::Clients, [Action::Edit].into()),
                (Module::Items, BTreeSet::new()),
            ]
            .into(),
            fields: BTreeSet::new(),
        }
        .normalized();
        assert!(p.can(Module::Clients, Action::View));
        assert!(!p.modules.contains_key(&Module::Items));
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(
            json,
            r#"{"admin":false,"modules":{"clients":["view","edit"]},"fields":[]}"#
        );
        assert_eq!(Perms::parse(Some(&json)), p);
        assert_eq!(Perms::parse(Some("garbage")), Perms::default());
        assert_eq!(Perms::parse(Some("{}")), Perms::default());
    }

    #[test]
    fn perms_skip_unknown_modules() {
        // Roles saved while Business Development existed.
        let p = Perms::parse(Some(
            r#"{"admin":false,"modules":{"bd":["view","create"],"clients":["view"]},"fields":[]}"#,
        ));
        assert!(p.can(Module::Clients, Action::View));
        assert_eq!(p.modules.len(), 1);
    }

    #[test]
    fn password_hashes_verify() {
        let h = hash_password("secret1").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("secret1", &h));
        assert!(!verify_password("secret2", &h));
        assert!(!verify_password("secret1", "not a hash"));
        assert!(hash_password("short").is_err());
    }
}
