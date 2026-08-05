//! Long-running maintenance loops.
//!
//! Moved out of `src-tauri/src/lib.rs::setup()` so a headless instance runs
//! the same reconciliation the GUI does. They take `Arc<Core>` instead of an
//! `AppHandle`.
//!
//! ⚠ Callers inside Tauri's `setup()` must spawn these with
//! `tauri::async_runtime::spawn`, **not** `tokio::spawn` — Tauri 2's setup
//! does not run inside a tokio reactor context, and `tokio::spawn` panics
//! there. That mistake shipped in 0.1.37/0.1.38 and aborted the app during
//! launch on macOS. A plain `tokio::spawn` is correct from a CLI `main`.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use crate::Core;

/// Prune logcat rows older than this.
const LOGCAT_RETENTION_MS: i64 = 24 * 60 * 60 * 1000;
/// Per-device row cap applied alongside the age cutoff.
const LOGCAT_PER_DEVICE_CAP: i64 = 2_000_000;

/// Reconcile paired Android devices with their actual connection state.
///
/// When the user yanks the USB cable without stopping the proxy, the phone's
/// `http_proxy` keeps pointing at 127.0.0.1:8888 — but adb reverse is gone,
/// so the device loses internet, and clearing a global proxy from the phone's
/// own settings is awkward (Samsung hides it). On reconnect:
///
///   - proxy running → re-apply `http_proxy` + reverse, so MITM resumes
///     without the user clicking Re-sync;
///   - proxy stopped → strip the proxy settings, restoring the device's
///     internet.
///
/// Only **paired** devices are touched, so plugging in an unrelated phone
/// leaves its settings alone.
pub async fn device_watchdog(core: Arc<Core>) {
    let mut last_seen: HashSet<String> = HashSet::new();
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    // First tick fires immediately; skip it so we don't race boot.
    interval.tick().await;

    loop {
        interval.tick().await;

        let attached: HashSet<String> = match core.devices.discover_attached().await {
            Ok(list) => list
                .into_iter()
                .filter(|d| d.platform == "android")
                .map(|d| d.serial)
                .collect(),
            // adb not on PATH or a daemon hiccup — skip this tick.
            Err(_) => continue,
        };
        if attached == last_seen {
            continue;
        }

        let newly_connected: Vec<String> = attached.difference(&last_seen).cloned().collect();
        last_seen = attached;
        if newly_connected.is_empty() {
            continue;
        }

        let paired_serials: HashSet<String> = core
            .devices
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.platform == "android" && d.connection == "usb")
            .map(|d| d.serial)
            .collect();

        let proxy_running = core.proxy_running();
        let ca = core.ca.material();

        for serial in newly_connected {
            if !paired_serials.contains(&serial) {
                continue; // not one of ours, leave alone
            }
            if proxy_running {
                let _ = core
                    .devices
                    .reapply_one_android_proxy(&serial, ca.clone())
                    .await;
                tracing::info!(serial, "watchdog: re-applied proxy on reconnect");
            } else {
                let _ = core.devices.clear_one_android_proxy(&serial).await;
                tracing::info!(serial, "watchdog: cleared stale proxy on reconnect");
            }
        }
    }
}

/// Prune persisted logcat rows on a 5-minute cadence.
pub async fn logcat_retention(core: Arc<Core>) {
    loop {
        if let Err(e) = core
            .storage
            .prune_logcat(LOGCAT_RETENTION_MS, LOGCAT_PER_DEVICE_CAP)
        {
            tracing::warn!(error = %e, "logcat: prune failed");
        }
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}
