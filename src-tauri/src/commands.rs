#[tauri::command]
#[specta::specta]
pub fn greet(name: String) -> String {
    sanco_core::greeting(&name)
}
