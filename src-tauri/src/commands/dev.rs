//! M3 sync test page: plain notes stored as to-dos, so several instances
//! can show that writes reach every PC. Replaced by real screens in M4.

use sanco_core::Actor;
use serde::Serialize;
use serde_json::json;
use specta::Type;
use tauri::State;

use crate::commands::sync::db;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Type)]
pub struct DevNote {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    /// Site that last wrote the note.
    pub site: String,
}

#[tauri::command]
#[specta::specta]
pub async fn dev_notes(state: State<'_, AppState>) -> Result<Vec<DevNote>, String> {
    let db = db(&state).await?;
    db.read(|c| {
        let mut stmt = c.prepare(
            "SELECT id, title, created_at, _site FROM todos
             WHERE _deleted = 0 ORDER BY created_at DESC LIMIT 50",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DevNote {
                id: r.get(0)?,
                title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                created_at: r.get(2)?,
                site: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn dev_add_note(state: State<'_, AppState>, title: String) -> Result<(), String> {
    let db = db(&state).await?;
    tauri::async_runtime::spawn_blocking(move || {
        db.write(Actor::system(), move |tx| {
            tx.insert("todos", json!({ "title": title, "done": 0 }))
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map(drop)
    .map_err(|e| e.to_string())
}
