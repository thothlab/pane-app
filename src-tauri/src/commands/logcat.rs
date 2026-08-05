//! Logcat window — opens a non-modal `WebviewWindow` per Android device.
//!
//! This is the one command that genuinely cannot move into `pane-core`:
//! creating a webview is Tauri's job. The `adb logcat` stream it used to own
//! now lives in `Core::logcat_start`, keyed by serial, so the CLI can attach
//! to the same session.
//!
//! One window per `serial` (label = `logcat-{serial}`). A second click on the
//! same device refocuses the existing window — never double-spawns the
//! subprocess.
//!
//! Lifecycle:
//!   - `logcat_open` builds the window, then asks the core to start (or
//!     reuse) the stream.
//!   - `WindowEvent::Destroyed` tells the core to stop it, which fires the
//!     shutdown channel; the task then kills the `adb` child.
//!
//! Frontend contract (unchanged):
//!   - Window URL `index.html?logcat=1&serial=…`; `src/main.tsx` reads
//!     `location.search` and mounts `LogcatView` instead of the main app.
//!   - Each persisted batch produces a lightweight `logcat://appended` ping
//!     on that window (payload = new-row count); the window re-queries via
//!     `logcat_query`. The firehose never crosses IPC. `lib.rs` translates
//!     the core's `logcat.appended` bus events into those per-window emits.

use super::{CmdResult, CoreState};
use pane_ipc::{
    kinds, ClearResult, LogcatClearArgs, LogcatExportArgs, LogcatNewCountArgs, LogcatQueryArgs,
    LogcatQueryOlderArgs, LogcatRowDto,
};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// Window label for a device's logcat window. `lib.rs` uses the same mapping
/// to route `logcat.appended` events back to the right window.
pub fn label_for(serial: &str) -> String {
    // serial is opaque (`adb devices` output); real serials are `[A-Z0-9.:]`.
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

    // The frontend reads ?logcat=1&serial=... in main.tsx. Keeping it a query
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
        .map_err(pane_core::to_api(kinds::WINDOW_BUILD))?;

    let core = app.state::<std::sync::Arc<pane_core::Core>>();
    let outcome = core.logcat_start(&serial)?;

    // Stop the subprocess when the user closes the window.
    // `WindowEvent::Destroyed` is the right signal — it fires after the
    // window is gone, regardless of close path (Cmd-Q on the single window,
    // parent app exit, OS forced quit).
    let app_for_cleanup = app.clone();
    let serial_for_cleanup = serial.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            let core = app_for_cleanup.state::<std::sync::Arc<pane_core::Core>>();
            core.logcat_stop(&serial_for_cleanup);
        }
    });

    Ok(serde_json::json!({ "label": label, "reused": outcome.reused }))
}

/// Write `content` as-is to `path`, for the Logcat window's Export button.
///
/// Why a backend command instead of `@tauri-apps/plugin-fs`: plugin-fs
/// requires `fs:allow-write-text-file` plus scope rules per capability, which
/// gets ugly fast for "write anywhere the user picked".
#[tauri::command]
pub async fn logcat_write_export(path: String, content: String) -> CmdResult<usize> {
    pane_core::write_text_file(&path, &content)
}

#[tauri::command]
pub async fn logcat_query(
    state: CoreState<'_>,
    args: LogcatQueryArgs,
) -> CmdResult<Vec<LogcatRowDto>> {
    state.logcat_query(args).await
}

/// "Load older on scroll-up". Same filter/PID contract as `logcat_query`.
#[tauri::command]
pub async fn logcat_query_older(
    state: CoreState<'_>,
    args: LogcatQueryOlderArgs,
) -> CmdResult<Vec<LogcatRowDto>> {
    state.logcat_query_older(args).await
}

/// Count matching rows newer than `after_id` — the "+N new" badge while frozen.
#[tauri::command]
pub async fn logcat_new_count(state: CoreState<'_>, args: LogcatNewCountArgs) -> CmdResult<i64> {
    state.logcat_new_count(args).await
}

#[tauri::command]
pub async fn logcat_clear(state: CoreState<'_>, args: LogcatClearArgs) -> CmdResult<ClearResult> {
    state.logcat_clear(&args.serial).await
}

/// Export the full (uncapped) filtered set to a file in threadtime format.
#[tauri::command]
pub async fn logcat_export(state: CoreState<'_>, args: LogcatExportArgs) -> CmdResult<usize> {
    state.logcat_export(args).await
}

/// Full PID → process-name snapshot, polled every 10s to label rows with the
/// package the entry came from.
#[tauri::command]
pub async fn android_pid_names(
    state: CoreState<'_>,
    serial: String,
) -> CmdResult<std::collections::HashMap<u32, String>> {
    state.android_pid_names(&serial).await
}

/// URL-encode for a query-parameter value. Real adb serials (`[A-Z0-9.:]`)
/// don't need escaping, but being defensive about future weirdness is cheap.
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
