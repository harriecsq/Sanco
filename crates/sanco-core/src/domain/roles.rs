//! Roles: named permission sets, editable by admins.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::auth::Perms;
use crate::domain::users;
use crate::error::{Error, Result};
use crate::ids;
use crate::repo::WriteTx;

pub const ROLES: &str = "roles";
pub const USER_ROLES: &str = "user_roles";

/// Seeded roles use fixed ids, so a role seeded twice is one row.
pub fn admin_role_id() -> String {
    ids::derived_id("role", "admin")
}

pub fn staff_role_id() -> String {
    ids::derived_id("role", "staff")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RoleInfo {
    pub id: String,
    pub name: String,
    pub perms: Perms,
    /// Active users that have this role.
    pub user_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RoleInput {
    /// `None` creates a new role.
    pub id: Option<String>,
    pub name: String,
    pub perms: Perms,
}

pub fn list(conn: &Connection) -> Result<Vec<RoleInfo>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.name, r.perms_json,
                (SELECT COUNT(DISTINCT ur.user_id) FROM user_roles ur
                 JOIN users u ON u.id = ur.user_id AND u._deleted = 0 AND u.active = 1
                 WHERE ur.role_id = r.id AND ur._deleted = 0)
         FROM roles r WHERE r._deleted = 0
         ORDER BY r.name COLLATE NOCASE, r.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(RoleInfo {
            id: r.get(0)?,
            name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            perms: Perms::parse(r.get::<_, Option<String>>(2)?.as_deref()),
            user_count: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Creates the Admin and Staff roles. Run once, when a workspace is created.
pub fn seed(tx: &mut WriteTx) -> Result<()> {
    for (id, name, perms) in [
        (admin_role_id(), "Admin", Perms::admin()),
        (staff_role_id(), "Staff", Perms::staff()),
    ] {
        if tx.get(ROLES, &id)?.is_none() {
            tx.insert(
                ROLES,
                json!({ "id": id, "name": name, "perms_json": serde_json::to_string(&perms)? }),
            )?;
        }
    }
    Ok(())
}

/// Creates or updates a role. Returns its id.
pub fn save(tx: &mut WriteTx, input: RoleInput) -> Result<String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(Error::Invalid("enter a role name".into()));
    }
    let taken: bool = tx.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM roles WHERE _deleted = 0
                        AND name = ?1 COLLATE NOCASE AND id IS NOT ?2)",
        (&name, &input.id),
        |r| r.get(0),
    )?;
    if taken {
        return Err(Error::Invalid(format!("a role named {name} already exists")));
    }
    let data = json!({
        "name": name,
        "perms_json": serde_json::to_string(&input.perms.normalized())?,
    });
    let id = match input.id {
        Some(id) => {
            if tx.get(ROLES, &id)?.is_none() {
                return Err(Error::Invalid("that role no longer exists".into()));
            }
            tx.update(ROLES, &id, data)?;
            id
        }
        None => tx.insert(ROLES, data)?,
    };
    users::ensure_admin_remains(tx)?;
    Ok(id)
}

/// Deletes a role and takes it away from its users.
pub fn delete(tx: &mut WriteTx, id: &str) -> Result<()> {
    let links: Vec<String> = {
        let mut stmt = tx
            .conn()
            .prepare("SELECT id FROM user_roles WHERE role_id = ?1 AND _deleted = 0")?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for link in links {
        tx.delete(USER_ROLES, &link)?;
    }
    tx.delete(ROLES, id)?;
    users::ensure_admin_remains(tx)
}
