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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use crate::Core;

/// Prune logcat rows older than this.
const LOGCAT_RETENTION_MS: i64 = 24 * 60 * 60 * 1000;
/// Per-device row cap applied alongside the age cutoff.
const LOGCAT_PER_DEVICE_CAP: i64 = 2_000_000;

/// How many consecutive unhealthy probes before we repair a device.
///
/// One miss is not enough: a probe can legitimately catch a device mid-reboot
/// or between an `adb reverse` teardown and its rebuild, and repairing on that
/// would mean a full re-pair (including an APK install) on every hiccup. Two
/// misses is ~10 s of genuinely broken tunnel, which is well below what a user
/// notices as "it stopped working" and well above transient noise.
const UNHEALTHY_STRIKES: u8 = 2;

/// Reconcile paired Android devices with their actual connection state.
/// Runs every 5 seconds.
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
pub async fn device_watchdog(core: Arc<Core>) {
    let mut strikes: HashMap<String, u8> = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    // First tick fires immediately; skip it so we don't race boot.
    interval.tick().await;

    loop {
        interval.tick().await;

        // Snapshot what's plugged in right now. An error here means adb
        // couldn't answer, NOT that the devices went away — skip the tick
        // without touching any accumulated state.
        let attached: HashSet<String> = match core.devices.discover_attached().await {
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

        let paired: Vec<String> = core
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

        let proxy_running = core.proxy_running();
        let ca = core.ca.material();

        for serial in paired {
            if !proxy_running {
                // Proxy is stopped, so the tunnel is irrelevant by definition —
                // the only question is whether the phone is stranded pointing
                // at a dead port. One adb call instead of two, in what is a
                // very common idle state.
                match core.devices.android_still_points_at_us(&serial).await {
                    Ok(false) => {} // already clean, leave it alone
                    Ok(true) => match core.devices.clear_one_android_proxy(&serial).await {
                        Ok(()) => tracing::info!(
                            serial,
                            "watchdog: cleared stale proxy (proxy not running)"
                        ),
                        Err(e) => tracing::warn!(error = %e, serial, "watchdog: clear failed"),
                    },
                    Err(e) => {
                        tracing::debug!(error = %e, serial, "watchdog: probe skipped")
                    }
                }
                continue;
            }

            let probe = match core.devices.probe_android_proxy(&serial).await {
                Ok(p) => p,
                Err(e) => {
                    // "Couldn't tell" — device is mid-setup or adb blipped.
                    // Explicitly not a strike: repairing on top of a running
                    // setup is how devices got half-configured before.
                    tracing::debug!(error = %e, serial, "watchdog: probe skipped");
                    continue;
                }
            };

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
            match core
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
