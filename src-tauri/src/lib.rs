mod commands;

use specta_typescript::Typescript;
use tauri_specta::{Builder, collect_commands};

fn specta_builder() -> Builder<tauri::Wry> {
    // i64 money (centavos) stays far below 2^53, so plain JS numbers are exact.
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![commands::greet])
        .dangerously_cast_bigints_to_number()
}

/// Writes `src/lib/bindings.ts`. Runs on every debug start and from the
/// `--export-bindings` flag, so the frontend always matches the commands.
pub fn export_bindings() {
    specta_builder()
        .export(
            Typescript::default(),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../src/lib/bindings.ts"),
        )
        .expect("failed to export TypeScript bindings");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(debug_assertions)]
    export_bindings();

    let builder = specta_builder();

    tauri::Builder::default()
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Sanco");
}

