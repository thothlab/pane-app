//! Shared app state — proxy engine, storage, devices, CA store.
//!
//! Built once at startup and passed to all Tauri commands via `tauri::State`.

use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;

use pane_ca::CaStore;
use pane_devices::DeviceManager;
use pane_engine::EngineHandle;
use pane_storage::Storage;

pub struct AppState {
    pub storage: Arc<Storage>,
    pub ca: Arc<CaStore>,
    pub devices: Arc<DeviceManager>,
    pub proxy_handle: Mutex<Option<EngineHandle>>,
}

impl AppState {
    pub fn bootstrap() -> Result<Self> {
        let dirs = directories::ProjectDirs::from("tech", "thothlab", "pane")
            .ok_or_else(|| anyhow::anyhow!("no project dirs"))?;

        let data_dir = dirs.data_dir().to_path_buf();
        std::fs::create_dir_all(&data_dir)?;

        let storage = Arc::new(Storage::open(&data_dir)?);
        let ca = Arc::new(CaStore::open_or_init(&data_dir, &storage)?);
        let devices = Arc::new(DeviceManager::new(storage.clone()));

        Ok(Self {
            storage,
            ca,
            devices,
            proxy_handle: Mutex::new(None),
        })
    }
}
