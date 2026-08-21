//! DeviceManager: cross-platform device discovery and state machine.
//!
//! Delegates the iOS- and Android-specific work to `pane-ios` and
//! `pane-android` sibling crates. Persists every state transition in
//! SQLite so the UI list survives restarts.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use pane_android::{AndroidPlatform, DeviceProxyState, Presence};
use pane_ca::CaMaterial;
use pane_engine::{DevicePortRegistry, PortAssignment};
use pane_ios::IosPlatform;
use pane_ipc::{AndroidToolingStatusDto, DeviceDto, DiscoveredDeviceDto, RemoveDeviceResult};
use pane_storage::Storage;
use rusqlite::params;
use time::OffsetDateTime;
use uuid::Uuid;

pub struct DeviceManager {
    storage: Arc<Storage>,
    ios: IosPlatform,
    android: AndroidPlatform,
    /// Shared with the proxy engine: maps serial→Mac-port→device_id so each
    /// device's captures can be attributed. DeviceManager owns the resolution
    /// (it has storage to look up the persisted device-row id) and assigns the
    /// port at pair time; the engine resolves device_id from the local port.
    registry: DevicePortRegistry,
}

impl DeviceManager {
    pub fn new(storage: Arc<Storage>, registry: DevicePortRegistry) -> Self {
        Self {
            storage,
            ios: IosPlatform::new(),
            android: AndroidPlatform::new(),
            registry,
        }
    }

    /// Resolve the PERSISTED device-row id for an Android serial. This is the
    /// id `devices_list` returns and the UI joins against — NOT the fresh UUID
    /// `add_usb` mints in its DeviceDto. `transition(... pairing ...)` has
    /// already inserted/updated the row by the time we call this, so the row
    /// exists. Returns None only if the row is somehow missing (then the
    /// capture stays unattributed rather than mis-attributed).
    fn android_device_id(&self, serial: &str) -> Option<String> {
        let conn = self.storage.conn().lock();
        conn.query_row(
            "SELECT id FROM device WHERE platform='android' AND serial=?1",
            params![serial],
            |r| r.get::<_, String>(0),
        )
        .ok()
    }

    /// Assign this serial its Mac-side proxy port, resolving + storing the
    /// device_id for attribution. Idempotent per serial (returns the existing
    /// port on re-pair / reapply).
    fn assign_port(&self, serial: &str) -> PortAssignment {
        let device_id = self.android_device_id(serial);
        let assignment = self.registry.assign(serial, device_id);
        if !assignment.attributed {
            tracing::warn!(
                serial,
                port = assignment.port,
                "proxy port pool exhausted; this device shares a port and its \
                 captures will show no device"
            );
        }
        assignment
    }

    /// Devices physically attached right now.
    ///
    /// Android failures **propagate**. They used to be swallowed by
    /// `unwrap_or_default()`, which turned "adb hiccuped" into "no devices are
    /// attached" — indistinguishable to every caller. The watchdog then
    /// committed that empty set to its `last_seen`, decided on the next tick
    /// that the device had just been plugged in, and kicked off a spurious
    /// full re-pair (or stripped the proxy off a perfectly healthy phone).
    /// The UI, for its part, showed an empty attached-list instead of an error.
    ///
    /// iOS stays best-effort: libimobiledevice being absent is the common case
    /// on an Android-only setup and must not fail the whole enumeration.
    pub async fn discover_attached(&self) -> Result<Vec<DiscoveredDeviceDto>> {
        let mut out = Vec::new();
        out.extend(self.ios.discover().await.unwrap_or_default());
        out.extend(self.android.discover().await?);
        Ok(out)
    }

    /// Read back whether a paired Android device is still actually wired up to
    /// us. `Err` means "couldn't tell" (adb unreachable, or the serial is
    /// mid-setup and its lock is held) — callers must treat that as "skip",
    /// never as "unhealthy", or a busy device would be reapplied on top of the
    /// setup that's already running.
    pub async fn probe_android_proxy(&self, serial: &str) -> Result<DeviceProxyState> {
        // Idempotent per serial, so this yields the same port the device is
        // (or is about to be) wired to. On a fresh process the registry is
        // empty and this assigns one — the probe then correctly reports the
        // reverse as down, and the watchdog repairs it.
        let mac_port = self.assign_port(serial).port;
        self.android.probe_proxy_state(serial, mac_port).await
    }

