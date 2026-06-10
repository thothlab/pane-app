//! Logcat window — opens a non-modal `WebviewWindow` per Android device
//! and pipes `adb logcat` into it.
//!
//! One window per `serial` (label = `logcat-{serial}`). A second click
//! on the same device just refocuses the existing window — never
//! double-spawns the subprocess.
//!
//! Lifecycle:
//!   - `logcat_open` builds the WebviewWindow and spawns
//!     `pane_android::logcat::spawn`, which owns the `adb` child with
//!     `kill_on_drop(true)`.
//!   - `WindowEvent::Destroyed` on the webview fires the shutdown
//!     channel; the task then `child.kill().await`s and exits.
//!   - The shutdown sender is parked in `AppState::logcat_shutdowns`
//!     (Mutex<HashMap<label, Sender>>) so a "Logcat" double-click
//!     can find and re-use the existing session.
//!
//! Frontend contract:
//!   - Window URL: `index.html?logcat=1&serial=...&app_label=...`.
//!     (Query string, not path — easier with vite's index.html mount;
//!     a tiny dispatcher in `src/main.tsx` reads `location.search` and
//!     mounts `LogcatView` instead of the main `App`.)
//!   - Per-batch event: `logcat://batch`, payload `Vec<LogEntry>`,
//!     emitted only on that webview window (no firehose to main).

use std::collections::HashMap;

use pane_android::logcat::{spawn as spawn_logcat, LogcatConfig, LogcatEvent};
use pane_android::AndroidPlatform;
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::mpsc;

use super::{to_api, CmdResult};

/// Tracks an open logcat session so a re-open call can detect and
/// focus instead of re-spawning. Lives on `AppState` (initialised in
/// `state.rs`).
pub struct LogcatSessions(pub Mutex<HashMap<String, mpsc::Sender<()>>>);

impl LogcatSessions {
    pub fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

fn label_for(serial: &str) -> String {
    // serial is opaque (`adb devices` output); for an actual filesystem-
    // unsafe character we'd sanitize, but real serials are `[A-Z0-9.:]`.
    format!("logcat-{serial}")
}

#[tauri::command]
pub async fn logcat_open(
    app: AppHandle,
    serial: String,
    app_label: Option<String>,
) -> CmdResult<serde_json::Value> {
    let label = label_for(&serial);

    // Existing window? Focus and return — don't double-spawn the subprocess.
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        return Ok(serde_json::json!({ "label": label, "reused": true }));
    }

    // The frontend reads ?logcat=1&serial=... in main.tsx and mounts
    // LogcatView instead of the normal App. Keeping it as a query
    // string (not a path) sidesteps a separate vite build entry.
    let serial_q = url_encode(&serial);
    let url = WebviewUrl::App(
        format!("index.html?logcat=1&serial={serial_q}")
            .as_str()
            .into(),
    );
    let title = match &app_label {
        Some(name) => format!("Logcat — {name}"),
        None => format!("Logcat — {serial}"),
    };
    let window = WebviewWindowBuilder::new(&app, label.clone(), url)
        .title(title)
        .inner_size(1100.0, 720.0)
        .min_inner_size(700.0, 400.0)
        .resizable(true)
        .visible(true)
        .build()
        .map_err(to_api("window_build"))?;

    // Spawn the adb logcat stream. The callback forwards parsed batches
    // to the webview only (scoped emit), so the main window never sees
    // the firehose.
    let win_for_emit = window.clone();
    let cfg = LogcatConfig {
        serial: serial.clone(),
        ..Default::default()
    };
    let shutdown_tx = spawn_logcat(cfg, move |ev| match ev {
        LogcatEvent::Batch(entries) => {
            // Tauri 2 instance-method emit — webview-scoped.
            if let Err(e) = win_for_emit.emit("logcat://batch", &entries) {
                tracing::warn!(error = %e, "logcat: emit failed (window gone?)");
            }
        }
        LogcatEvent::Error(msg) => {
            let _ = win_for_emit.emit(
                "logcat://error",
                serde_json::json!({ "message": msg }),
            );
        }
    })
    .map_err(to_api("logcat_spawn"))?;

    // Park the shutdown sender so we can fire it on window close.
    let sessions = app.state::<LogcatSessions>();
    sessions.0.lock().insert(label.clone(), shutdown_tx.clone());

    // Stop the subprocess + drop the session entry when the user closes
    // the window. WindowEvent::Destroyed is the right signal — fires
    // after the window is gone, regardless of close path (Cmd-Q on the
    // single window, parent app exit, OS forced quit).
    let app_handle_for_cleanup = app.clone();
    let label_for_cleanup = label.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            // Take the sender out of the map; drop the guard before the
            // try_send so the temporary MutexGuard's lifetime ends
            // before the borrow of `sessions` does (NLL nuance —
            // otherwise the State<'_>::Drop fires while we still hold
            // the inner reference).
            let tx_opt = {
                let sessions = app_handle_for_cleanup.state::<LogcatSessions>();
                let removed = sessions.0.lock().remove(&label_for_cleanup);
                removed
            };
            if let Some(tx) = tx_opt {
                // try_send so we don't block the event handler; the
                // task wakes up on next select tick anyway.
                let _ = tx.try_send(());
            }
        }
    });

    Ok(serde_json::json!({ "label": label, "reused": false }))
}

/// Write `content` as-is to `path`. Used by the Logcat window's
/// Export button — the frontend already serialised the visible
/// entries to threadtime-formatted text, we just need to drop it
/// to disk. Path comes from a save-dialog the user just confirmed,
/// so it's trusted; we don't gate on capability scope (the same
/// way `api.ca.save_to_file` works).
///
/// Why a backend command instead of `@tauri-apps/plugin-fs`:
/// plugin-fs requires `fs:allow-write-text-file` + scope rules
/// per capability, which gets ugly fast for "write anywhere the
/// user picked." A thin Rust command sidesteps the whole thing.
#[tauri::command]
pub async fn logcat_write_export(path: String, content: String) -> CmdResult<usize> {
    let bytes = content.len();
    std::fs::write(&path, content).map_err(to_api("io"))?;
    Ok(bytes)
}

/// Resolve a package's current PID, or `None` if it isn't running.
/// The Logcat window polls this every 5s for each `app:<pkg>` token in
/// the active filter, so process restarts pick up the new PID
/// transparently.
#[tauri::command]
pub async fn android_pidof(
    serial: String,
    package: String,
) -> CmdResult<Option<u32>> {
    let android = AndroidPlatform::new();
    android
        .pidof(&serial, &package)
        .await
        .map_err(to_api("adb"))
}

/// URL-encode a string for use inside a query parameter value. We avoid
/// pulling `urlencoding` for one-call use — the character set we see in
/// real adb serials (`[A-Z0-9.:]`) doesn't actually need escaping, but
/// being defensive against future weirdness is cheap.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            for byte in c.to_string().as_bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}
