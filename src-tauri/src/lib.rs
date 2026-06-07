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
            // Hand the companion helper APK path to AndroidPlatform.
            // Production: bundled into the .app by tauri.conf.json
            // `bundle.resources`. Dev (`tauri dev` / `make tauri-dev`):
            // resource_dir() returns Err on macOS, so we also probe the
            // repo's `src-tauri/binaries/` relative to the current exe
            // (target/debug/pane → up three → src-tauri/binaries).
            // First non-empty hit wins.
            use tauri::Manager;
            let apk = resolve_helper_apk(app.handle());
            if let Some(path) = apk {
                let state: tauri::State<AppState> = app.state();
                state.devices.set_android_helper_apk(path.clone());
                tracing::info!(path = %path.display(), "pane-helper APK registered");
            } else {
                tracing::debug!("pane-helper APK not found in resources or dev paths");
            }
            // Spawn device watchdog: polls adb for attached devices every 5s,
            // auto-applies the right thing when a paired phone reconnects.
            // Fixes the "unplugged USB → device stuck with dead proxy → no
            // internet" footgun. When the phone comes back:
            //   - Pane proxy running → re-apply http_proxy + reverse (MITM
            //     resumes seamlessly, no manual Re-sync needed).
            //   - Pane proxy stopped → clear the proxy setting (device gets
            //     its internet back, ready for normal use).
            let app_handle = app.handle().clone();
            // tauri::async_runtime::spawn, NOT tokio::spawn — Tauri 2's
            // setup() does NOT run inside a current_thread tokio runtime
            // context, so `tokio::spawn(...)` panics with "no reactor
            // running" the moment it tries to register the task. Tauri
            // ships its own multi-thread runtime; spawn through that.
            // Caused 0.1.37 + 0.1.38 to abort during
            // applicationDidFinishLaunching on every macOS launch.
            tauri::async_runtime::spawn(async move {
                device_watchdog(app_handle).await;
            });
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

/// Background watchdog that reconciles paired Android devices with their
/// actual connection state. Runs every 5 seconds.
///
/// Why: when the user yanks the USB cable without first stopping the
/// proxy in Pane, the phone's `http_proxy` setting keeps pointing at
/// 127.0.0.1:8888 — but adb reverse is gone, so connections to that
/// port refuse, and the device loses internet. The user can't easily
/// undo this from the phone itself (Samsung settings hide global
/// proxy clear). Watchdog fixes both directions:
///
///   - Phone reconnects + Pane proxy is running → re-apply http_proxy
///     and adb reverse. MITM resumes without the user clicking Re-sync.
///   - Phone reconnects + Pane proxy is NOT running → strip the proxy
///     settings off the device, restoring its internet.
///
/// We only act on devices that are **paired** (have a `device` row),
/// so plugging in a random unrelated phone doesn't touch its settings.
/// Tracking last-seen serials skips the redundant work when nothing
/// changed.
async fn device_watchdog(app: tauri::AppHandle) {
    use std::collections::HashSet;
    use std::time::Duration;
    use tauri::Manager;

    let mut last_seen: HashSet<String> = HashSet::new();
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    // First tick fires immediately; skip it so we don't race app boot.
    interval.tick().await;

    loop {
        interval.tick().await;
        let state: tauri::State<AppState> = app.state();

        // Snapshot what's plugged in right now.
        let attached: HashSet<String> = match state.devices.discover_attached().await {
            Ok(list) => list
                .into_iter()
                .filter(|d| d.platform == "android")
                .map(|d| d.serial)
                .collect(),
            Err(_) => continue, // adb not on PATH or daemon hiccup — skip tick
        };
        if attached == last_seen {
            continue;
        }

        // Newly-connected serials = attached \ last_seen.
        let newly_connected: Vec<String> = attached.difference(&last_seen).cloned().collect();
        last_seen = attached;

        if newly_connected.is_empty() {
            continue;
        }

        // Cross-reference with paired devices.
        let paired_serials: HashSet<String> = state
            .devices
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.platform == "android" && d.connection == "usb")
            .map(|d| d.serial)
            .collect();

        let proxy_running = state.proxy_handle.lock().is_some();
        let ca = state.ca.material();

        for serial in newly_connected {
            if !paired_serials.contains(&serial) {
                continue; // not one of ours, leave alone
            }
            if proxy_running {
                let _ = state
                    .devices
                    .reapply_one_android_proxy(&serial, ca.clone())
                    .await;
                tracing::info!(serial, "watchdog: re-applied proxy on reconnect");
            } else {
                let _ = state.devices.clear_one_android_proxy(&serial).await;
                tracing::info!(serial, "watchdog: cleared stale proxy on reconnect");
            }
        }
    }
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

/// Find the companion helper APK at runtime. Production builds bundle
/// it via tauri.conf.json `bundle.resources` and we get it back from
/// `resource_dir()`. Dev builds (`cargo tauri dev`, `make tauri-dev`)
/// don't go through the bundler — fall back to probing the repo
/// `src-tauri/binaries/pane-helper.apk` relative to `current_exe`.
///
/// Returns the path only if the file exists *and* is non-empty (the
/// committed placeholder is 0 bytes before CI populates it — same
/// shape as `apk_is_present` in pane-android, kept consistent here so
/// dev runs without a real APK silently fall through to "watchdog
/// disabled" instead of trying to install garbage).
fn resolve_helper_apk(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    if let Ok(res_dir) = app.path().resource_dir() {
        let p = res_dir.join("binaries").join("pane-helper.apk");
        if file_is_non_empty(&p) {
            return Some(p);
        }
    }
    // Dev probe: walk up from target/debug/pane (or target/release/pane)
    // to find a sibling `src-tauri/binaries/pane-helper.apk`.
    if let Ok(exe) = std::env::current_exe() {
        // exe = .../target/{debug,release}/pane
        // Want = .../src-tauri/binaries/pane-helper.apk
        // Going up two levels from exe lands at `target/`; one more at
        // the repo root. Then descend into src-tauri/binaries.
        if let Some(repo_root) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            let p = repo_root
                .join("src-tauri")
                .join("binaries")
                .join("pane-helper.apk");
            if file_is_non_empty(&p) {
                return Some(p);
            }
        }
    }
    None
}

fn file_is_non_empty(p: &std::path::Path) -> bool {
    std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)
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
