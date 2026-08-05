//! Pane Tauri application entry point.
//!
//! Wires together the proxy engine, storage, devices, IPC commands and the
//! frontend window. Domain logic lives in workspace crates; this file is glue.

mod commands;
mod host_proxy;
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
        .manage(app_state)
        // LogcatSessions tracks one shutdown-sender per open logcat
        // window so a second click on the Logcat button can detect
        // and focus the existing window instead of double-spawning
        // the adb subprocess.
        .manage(commands::logcat::LogcatSessions::new())
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
            host_proxy::self_heal_on_start();
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

            // Logcat retention: prune persisted logcat rows older than 24h and
            // trim each device to a per-device row cap. Runs once on startup,
            // then every 5 min. async_runtime::spawn (not tokio::spawn) for the
            // same reason as the watchdog above.
            let retention_state = app.state::<AppState>().storage.clone();
            tauri::async_runtime::spawn(async move {
                const RETENTION_MS: i64 = 24 * 60 * 60 * 1000;
                const PER_DEVICE_CAP: i64 = 2_000_000;
                loop {
                    if let Err(e) =
                        retention_state.prune_logcat(RETENTION_MS, PER_DEVICE_CAP)
                    {
                        tracing::warn!(error = %e, "logcat: prune failed");
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                }
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
                let state: tauri::State<AppState> = app.state();
                if let Err(e) = host_proxy::disable(&state) {
                    tracing::warn!(error = %e, "failed to revert host proxy on exit");
                }
            }
        });
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
    // AppKit then auto-appends an entry per open window (main + each
    // Logcat window) and handles switching between them — without this
    // the menu is just Minimize/Zoom/Close and lists no windows.
    #[cfg(target_os = "macos")]
    window_submenu.set_as_windows_menu_for_nsapp()?;

    Ok(())
}

/// How many consecutive unhealthy probes before we repair a device.
///
/// One miss is not enough: a probe can legitimately catch a device mid-reboot
/// or between an `adb reverse` teardown and its rebuild, and repairing on that
/// would mean a full re-pair (including an APK install) on every hiccup. Two
/// misses is ~10 s of genuinely broken tunnel, which is well below what a user
/// notices as "it stopped working" and well above transient noise.
const UNHEALTHY_STRIKES: u8 = 2;

/// Background watchdog that reconciles paired Android devices with their
/// actual connection state. Runs every 5 seconds.
///
/// Why: the phone's `http_proxy` and the `adb reverse` tunnel can fall out of
/// sync with reality in both directions, and neither side notices on its own.
/// Yank the USB cable without stopping the proxy and the phone keeps pointing
/// at 127.0.0.1:8888 with nothing behind it — no internet, and the user can't
/// easily undo it from the phone (Samsung hides the global proxy setting).
/// Conversely, the tunnel can die while the setting stays, which looks like
/// "Pane just stopped capturing" with no error anywhere.
///
/// So this reconciles **state**, not events:
///
///   - Proxy running + device unhealthy for `UNHEALTHY_STRIKES` ticks
///     → re-apply. Covers a dead reverse, a cleared setting, and a phone that
///     was replugged.
///   - Proxy stopped + device still points at us → clear it, restoring the
///     device's internet.
///   - Proxy stopped + device already clean → do nothing.
///
/// The previous version acted only on the *transition* into `attached`, which
/// had two fatal consequences. On boot `last_seen` is empty, so every attached
/// device counted as newly-connected while the proxy was still stopped — the
/// watchdog stripped the proxy off every paired phone roughly five seconds
/// after each launch. And once a serial was in `last_seen` it was never
/// examined again, so the one fire-and-forget reapply from `proxy.start` was
/// the *only* chance to restore the tunnel. If it failed, nothing retried and
/// the user saw a green UI with no traffic until they replugged the cable.
///
/// We only touch devices that are **paired** (have a `device` row), so
/// plugging in an unrelated phone never has its settings changed.
async fn device_watchdog(app: tauri::AppHandle) {
    use std::collections::{HashMap, HashSet};
    use std::time::Duration;
    use tauri::Manager;

    let mut strikes: HashMap<String, u8> = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    // First tick fires immediately; skip it so we don't race app boot.
    interval.tick().await;

    loop {
        interval.tick().await;
        let state: tauri::State<AppState> = app.state();

        // Snapshot what's plugged in right now. An error here means adb
        // couldn't answer, NOT that the devices went away — skip the tick
        // without touching any accumulated state.
        let attached: HashSet<String> = match state.devices.discover_attached().await {
            Ok(list) => list
                .into_iter()
                .filter(|d| d.platform == "android")
                .map(|d| d.serial)
                .collect(),
            Err(e) => {
                tracing::debug!(error = %e, "watchdog: device enumeration failed; skipping tick");
                continue;
            }
        };

        // Forget strikes for anything no longer plugged in, so a device that
        // comes back doesn't inherit a stale count and get repaired instantly.
        strikes.retain(|serial, _| attached.contains(serial));

        let paired: Vec<String> = state
            .devices
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.platform == "android" && d.connection == "usb")
            .map(|d| d.serial)
            .filter(|s| attached.contains(s))
            .collect();
        if paired.is_empty() {
            continue;
        }

        let proxy_running = state.proxy_handle.lock().is_some();
        let ca = state.ca.material();

        for serial in paired {
            let probe = match state.devices.probe_android_proxy(&serial).await {
                Ok(p) => p,
                Err(e) => {
                    // "Couldn't tell" — device is mid-setup or adb blipped.
                    // Explicitly not a strike: repairing on top of a running
                    // setup is how devices got half-configured before.
                    tracing::debug!(error = %e, serial, "watchdog: probe skipped");
                    continue;
                }
            };

            if proxy_running {
                if probe.is_healthy() {
                    strikes.remove(&serial);
                    continue;
                }
                let n = strikes.entry(serial.clone()).or_insert(0);
                *n += 1;
                if *n < UNHEALTHY_STRIKES {
                    tracing::debug!(
                        serial,
                        strikes = *n,
                        proxy_set = probe.proxy_set,
                        reverse_up = probe.reverse_up,
                        "watchdog: device unhealthy, waiting for confirmation"
                    );
                    continue;
                }
                strikes.remove(&serial);
                match state
                    .devices
                    .reapply_one_android_proxy(&serial, ca.clone())
                    .await
                {
                    Ok(()) => tracing::info!(
                        serial,
                        proxy_set = probe.proxy_set,
                        reverse_up = probe.reverse_up,
                        "watchdog: repaired unhealthy device"
                    ),
                    Err(e) => tracing::warn!(
                        error = %e, serial,
                        "watchdog: repair failed; will retry next tick"
                    ),
                }
            } else if probe.proxy_set {
                // Proxy is stopped but the phone still routes through us —
                // that's the "no internet on the device" footgun. Only act
                // when the setting is actually there, so a clean device is
                // left alone instead of being needlessly torn down on every
                // launch.
                match state.devices.clear_one_android_proxy(&serial).await {
                    Ok(()) => {
                        tracing::info!(serial, "watchdog: cleared stale proxy (proxy not running)")
                    }
                    Err(e) => tracing::warn!(error = %e, serial, "watchdog: clear failed"),
                }
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
