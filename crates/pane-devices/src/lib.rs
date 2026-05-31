//! DeviceManager: cross-platform device discovery and state machine.
//!
//! Delegates the iOS- and Android-specific work to `pane-ios` and
//! `pane-android` sibling crates. Persists every state transition in
//! SQLite so the UI list survives restarts.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use pane_android::AndroidPlatform;
use pane_ca::CaMaterial;
use pane_ios::IosPlatform;
use pane_ipc::{DeviceDto, DiscoveredDeviceDto, RemoveDeviceResult};
use pane_storage::Storage;
use rusqlite::params;
use time::OffsetDateTime;
use uuid::Uuid;

pub struct DeviceManager {
    storage: Arc<Storage>,
    ios: IosPlatform,
    android: AndroidPlatform,
}

impl DeviceManager {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            ios: IosPlatform::new(),
            android: AndroidPlatform::new(),
        }
    }

    pub async fn discover_attached(&self) -> Result<Vec<DiscoveredDeviceDto>> {
        let mut out = Vec::new();
        out.extend(self.ios.discover().await.unwrap_or_default());
        out.extend(self.android.discover().await.unwrap_or_default());
        Ok(out)
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
        let outcome = self.android.add_usb(serial, &ca).await;
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
            "android" => self.android.remove(&dev.serial).await.is_ok(),
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
            .and_then(|s| OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok())
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
