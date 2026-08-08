//! Pane Tauri application entry point.
//!
//! A shell around `pane_core::Core`: this file wires plugins, the menu, the
//! window, and the bridge between the core event bus and the webview. Every
//! operation lives in the core so the CLI and control server share it.

mod commands;

use std::sync::Arc;

use pane_core::{topics, BootstrapError, Core, CoreConfig};
use tauri::menu::{AboutMetadata, MenuBuilder, SubmenuBuilder};
use tracing_subscriber::EnvFilter;

pub fn run() {
    init_logging();

    let core = match Core::bootstrap(CoreConfig::owning()) {
        Ok(c) => Arc::new(c),
        Err(BootstrapError::AlreadyRunning { data_dir }) => {
            // The instance lock is advisory and kernel-released, so this means
            // a live process really does own the data directory. Two GUIs
            // would otherwise both try to bind 8888 and both write the same
            // SQLite file.
            tracing::error!(
                data_dir = %data_dir.display(),
                "another Pane instance is already running; exiting"
            );
            eprintln!(
                "Pane is already running (data directory: {}).",
                data_dir.display()
            );
            std::process::exit(1);
        }
        Err(e) => panic!("failed to bootstrap app state: {e}"),
    };

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
        .plugin(tauri_plugin_clipboard_manager::init())
        // Persist & restore the main window's size and position across
        // launches. Scoped to the "main" window via with_filter so the
        // per-device logcat windows (label "logcat-{serial}") keep their
        // own fixed 1100x720 first-open geometry from logcat_open.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_filter(|label| label == "main")
                .build(),
        )
        .manage(core)
        .invoke_handler(tauri::generate_handler![
            // proxy
            commands::proxy::start,
            commands::proxy::stop,
            commands::proxy::status,
            // host capture ("Capture this Mac")
            commands::host::host_capture_enable,
            commands::host::host_capture_disable,
            commands::host::host_capture_status,
            // logcat
            commands::logcat::logcat_open,
            commands::logcat::android_pid_names,
            commands::logcat::logcat_write_export,
            commands::logcat::logcat_query,
            commands::logcat::logcat_query_older,
            commands::logcat::logcat_new_count,
            commands::logcat::logcat_clear,
            commands::logcat::logcat_export,
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
            commands::captures::captures_export_write,
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
            commands::rules::rules_set_enabled_bulk,
            commands::rules::rule_set_priority,
            commands::rules::collections_list,
            commands::rules::collection_upsert,
            commands::rules::collection_delete,
            commands::rules::collection_set_enabled,
            commands::rules::collection_set_priority,
            commands::rules::rules_export_write,
            commands::rules::rules_import_read,
        ])
        .setup(|app| {
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "Pane starting");
            // Clear any stale 127.0.0.1:8888 system-proxy pointer left by a
            // previous crash (SIGKILL skips our exit handler), so the user
            // isn't stranded offline. No-op if nothing stale. macOS-only.
            pane_core::host_proxy::self_heal_on_start();
            if let Err(e) = install_app_menu(app.handle()) {
                tracing::warn!(error = %e, "failed to install app menu");
            }

            use tauri::Manager;
            let core = app.state::<Arc<Core>>().inner().clone();

            // Hand the companion helper APK path to the device manager.
            // Production: bundled into the .app by tauri.conf.json
            // `bundle.resources`. Dev builds don't go through the bundler and
            // resource_dir() returns Err on macOS, so fall back to the core's
            // probe. First non-empty hit wins.
            match resolve_helper_apk(app.handle()) {
                Some(path) => {
                    core.devices.set_android_helper_apk(path.clone());
                    tracing::info!(path = %path.display(), "pane-helper APK registered");
                }
                None => tracing::debug!("pane-helper APK not found in resources or dev paths"),
            }

            // Bridge the core event bus to the webview. One forwarder for the
            // whole app, replacing the per-proxy-start forwarder that used to
            // live in commands/proxy.rs — that one dropped the engine Arc on
            // return, which is why nothing could subscribe afterwards.
            //
            // tauri::async_runtime::spawn, NOT tokio::spawn — Tauri 2's
            // setup() does not run inside a current_thread tokio runtime
            // context, so tokio::spawn panics with "no reactor running" the
            // moment it registers the task. That caused 0.1.37 + 0.1.38 to
            // abort during applicationDidFinishLaunching on every macOS
            // launch. Same rule applies to the background tasks below.
            let app_handle = app.handle().clone();
            let mut rx = core.events.subscribe();
            tauri::async_runtime::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(ev) => forward_to_webview(&app_handle, &ev),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(skipped = n, "webview event forwarder lagged");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            // Local control endpoint, so the `pane` CLI and the MCP server can
            // drive this instance instead of contending with it for the
            // database and port 8888. On by default: requiring a trip to
            // Settings first would make the feature invisible in exactly the
            // case it exists for — "Pane is already open, run `pane captures
            // tail`". PANE_CONTROL=off opts out.
            //
            // The ServeHandle aborts the accept loop when dropped, so it is
            // held for the process lifetime rather than returned.
            if std::env::var("PANE_CONTROL").as_deref() != Ok("off") {
                let control_core = core.clone();
                tauri::async_runtime::spawn(async move {
                    match pane_control::ControlServer::bind(
                        control_core,
                        pane_control::InstanceKind::Gui,
                    )
                    .await
                    {
                        Ok(kept) => {
                            // Both values must outlive setup(): ServeHandle
                            // aborts the accept loop on drop. Parking them in
                            // this task keeps them alive for the process
                            // lifetime while still dropping them cleanly if
                            // the task is ever cancelled.
                            let _kept = kept;
                            std::future::pending::<()>().await;
                        }
                        Err(e) => {
                            // Not a warning: with no endpoint the `pane` CLI and
                            // the MCP server are dead to this instance, and both
                            // report it as "no running instance" — which reads
                            // as *the app* being down, while its window sits
                            // there working. Loud enough to find in the log.
                            tracing::error!(
                                error = %e,
                                "control endpoint unavailable — `pane` CLI and MCP cannot attach"
                            );
                        }
                    }
                });
            }

            // Device watchdog: reconciles paired Android devices with their
            // actual connection state every 5s. Fixes the "unplugged USB →
            // device stuck with dead proxy → no internet" footgun.
            let watchdog_core = core.clone();
            tauri::async_runtime::spawn(async move {
                pane_core::background::device_watchdog(watchdog_core).await;
            });

            // Logcat retention: prune rows older than 24h and trim each device
            // to a row cap. Runs on startup, then every 5 min.
            let retention_core = core.clone();
            tauri::async_runtime::spawn(async move {
                pane_core::background::logcat_retention(retention_core).await;
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Pane")
        .run(|app, event| {
            // Revert the Mac's system proxy on quit so we never leave it
            // pointing at a dead 127.0.0.1:8888. `Exit` covers the broadest
            // set of quit paths (menu Quit, Cmd-Q, last window closed). This
            // runs without a tokio reactor, which is why host_proxy uses
            // synchronous std::process::Command. SIGKILL bypasses this — that
            // case is handled by self_heal_on_start on the next launch.
            if let tauri::RunEvent::Exit = event {
                use tauri::Manager;
                let core = app.state::<Arc<Core>>();
                if let Err(e) = pane_core::host_proxy::disable(&core) {
                    tracing::warn!(error = %e, "failed to revert host proxy on exit");
                }
                // Drop the control socket and its metadata so the next start
                // does not have to treat them as crash residue.
                pane_control::discovery::cleanup(core.data_dir());
            }
        });
}

/// Relay one core event to the webview, preserving the topic names the
/// frontend already listens for.
///
/// Logcat is the exception: its events are scoped to the matching
/// `logcat-{serial}` window rather than broadcast, so the main window never
/// sees the firehose — the behaviour the old inline callback had.
fn forward_to_webview(app: &tauri::AppHandle, ev: &pane_core::CoreEvent) {
    use tauri::{Emitter, Manager};

    match ev.topic.as_str() {
        topics::LOGCAT_APPENDED | topics::LOGCAT_ERROR => {
            let Some(serial) = ev.payload.get("serial").and_then(|v| v.as_str()) else {
                return;
            };
            let Some(window) = app.get_webview_window(&commands::logcat::label_for(serial)) else {
                return; // window already closed
            };
            let result = if ev.topic == topics::LOGCAT_APPENDED {
                window.emit("logcat://appended", ev.payload.get("inserted").cloned())
            } else {
                window.emit(
                    "logcat://error",
                    serde_json::json!({ "message": ev.payload.get("message").cloned() }),
                )
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, "logcat: emit failed (window gone?)");
            }
        }
        _ => {
            let _ = app.emit(&ev.topic, ev.payload.clone());
        }
    }
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

    // Designate the Window submenu as the macOS application Windows menu.
    // AppKit then auto-appends an entry per open window (main + each Logcat
    // window) and handles switching between them — without this the menu is
    // just Minimize/Zoom/Close and lists no windows.
    #[cfg(target_os = "macos")]
    window_submenu.set_as_windows_menu_for_nsapp()?;

    Ok(())
}

fn init_logging() {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer};
    // MYCHARLES_LOG is the historical name; PANE_LOG matches the product.
    // Both work, PANE_LOG wins.
    let filter = EnvFilter::try_from_env("PANE_LOG")
        .or_else(|_| EnvFilter::try_from_env("MYCHARLES_LOG"))
        .unwrap_or_else(|_| EnvFilter::new("info,pane=debug,pane_engine_mitm=debug"));
    let stdout_layer = fmt::layer().with_target(true);

    // GUI launches of Pane.app have no terminal — stdout logs vanish. Mirror
    // them to ~/Library/Application Support/.../pane.log so users can attach a
    // log to a bug report. tracing-appender keeps the file handle on a
    // dedicated writer thread, which is the only safe way to satisfy
    // `MakeWriter` without re-opening the file per record. The worker guard is
    // leaked because there's no shutdown hook in Tauri's builder; dropping it
    // would silently swallow the trailing log buffer.
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

/// Find the companion helper APK at runtime. Production builds bundle it via
/// `bundle.resources` and we get it back from `resource_dir()`. Dev builds
/// don't go through the bundler, so fall back to the core's probe (which also
/// honours `$PANE_HELPER_APK`).
fn resolve_helper_apk(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    if let Ok(res_dir) = app.path().resource_dir() {
        let p = res_dir.join("binaries").join("pane-helper.apk");
        if pane_core::helper_apk::is_non_empty(&p) {
            return Some(p);
        }
    }
    pane_core::helper_apk::probe()
}

fn log_file_appender() -> Option<tracing_appender::non_blocking::NonBlocking> {
    let dir = pane_core::default_data_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let file_appender = tracing_appender::rolling::never(dir, "pane.log");
    let (nb, guard) = tracing_appender::non_blocking(file_appender);
    Box::leak(Box::new(guard));
    Some(nb)
}
