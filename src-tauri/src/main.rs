// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `cargo run -p sanco -- --export-bindings` regenerates src/lib/bindings.ts
    // without opening a window (used by `npm run bindings`).
    if std::env::args().any(|a| a == "--export-bindings") {
        sanco_lib::export_bindings();
        return;
    }
    sanco_lib::run();
}
