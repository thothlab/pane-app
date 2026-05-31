//! Pane Tauri application entry point.
//!
//! Wires together the proxy engine, storage, devices, IPC commands and the
//! frontend window. Domain logic lives in workspace crates; this file is glue.

mod commands;
mod state;

use state::AppState;
use tracing_subscriber::EnvFilter;

pub fn run() {
    init_logging();

    let app_state = AppState::bootstrap().expect("failed to bootstrap app state");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            // proxy
            commands::proxy::start,
            commands::proxy::stop,
            commands::proxy::status,
            // ca
            commands::ca::current,
            commands::ca::rotate,
            commands::ca::export,
            commands::ca::save_to_file,
            // devices
            commands::devices::list_attached_usb,
            commands::devices::add_ios_usb,
            commands::devices::add_android_usb,
            commands::devices::remove,
            commands::devices::devices_get,
            commands::devices::devices_list,
            // captures
            commands::captures::captures_list,
            commands::captures::captures_get,
            commands::captures::get_body,
            commands::captures::clear,
            commands::captures::export_one,
            // replay
            commands::replay::send,
            // filters
            commands::filters::filters_save,
            commands::filters::filters_list,
            commands::filters::filters_delete,
        ])
        .setup(|app| {
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "Pane starting");
            let _ = app;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Pane");
}

fn init_logging() {
    let filter = EnvFilter::try_from_env("MYCHARLES_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,pane=debug"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}