    /// May this serial still be reversing onto its assigned pool port?
    ///
    /// `adb reverse` mappings are bound to the device transport: the adb server
    /// drops them when the device disconnects, and `adb -s <gone> reverse
    /// --list` errors with "device not found". So an unplugged phone cannot
    /// send anything to its old port, and its reservation is pure leak — which
    /// matters because removing a device *with the cable out* is the common
    /// case, and each such removal would otherwise book a pool port for the
    /// rest of the process.
    ///
    /// An enumeration failure means "adb couldn't answer", not "nothing is
    /// attached" — the same rule the watchdog and `discover_attached` follow —
    /// so it resolves to "assume it's there" and keeps the reservation.
    async fn android_may_hold_reverse(&self, serial: &str) -> bool {
        match self.discover_attached().await {
            Ok(list) => list.iter().any(|d| d.serial == serial),
            Err(e) => {
                tracing::debug!(error = %e, serial, "can't tell if device is attached; keeping its port reserved");
                true
            }
        }
    }

    /// Cheap check for the proxy-stopped case: is this device still stranded
    /// pointing at us? Same `Err` == "skip, don't act" contract as
    /// `probe_android_proxy`.
    pub async fn android_still_points_at_us(&self, serial: &str) -> Result<bool> {
        self.android.is_proxy_pointed_at_us(serial).await
    }

    pub fn android_tooling_status(&self) -> AndroidToolingStatusDto {
        self.android.tooling_status()
    }

    /// Late-binds the path to the bundled pane-helper APK. Tauri's
    /// `resource_dir()` isn't resolvable from `bootstrap()`, so the
    /// desktop crate calls this from its setup handler instead.
    pub fn set_android_helper_apk(&self, path: std::path::PathBuf) {
        self.android.set_helper_apk(path);
    }

    pub async fn add_ios_usb(&self, serial: &str, ca: CaMaterial) -> Result<DeviceDto> {
        self.transition("ios", serial, "pairing", None)?;
        let outcome = self.ios.add_usb(serial, &ca).await;
        match outcome {
            Ok(device) => {
                self.record_ready(&device)?;
                Ok(device)
            }
            Err(e) => {
                self.transition("ios", serial, "error", Some(&e.to_string()))?;
                Err(e)
            }
        }
    }

    pub async fn add_android_usb(&self, serial: &str, ca: CaMaterial) -> Result<DeviceDto> {
        self.transition("android", serial, "pairing", None)?;
        // Resolve device_id + reserve the Mac-side port AFTER the pairing row
        // exists (so android_device_id finds it) but BEFORE add_usb sets up the
        // `adb reverse`, which needs the assigned port.
        let assignment = self.assign_port(serial);
        // Interactive: the user clicked Add or Re-sync against the on-screen
        // attached list, which may be a few seconds stale after a replug.
        let outcome = self
            .android
            .add_usb(serial, &ca, assignment.port, Presence::Wait)
            .await;
        match outcome {
            Ok(device) => {
                self.record_ready(&device)?;
                Ok(device)
            }
            Err(e) => {
                // Give the port back if this call is what took it. A pairing
                // that failed set up no `adb reverse`, so nothing on the device
                // can send traffic to that port — holding it just shrinks the
                // pool. A phone that has been unplugged for weeks was still
                // reserving a port on every proxy start this way, and eight of
                // those quietly turn every device unattributed.
                //
                // `fresh` matters: on a failed Re-sync of a device that paired
                // successfully earlier, the phone may still have a live reverse
                // onto this port, and handing it to another device would stamp
                // that device's id on this phone's traffic.
                if assignment.fresh {
                    self.registry.release(serial);
                }
                self.transition("android", serial, "error", Some(&e.to_string()))?;
                Err(e)
            }
        }
    }

    pub async fn remove(&self, id: Uuid) -> Result<RemoveDeviceResult> {
        let dev = self.get(id)?;
        let cleaned = match dev.platform.as_str() {
            "ios" => self.ios.remove(&dev.serial).await.is_ok(),
            "android" => {
                // Release the port only once nothing can still be reversing
                // onto it. Releasing unconditionally — which is what this did —
                // hands the port back to the pool while a reachable phone may
                // still be pointing at it; the next device to pair inherits the
                // port and the first phone's traffic arrives stamped with the
                // new device's id.
                //
                // Teardown succeeding is the clean case. Teardown failing on a
                // phone that isn't attached any more is equally safe: its
                // reverse died with the USB connection. Only a device that is
                // still attached *and* wouldn't be cleaned keeps its port.
                let ok = self.android.remove(&dev.serial).await.is_ok();
                if ok || !self.android_may_hold_reverse(&dev.serial).await {
                    self.registry.release(&dev.serial);
                } else {
                    tracing::warn!(
                        serial = %dev.serial,
                        "couldn't tear down proxy on a still-attached device; \
                         keeping its port reserved so no other device inherits it"
                    );
                }
                ok
            }
            _ => false,
        };
        let conn = self.storage.conn().lock();
        conn.execute(
            "UPDATE device SET state='removed' WHERE id=?1",
            params![id.to_string()],
        )?;
        // Drop this device from every rule's scope. The row itself is kept
        // (re-pairing restores the device), but a stale scope entry would sit
        // in the rule list as a device nobody can see, and would come back to
        // life pointing at whatever re-used that id.
        conn.execute(
            "DELETE FROM rule_device WHERE device_id=?1",
            params![id.to_string()],
        )?;
        Ok(RemoveDeviceResult {
            cleaned,
            pending_cleanup: !cleaned,
        })
    }

