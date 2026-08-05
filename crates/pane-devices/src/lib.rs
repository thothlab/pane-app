//! DeviceManager: cross-platform device discovery and state machine.
//!
//! Delegates the iOS- and Android-specific work to `pane-ios` and
//! `pane-android` sibling crates. Persists every state transition in
//! SQLite so the UI list survives restarts.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use pane_android::{AndroidPlatform, DeviceProxyState};
use pane_ca::CaMaterial;
use pane_engine::DevicePortRegistry;
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
    fn assign_port(&self, serial: &str) -> u16 {
        let device_id = self.android_device_id(serial);
        self.registry.assign(serial, device_id)
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
        let mac_port = self.assign_port(serial);
        self.android.probe_proxy_state(serial, mac_port).await
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
        let mac_port = self.assign_port(serial);
        let outcome = self.android.add_usb(serial, &ca, mac_port).await;
        match outcome {
            Ok(device) => {
                self.record_ready(&device)?;
                Ok(device)
            }
            Err(e) => {
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
                self.registry.release(&dev.serial);
                self.android.remove(&dev.serial).await.is_ok()
            }
            _ => false,
        };
        let conn = self.storage.conn().lock();
        conn.execute(
            "UPDATE device SET state='removed' WHERE id=?1",
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
        let mac_port = self.assign_port(serial);
        match self.android.add_usb(serial, &ca, mac_port).await {
            Ok(device) => {
                // Persist the outcome, so a device that had previously failed
                // clears its error banner once it comes good again.
                self.record_ready(&device)?;
                Ok(())
            }
            Err(e) => {
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
        self.registry.release(serial);
        self.android.remove(serial).await
    }

    pub async fn reapply_all_android_proxies(&self, ca: CaMaterial) -> Vec<String> {
        let serials: Vec<String> = self
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.platform == "android" && d.connection == "usb")
            .map(|d| d.serial)
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
        let mut cleaned = Vec::with_capacity(serials.len());
        for serial in serials {
            self.registry.release(&serial);
            if self.android.remove(&serial).await.is_ok() {
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
