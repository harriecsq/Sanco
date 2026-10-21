mod commands;
mod state;

use specta_typescript::Typescript;
use tauri::Manager;
use tauri_specta::{Builder, collect_commands};

use crate::state::{AppState, Args};

fn specta_builder() -> Builder<tauri::Wry> {
    // i64 money (centavos) stays far below 2^53, so plain JS numbers are exact.
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::sync::workspace_status,
            commands::sync::create_workspace,
            commands::sync::join_workspace,
            commands::sync::invite_code,
            commands::sync::sync_status,
            commands::sync::sync_now,
            commands::dev::dev_notes,
            commands::dev::dev_add_note,
        ])
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
    let args = Args::parse(std::env::args().skip(1));

    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                })
                // Dependencies' request-level logs are noise.
                .level_for("hyper", log::LevelFilter::Warn)
                .level_for("hyper_util", log::LevelFilter::Warn)
                .level_for("reqwest", log::LevelFilter::Warn)
                .build(),
        )
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let default_dir = app.path().app_data_dir()?;
            let state = AppState::new(app.handle().clone(), args, default_dir);
            log::info!(
                "data dir {}, sync port {}",
                state.data_dir.display(),
                state.port
            );
            app.manage(state);
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                handle.state::<AppState>().resume().await;
                use tauri::Emitter;
                let _ = handle.emit("sync-status", ());
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Sanco");
}
