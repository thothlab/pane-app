//! Pane Tauri application entry point.
//!
//! Wires together the proxy engine, storage, devices, IPC commands and the
//! frontend window. Domain logic lives in workspace crates; this file is glue.

mod commands;
mod state;

use state::AppState;
use tauri::menu::{AboutMetadata, MenuBuilder, SubmenuBuilder};
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
            // rules
            commands::rules::rules_list,
            commands::rules::rule_get,
            commands::rules::rule_upsert,
            commands::rules::rule_delete,
            commands::rules::rule_set_enabled,
            commands::rules::collections_list,
            commands::rules::collection_upsert,
            commands::rules::collection_delete,
            commands::rules::collection_set_enabled,
        ])
        .setup(|app| {
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "Pane starting");
            if let Err(e) = install_app_menu(app.handle()) {
                tracing::warn!(error = %e, "failed to install app menu");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Pane");
}

/// Build the application menu so the About dialog shows the Pane icon and
/// version (Tauri's default About is the macOS folder icon). The icon comes
/// from `bundle.icon` in tauri.conf.json — tauri-build compiled it into the
/// binary, and `default_window_icon()` hands it back to us.
fn install_app_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    let icon = app.default_window_icon().cloned();

    let about = AboutMetadata {
        name: Some("Pane".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        icon,
        ..Default::default()
    };

    let app_submenu = SubmenuBuilder::new(app, "Pane")
        .about(Some(about))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let edit_submenu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view_submenu = SubmenuBuilder::new(app, "View").fullscreen().build()?;

    let window_submenu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()?;

    let menu = MenuBuilder::new(app)
        .items(&[&app_submenu, &edit_submenu, &view_submenu, &window_submenu])
        .build()?;

    app.set_menu(menu)?;
    Ok(())
}

fn init_logging() {
    let filter = EnvFilter::try_from_env("MYCHARLES_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,pane=debug"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}
