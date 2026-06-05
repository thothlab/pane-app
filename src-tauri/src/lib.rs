//! Pane Tauri application entry point.
//!
//! Wires together the proxy engine, storage, devices, IPC commands and the
//! frontend window. Domain logic lives in workspace crates; this file is glue.

mod commands;
mod state;

use state::AppState;
use tauri::menu::{AboutMetadata, MenuBuilder, SubmenuBuilder};
use tauri::Manager;
use tracing_subscriber::EnvFilter;

pub fn run() {
    init_logging();

    let app_state = AppState::bootstrap().expect("failed to bootstrap app state");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        // Auto-updater. Checks plugins.updater.endpoints in tauri.conf.json
        // on demand from the renderer (see `src/lib/updater.ts`). The bundle
        // is verified against the minisign pubkey before install.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
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
            commands::devices::android_tooling_status,
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
            // Hand the helper APK path to AndroidPlatform. Bundled by
            // tauri.conf.json `bundle.resources`. resource_dir() returns
            // None outside a packaged build — fine, dev runs just take
            // the CertInstaller fallback.
            if let Ok(res_dir) = app.path().resource_dir() {
                let apk = res_dir.join("binaries").join("pane-helper.apk");
                if apk.is_file() {
                    let state: tauri::State<AppState> = app.state();
                    state.devices.set_android_helper_apk(apk.clone());
                    tracing::info!(path = %apk.display(), "pane-helper APK registered");
                } else {
                    tracing::debug!(path = %apk.display(), "pane-helper APK not present");
                }
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
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer};
    let filter = EnvFilter::try_from_env("MYCHARLES_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,pane=debug,pane_engine_mitm=debug"));
    let stdout_layer = fmt::layer().with_target(true);

    // GUI launches of Pane.app have no terminal — stdout logs vanish.
    // Mirror them to ~/Library/Application Support/.../pane.log so users
    // can attach a log to a bug report. tracing-appender keeps the file
    // handle on a dedicated writer thread, which is the only safe way to
    // satisfy `MakeWriter` without re-opening the file per record. The
    // worker guard is leaked because there's no shutdown hook in Tauri's
    // builder; dropping it would silently swallow the trailing log buffer.
    let file_layer = log_file_appender().map(|writer| {
        fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .with_target(true)
            .boxed()
    });

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .try_init();
}

fn log_file_appender() -> Option<tracing_appender::non_blocking::NonBlocking> {
    let dirs = directories::ProjectDirs::from("tech", "thothlab", "pane")?;
    let dir = dirs.data_dir();
    std::fs::create_dir_all(dir).ok()?;
    let file_appender = tracing_appender::rolling::never(dir, "pane.log");
    let (nb, guard) = tracing_appender::non_blocking(file_appender);
    Box::leak(Box::new(guard));
    Some(nb)
}
