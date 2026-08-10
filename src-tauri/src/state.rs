//! Shared app state — proxy engine, storage, devices, CA store.
//!
//! Built once at startup and passed to all Tauri commands via `tauri::State`.

use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;

use pane_ca::CaStore;
use pane_devices::DeviceManager;
use pane_engine::{DevicePortRegistry, EngineHandle, NoMitmSet};
use pane_storage::Storage;

pub struct AppState {
    pub storage: Arc<Storage>,
    pub ca: Arc<CaStore>,
    pub devices: Arc<DeviceManager>,
    pub proxy_handle: Mutex<Option<EngineHandle>>,
    /// Shared serial↔port↔device_id registry. Owned here so both the device
    /// wiring (DeviceManager) and the proxy engine (via EngineConfig) see the
    /// same map.
    pub registry: DevicePortRegistry,
    /// Hosts the proxy has given up decrypting. Lives here, not in the engine,
    /// so the UI can list and clear it and so the paths that invalidate it
    /// (CA rotation, pairing a device) can reach it without a running proxy.
    /// `proxy.start`/`proxy.stop` reset it, so its effective scope is still one
    /// proxy run — it just stopped being unreachable state.
    pub no_mitm: NoMitmSet,
    /// "Capture this Mac" prior-proxy snapshot. `Some` ⇒ host capture is
    /// active; holds the exact network service and its prior web/secure proxy
    /// config so `host_proxy::disable` restores it verbatim. macOS-only.
    #[cfg(target_os = "macos")]
    pub host_proxy: Mutex<Option<crate::host_proxy::HostProxySnapshot>>,
}

impl AppState {
    pub fn bootstrap() -> Result<Self> {
        let dirs = directories::ProjectDirs::from("tech", "thothlab", "pane")
            .ok_or_else(|| anyhow::anyhow!("no project dirs"))?;

        let data_dir = dirs.data_dir().to_path_buf();
        std::fs::create_dir_all(&data_dir)?;

        let storage = Arc::new(Storage::open(&data_dir)?);
        let ca = Arc::new(CaStore::open_or_init(&data_dir, &storage)?);
        let registry = DevicePortRegistry::new();
        let devices = Arc::new(DeviceManager::new(storage.clone(), registry.clone()));

        Ok(Self {
            storage,
            ca,
            devices,
            proxy_handle: Mutex::new(None),
            registry,
            no_mitm: NoMitmSet::new(),
            #[cfg(target_os = "macos")]
            host_proxy: Mutex::new(None),
        })
    }
}