    pub fn get(&self, id: Uuid) -> Result<DeviceDto> {
        let conn = self.storage.conn().lock();
        let mut stmt = conn.prepare(
            "SELECT id, platform, connection, serial, display_name, state, ca_installed_at,
                    capabilities_json, last_error
             FROM device WHERE id=?1",
        )?;
        let row = stmt
            .query_row(params![id.to_string()], Self::map_row)
            .map_err(|_| anyhow!("device not found"))?;
        Ok(row)
    }

    /// Best-effort: re-apply PAC + adb reverse on every paired Android
    /// device. Called from proxy.start so paired phones reconnect to
    /// the freshly-started proxy without the user having to click
    /// Re-sync on each row by hand. Returns the serials we successfully
    /// re-applied. Errors per device are swallowed — adb may not be
    /// connected, the user may have unplugged, etc.
    /// Single-device version of reapply_all_android_proxies. Used by
    /// the watchdog on reconnect events.
    pub async fn reapply_one_android_proxy(
        &self,
        serial: &str,
        ca: CaMaterial,
    ) -> anyhow::Result<()> {
        let assignment = self.assign_port(serial);
        // Background sweep over every paired device, most of which may not be
        // plugged in at all — waiting on each would delay the one that is.
        match self
            .android
            .add_usb(serial, &ca, assignment.port, Presence::Assume)
            .await
        {
            Ok(device) => {
                // Persist the outcome, so a device that had previously failed
                // clears its error banner once it comes good again.
                self.record_ready(&device)?;
                Ok(())
            }
            Err(e) => {
                // Same rollback as the interactive path: a reapply that never
                // established a reverse shouldn't keep a pool port booked.
                if assignment.fresh {
                    self.registry.release(serial);
                }
                // Surface it on the device row. Without this the reapply paths
                // fail entirely in the logs and the UI keeps showing "ready".
                let _ = self.transition("android", serial, "error", Some(&e.to_string()));
                Err(e)
            }
        }
    }

    /// Single-device cleanup. Used by the watchdog when a paired phone
    /// reconnects while Pane proxy is stopped — restores the phone's
    /// internet by stripping the stale http_proxy setting.
    pub async fn clear_one_android_proxy(&self, serial: &str) -> anyhow::Result<()> {
        // Release after the teardown, under the same rule as `remove`: safe
        // once either the cleanup worked or the device is gone from the bus.
        let outcome = self.android.remove(serial).await;
        if outcome.is_ok() || !self.android_may_hold_reverse(serial).await {
            self.registry.release(serial);
        }
        outcome
    }

