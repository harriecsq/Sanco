//! To-dos on the Dashboard: personal (assigned to someone) or shared (no
//! assignee). The app also writes some itself, e.g. when a document number
//! was renumbered or typed twice; those point at the document
//! (`ref_tbl`, `ref_id`).
//!
//! Everyone sees shared to-dos, the ones assigned to them and the ones they
//! assigned to others. Anyone who sees a to-do may tick it off or edit it;
//! only its creator, its assignee or an admin may delete it.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::{date_opt, require_text, text_opt};
use crate::error::{Error, Result};
use crate::repo::WriteTx;

const TODOS: &str = "todos";
const MAX_TITLE: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct TodoInput {
    pub id: Option<String>,
    pub title: String,
    pub notes: String,
    /// `YYYY-MM-DD`, or blank.
    pub due: Option<String>,
    /// `None` = shared with everyone.
    pub assignee_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct TodoInfo {
    pub id: String,
    pub title: String,
    pub notes: Option<String>,
    pub due: Option<String>,
    pub assignee_id: Option<String>,
    pub assignee_name: Option<String>,
    pub done: bool,
    /// The document this to-do is about.
    pub ref_tbl: Option<String>,
    pub ref_id: Option<String>,
    /// That document's number, if it has one.
    pub ref_no: Option<String>,
    pub created_at: i64,
    /// `None` for to-dos the app wrote.
    pub created_by: Option<String>,
    pub created_by_name: Option<String>,
}

/// To-dos `user_id` sees: the open ones by due date, or with `archived` the
/// ticked-off ones, most recently changed first.
pub fn list(conn: &Connection, user_id: &str, archived: bool) -> Result<Vec<TodoInfo>> {
    let order = if archived {
        "t._hlc DESC, t.id"
    } else {
        "t.due IS NULL, t.due, t.created_at DESC"
    };
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT t.id, t.title, t.notes, t.due, t.assignee_id,
                COALESCE(a.display_name, a.username), COALESCE(t.done, 0),
                t.ref_tbl, t.ref_id,
                (SELECT MIN(n.no) FROM doc_numbers n
                 WHERE n.ref_id = t.ref_id AND n._deleted = 0),
                t.created_at, t.created_by, COALESCE(c.display_name, c.username)
         FROM todos t
         LEFT JOIN users a ON a.id = t.assignee_id
         LEFT JOIN users c ON c.id = t.created_by
         WHERE t._deleted = 0
           AND (t.assignee_id IS NULL OR t.assignee_id = ?1 OR t.created_by = ?1)
           AND (COALESCE(t.done, 0) != 0) = ?2
         ORDER BY {order}"
    ))?;
    let rows = stmt.query_map(rusqlite::params![user_id, archived], |r| {
        Ok(TodoInfo {
            id: r.get(0)?,
            title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            notes: r.get(2)?,
            due: r.get(3)?,
            assignee_id: r.get(4)?,
            assignee_name: r.get(5)?,
            done: r.get::<_, i64>(6)? != 0,
            ref_tbl: r.get(7)?,
            ref_id: r.get(8)?,
            ref_no: r.get(9)?,
            created_at: r.get(10)?,
            created_by: r.get(11)?,
            created_by_name: r.get(12)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Open to-dos `user_id` sees.
pub fn open_count(conn: &Connection, user_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM todos
         WHERE _deleted = 0 AND COALESCE(done, 0) = 0
           AND (assignee_id IS NULL OR assignee_id = ?1 OR created_by = ?1)",
        [user_id],
        |r| r.get(0),
    )?)
}

/// Creates or edits a to-do. Returns its id.
pub fn save(tx: &mut WriteTx, input: &TodoInput) -> Result<String> {
    let title = require_text(&input.title, "a title")?;
    if title.chars().count() > MAX_TITLE {
        return Err(Error::Invalid(format!(
            "keep the title under {MAX_TITLE} characters"
        )));
    }
    let due = date_opt(input.due.as_deref(), "due date")?;
    let assignee = input
        .assignee_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(a) = assignee {
        let active: Option<i64> = tx
            .conn()
            .query_row(
                "SELECT COALESCE(active, 0) FROM users WHERE id = ?1 AND _deleted = 0",
                [a],
                |r| r.get(0),
            )
            .optional()?;
        if active != Some(1) {
            return Err(Error::Invalid("pick an active user, or share it".into()));
        }
    }
    let data = json!({
        "title": title,
        "notes": text_opt(&input.notes),
        "due": due,
        "assignee_id": assignee,
    });
    match &input.id {
        Some(id) => {
            live(tx, id)?;
            tx.update(TODOS, id, data)?;
            Ok(id.clone())
        }
        None => {
            let mut data = data;
            data["done"] = Value::from(0);
            tx.insert(TODOS, data)
        }
    }
}

pub fn set_done(tx: &mut WriteTx, id: &str, done: bool) -> Result<()> {
    live(tx, id)?;
    tx.update(TODOS, id, json!({ "done": i64::from(done) }))?;
    Ok(())
}

/// Deletes a to-do. `user_id` must have created it or be its assignee,
/// unless `admin`.
pub fn delete(tx: &mut WriteTx, id: &str, user_id: &str, admin: bool) -> Result<()> {
    let row = live(tx, id)?;
    let mine = ["created_by", "assignee_id"]
        .iter()
        .any(|c| row.get(*c).and_then(Value::as_str) == Some(user_id));
    if !mine && !admin {
        return Err(Error::Invalid(
            "only whoever created it, its assignee or an admin can delete this to-do".into(),
        ));
    }
    tx.delete(TODOS, id)
}

fn live(tx: &WriteTx, id: &str) -> Result<crate::db::registry::Row> {
    tx.get(TODOS, id)?
        .ok_or_else(|| Error::Invalid("this to-do was deleted".into()))
}
