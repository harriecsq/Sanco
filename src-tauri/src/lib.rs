mod commands;
mod state;
mod tray;

use std::time::Duration;

use specta_typescript::Typescript;
use tauri::{Emitter, Manager, WindowEvent};
use tauri_specta::{Builder, collect_commands};

use crate::state::{AppState, Args};

/// How often the daily backup is checked for (cheap: a file exists check).
const BACKUP_CHECK: Duration = Duration::from_secs(10 * 60);

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
            commands::sync::retry_sync,
            commands::sync::leave_workspace,
            commands::auth::session_info,
            commands::auth::login,
            commands::auth::logout,
            commands::auth::create_first_admin,
            commands::auth::change_password,
            commands::auth::perm_catalog,
            commands::users::list_users,
            commands::users::create_user,
            commands::users::update_user,
            commands::users::reset_password,
            commands::users::list_roles,
            commands::users::save_role,
            commands::users::delete_role,
            commands::users::user_options,
            commands::settings::list_devices,
            commands::settings::remove_device,
            commands::settings::list_conflicts,
            commands::settings::resolve_conflict,
            commands::settings::list_backups,
            commands::settings::backup_now,
            commands::settings::restore_backup,
            commands::settings::audit_log,
            commands::settings::audit_tables,
            commands::settings::pc_settings,
            commands::settings::set_autostart,
            commands::config::company_info,
            commands::config::set_company_info,
            commands::config::pricing_settings,
            commands::config::set_vat_rate,
            commands::config::list_charge_types,
            commands::config::save_charge_type,
            commands::config::delete_charge_type,
            commands::config::number_formats,
            commands::config::set_number_format,
            commands::config::set_number_mode,
            commands::clients::list_clients,
            commands::clients::get_client,
            commands::clients::save_client,
            commands::clients::delete_client,
            commands::clients::client_options,
            commands::suppliers::list_suppliers,
            commands::suppliers::save_supplier,
            commands::suppliers::delete_supplier,
            commands::suppliers::supplier_options,
            commands::locations::list_locations,
            commands::locations::save_location,
            commands::locations::delete_location,
            commands::items::list_items,
            commands::items::get_item,
            commands::items::item_suggestions,
            commands::items::save_item,
            commands::items::delete_item,
            commands::items::get_cost_sheet,
            commands::items::save_cost_sheet,
            commands::items::apply_supplier_price,
            commands::items::list_rates,
            commands::items::save_rate,
            commands::items::price_quote,
            commands::items::item_options,
            commands::inventory::stock_overview,
            commands::inventory::stock_lots,
            commands::inventory::box_stock,
            commands::inventory::expiring_stock,
            commands::inventory::item_stock_moves,
            commands::inventory::list_stock_docs,
            commands::inventory::get_stock_doc,
            commands::inventory::post_stock_doc,
            commands::inventory::void_stock_doc,
            commands::bookings::list_bookings,
            commands::bookings::get_booking,
            commands::bookings::save_booking,
            commands::bookings::set_booking_status,
            commands::bookings::delete_booking,
            commands::delivery::list_drs,
            commands::delivery::get_dr,
            commands::delivery::dr_lots,
            commands::delivery::dr_boxes,
            commands::delivery::request_dr,
            commands::delivery::update_dr,
            commands::delivery::approve_dr,
            commands::delivery::deliver_dr,
            commands::delivery::set_dr_number,
            commands::delivery::cancel_dr,
            commands::delivery::void_dr,
            commands::accounting::list_payments,
            commands::accounting::get_payment,
            commands::accounting::payment_open_drs,
            commands::accounting::save_payment,
            commands::accounting::void_payment,
            commands::accounting::check_step,
            commands::accounting::receivables_report,
            commands::accounting::client_balance,
            commands::accounting::client_receivables,
            commands::bank_accounts::list_bank_accounts,
            commands::bank_accounts::bank_account_moves,
            commands::bank_accounts::save_bank_account,
            commands::bank_accounts::delete_bank_account,
            commands::orders::list_pos,
            commands::orders::get_po,
            commands::orders::save_po,
            commands::orders::set_po_status,
            commands::orders::delete_po,
            commands::orders::po_to_cost_sheets,
            commands::orders::list_po_payments,
            commands::orders::save_po_payment,
            commands::orders::delete_po_payment,
            commands::orders::list_po_receipts,
            commands::orders::get_po_receipt,
            commands::orders::receive_po,
            commands::orders::void_po_receipt,
            commands::dashboard::dashboard,
            commands::dashboard::list_todos,
            commands::dashboard::save_todo,
            commands::dashboard::set_todo_done,
            commands::dashboard::delete_todo,
            commands::reports::report_sales,
            commands::reports::report_margin,
            commands::reports::report_po_spend,
            commands::reports::report_receivables,
            commands::reports::report_payments,
            commands::reports::report_stock,
            commands::reports::save_csv,
            commands::reports::show_in_folder,
            commands::update::update_status,
            commands::update::install_update,
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

    let mut app = tauri::Builder::default();
    // Must be the first plugin. Test instances (`--data-dir`) may run
    // alongside the main one.
    if !args.is_extra_instance() {
        app = app.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }));
    }
    app.plugin(
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
    .plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec!["--minimized"]),
    ))
    .plugin(tauri_plugin_notification::init())
    .plugin(
        // The window starts hidden; `setup` decides whether to show it.
        tauri_plugin_window_state::Builder::default()
            .with_state_flags(
                tauri_plugin_window_state::StateFlags::all()
                    - tauri_plugin_window_state::StateFlags::VISIBLE,
            )
            .build(),
    )
    .invoke_handler(builder.invoke_handler())
    .on_window_event(|window, event| {
        // Closing hides to the tray so sync keeps running. Quit is in the
        // tray menu.
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window.hide();
        }
    })
    .setup(move |app| {
        builder.mount_events(app);
        tray::create(app.handle())?;
        let minimized = args.minimized;
        let default_dir = app.path().app_data_dir()?;
        let state = AppState::new(app.handle().clone(), args, default_dir);
        log::info!(
            "data dir {}, sync port {}",
            state.data_dir.display(),
            state.port
        );
        app.manage(state);
        if !minimized {
            tray::show_main(app.handle());
        }
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            let state = handle.state::<AppState>();
            state.resume().await;
            state
                .started
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = handle.emit("sync-status", ());
            loop {
                state.daily_backup().await;
                tokio::time::sleep(BACKUP_CHECK).await;
            }
        });
        Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running Sanco");
}
