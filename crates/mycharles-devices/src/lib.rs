//! DeviceManager: cross-platform device discovery and state machine.
//!
//! Delegates the iOS- and Android-specific work to `mycharles-ios` and
//! `mycharles-android` sibling crates. Persists every state transition in
//! SQLite so the UI list survives restarts.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use mycharles_android::AndroidPlatform;
use mycharles_ca::CaMaterial;
use mycharles_ios::IosPlatform;
use mycharles_ipc::{DeviceDto, DiscoveredDeviceDto, RemoveDeviceResult};
use mycharles_storage::Storage;
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
                self.transition("ios", serial, "ready", None)?;
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
                self.transition("android", serial, "ready", None)?;
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