    pub async fn reapply_all_android_proxies(&self, ca: CaMaterial) -> Vec<String> {
        // Only touch phones that are actually plugged in. Every paired row used
        // to get the full treatment on each proxy start, so a device last seen
        // weeks ago still reserved a pool port and produced a burst of "device
        // 'X' not found" errors in the log before failing — with eight pool
        // ports, a handful of those stale rows is enough to leave the phone on
        // the desk unattributed.
        //
        // An enumeration failure means "adb couldn't answer", not "nothing is
        // attached", so fall back to trying everything rather than silently
        // configuring nothing.
        let attached: Option<std::collections::HashSet<String>> = match self
            .discover_attached()
            .await
        {
            Ok(list) => Some(list.into_iter().map(|d| d.serial).collect()),
            Err(e) => {
                tracing::debug!(error = %e, "reapply: device enumeration failed; trying all paired");
                None
            }
        };
        let serials: Vec<String> = self
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.platform == "android" && d.connection == "usb")
            .map(|d| d.serial)
            .filter(|s| attached.as_ref().is_none_or(|a| a.contains(s)))
            .collect();
        let mut ok = Vec::with_capacity(serials.len());
        for serial in serials {
            // Errors are per-device and recorded on the device row by
            // reapply_one; a phone that's simply unplugged shouldn't stop us
            // from configuring the ones that are present.
            match self.reapply_one_android_proxy(&serial, ca.clone()).await {
                Ok(()) => ok.push(serial),
                Err(e) => {
                    tracing::warn!(error = %e, serial, "reapply failed for device")
                }
            }
        }
        ok
    }

    /// Best-effort: clear the system http_proxy + adb-reverse on every
    /// paired Android device. Called from proxy.stop so users don't end
    /// up with a phone pointing at a now-dead 127.0.0.1:8888 (which
    /// means: no internet at all on the device until they remove or
    /// re-pair). Errors per device are swallowed — adb may not be
    /// connected; that's fine, the proxy setting will get cleaned up
    /// the next time `remove()` runs for that device.
    pub async fn clear_all_android_proxies(&self) -> Vec<String> {
        let serials: Vec<String> = self
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.platform == "android" && d.connection == "usb")
            .map(|d| d.serial)
            .collect();
        // One enumeration for the whole sweep — this runs on every proxy.stop,
        // and the unplugged device is the common case here.
        let attached: Option<std::collections::HashSet<String>> =
            match self.discover_attached().await {
                Ok(list) => Some(list.into_iter().map(|d| d.serial).collect()),
                Err(_) => None,
            };
        let mut cleaned = Vec::with_capacity(serials.len());
        for serial in serials {
            // Same rule as `remove`: the port goes back once nothing can still
            // be reversing onto it — either the teardown worked, or the device
            // is off the bus and its reverse died with the connection. When adb
            // couldn't answer at all we keep the reservation.
            let ok = self.android.remove(&serial).await.is_ok();
            let gone = attached.as_ref().is_some_and(|a| !a.contains(&serial));
            if ok || gone {
                self.registry.release(&serial);
            }
            if ok {
                cleaned.push(serial);
            }
        }
        cleaned
    }

    pub fn list(&self) -> Result<Vec<DeviceDto>> {
        let conn = self.storage.conn().lock();
        let mut stmt = conn.prepare(
            "SELECT id, platform, connection, serial, display_name, state, ca_installed_at,
                    capabilities_json, last_error
             FROM device WHERE state <> 'removed' ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], Self::map_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceDto> {
        let id: String = r.get(0)?;
        let caps: Option<String> = r.get(7)?;
        Ok(DeviceDto {
            id: Uuid::parse_str(&id).unwrap(),
            platform: r.get(1)?,
            connection: r.get(2)?,
            serial: r.get(3)?,
            display_name: r.get(4)?,
            state: r.get(5)?,
            ca_installed_at: r
                .get::<_, Option<i64>>(6)?
                .map(|t| OffsetDateTime::from_unix_timestamp(t).unwrap().to_string()),
            capabilities: caps
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::json!({})),
            last_error: r.get(8)?,
        })
    }

    /// Persist a successfully-paired device with all DTO metadata, so the UI
    /// sees `display_name`, `last_error` (e.g. no-root warning), and the
    /// `capabilities` blob. `transition` only writes the bare state — useful
    /// for `pairing`/`error` intermediate steps; this is the success-final
    /// write that supersedes it.
    fn record_ready(&self, d: &DeviceDto) -> Result<()> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let caps_json = d.capabilities.to_string();
        let ca_installed_at_unix = d
            .ca_installed_at
            .as_deref()
            .and_then(|s| {
                OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
            })
            .map(|t| t.unix_timestamp())
            .unwrap_or(now);
        // The pairing-row was already created by `transition(... pairing ...)`
        // at the start of `add_*_usb`, so plain UPDATE by (platform, serial)
        // is enough — sidesteps id-mismatch between the DTO's freshly-minted
        // UUID and the existing row's id.
        let conn = self.storage.conn().lock();
        conn.execute(
            "UPDATE device
                SET display_name=?1,
                    state='ready',
                    ca_installed_at=?2,
                    capabilities_json=?3,
                    last_error=?4
              WHERE platform=?5 AND serial=?6",
            params![
                &d.display_name,
                ca_installed_at_unix,
                caps_json,
                d.last_error.as_deref(),
                &d.platform,
                &d.serial,
            ],
        )?;
        Ok(())
    }

    fn transition(
        &self,
        platform: &str,
        serial: &str,
        new_state: &str,
        last_error: Option<&str>,
    ) -> Result<()> {
        let id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let conn = self.storage.conn().lock();
        conn.execute(
            "INSERT INTO device (id, platform, connection, serial, display_name, state, last_error, created_at)
             VALUES (?1, ?2, 'usb', ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(platform, serial) DO UPDATE SET state=excluded.state, last_error=excluded.last_error",
            params![
                id.to_string(),
                platform,
                serial,
                serial,
                new_state,
                last_error,
                now
            ],
        )?;
        Ok(())
    }
}
