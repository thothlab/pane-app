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
//!   - Each parsed batch is persisted to SQLite (`logcat_entry`), then a
//!     lightweight `logcat://appended` ping (payload = batch count) is emitted
//!     on that webview window. The window reads rows back via `logcat_query`;
//!     the firehose never crosses IPC.

use std::collections::HashMap;

use pane_android::logcat::{spawn as spawn_logcat, LogcatConfig, LogcatEvent};
use pane_android::AndroidPlatform;
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::mpsc;

use super::{to_api, CmdResult};
use crate::state::AppState;
use pane_ipc::{
    ClearResult, LogcatClearArgs, LogcatExportArgs, LogcatNewCountArgs, LogcatQueryArgs,
    LogcatQueryOlderArgs, LogcatRowDto,
};
use pane_storage::LogcatInsert;
use time::OffsetDateTime;

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

    // Spawn the adb logcat stream. The callback persists each batch to SQLite
    // (durable, per-device) and forwards it to the webview only (scoped emit),
    // so the main window never sees the firehose.
    let win_for_emit = window.clone();
    let storage_for_db = app.state::<AppState>().storage.clone();
    let serial_db = serial.clone();
    let cfg = LogcatConfig {
        serial: serial.clone(),
        ..Default::default()
    };
    let shutdown_tx = spawn_logcat(cfg, move |ev| match ev {
        LogcatEvent::Batch(entries) => {
            // Persist first — the DB is the source of truth (Phase 3 makes the
            // UI read from it). One created_at stamp shared across the batch.
            let created_at_ms =
                (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
            let rows: Vec<LogcatInsert> = entries
                .iter()
                .map(|e| LogcatInsert {
                    device_ts: e.timestamp.clone(),
                    pid: e.pid,
                    tid: e.tid,
                    level: e.level as i64,
                    tag: e.tag.clone(),
                    message: e.message.clone(),
                })
                .collect();
            // INSERT OR IGNORE returns how many rows were actually new. When
            // adb re-dumps the device ring buffer on window reopen / reconnect,
            // every replayed line collides with the dedup index and `inserted`
            // is 0 — so we skip the ping entirely and the window doesn't churn
            // through no-op re-queries of history it already shows from the DB.
            let inserted = match storage_for_db.insert_logcat_batch(
                &serial_db,
                created_at_ms,
                &rows,
            ) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(error = %e, "logcat: db insert failed");
                    0
                }
            };
            // The DB is the source of truth; the webview reads it via
            // logcat_query. Emit only a lightweight "rows appended" ping (count
            // of new rows) so the window knows to re-query / bump its badge —
            // no firehose over IPC. Webview-scoped (Tauri 2 instance emit).
            if inserted > 0 {
                if let Err(e) = win_for_emit.emit("logcat://appended", inserted) {
                    tracing::warn!(error = %e, "logcat: emit failed (window gone?)");
                }
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

/// Query persisted logcat rows for a device. `filter` is the raw DSL string;
/// `include_pids`/`exclude_pids` are the frontend-resolved `app:` PIDs.
#[tauri::command]
pub async fn logcat_query(
    state: State<'_, AppState>,
    args: LogcatQueryArgs,
) -> CmdResult<Vec<LogcatRowDto>> {
    state
        .storage
        .query_logcat(
            &args.serial,
            args.filter.as_deref(),
            &args.include_pids,
            &args.exclude_pids,
            args.limit,
        )
        .map_err(to_api("db"))
}

/// Query the newest `limit` rows older than `before_id` — "load older on
/// scroll-up". Same filter/PID contract as `logcat_query`.
#[tauri::command]
pub async fn logcat_query_older(
    state: State<'_, AppState>,
    args: LogcatQueryOlderArgs,
) -> CmdResult<Vec<LogcatRowDto>> {
    state
        .storage
        .query_logcat_before(
            &args.serial,
            args.filter.as_deref(),
            &args.include_pids,
            &args.exclude_pids,
            args.before_id,
            args.limit,
        )
        .map_err(to_api("db"))
}

/// Count matching rows newer than `after_id` — the "+N new" badge while frozen.
#[tauri::command]
pub async fn logcat_new_count(
    state: State<'_, AppState>,
    args: LogcatNewCountArgs,
) -> CmdResult<i64> {
    state
        .storage
        .count_logcat_new(
            &args.serial,
            args.filter.as_deref(),
            &args.include_pids,
            &args.exclude_pids,
            args.after_id,
        )
        .map_err(to_api("db"))
}

/// Delete all persisted rows for one device (the Clear button).
#[tauri::command]
pub async fn logcat_clear(
    state: State<'_, AppState>,
    args: LogcatClearArgs,
) -> CmdResult<ClearResult> {
    let n = state
        .storage
        .clear_logcat(&args.serial)
        .map_err(to_api("db"))?;
    Ok(ClearResult { deleted: n as u64 })
}

/// Export the full (uncapped) filtered set for a device to a file in
/// threadtime format. Returns the number of lines written.
#[tauri::command]
pub async fn logcat_export(
    state: State<'_, AppState>,
    args: LogcatExportArgs,
) -> CmdResult<usize> {
    state
        .storage
        .export_logcat(
            &args.serial,
            args.filter.as_deref(),
            &args.include_pids,
            &args.exclude_pids,
            &args.path,
        )
        .map_err(to_api("io"))
}

/// Full PID → process-name snapshot. Polled by the Logcat window
/// every 10s to label rows with the package the entry came from
/// — the table's new App column reads from this map keyed on
/// entry.pid. Returns a JSON object with PID strings as keys.
#[tauri::command]
pub async fn android_pid_names(
    serial: String,
) -> CmdResult<std::collections::HashMap<u32, String>> {
    let android = AndroidPlatform::new();
    android.pid_names(&serial).await.map_err(to_api("adb"))
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
