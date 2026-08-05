//! `Core` — everything Pane can do, with no GUI attached.
//!
//! This is the former `src-tauri/src/state.rs::AppState`, plus the operation
//! bodies that used to live inline in the `#[tauri::command]` functions. The
//! Tauri commands are now thin adapters over these methods, and the CLI and
//! control server call exactly the same code.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use pane_ca::CaStore;
use pane_devices::DeviceManager;
use pane_engine::{DevicePortRegistry, EngineHandle, ProxyEngine};
use pane_storage::Storage;

use crate::config::CoreConfig;
use crate::events::EventBus;
use crate::lock::{InstanceLock, LockError};

/// Shutdown senders for running logcat streams, keyed by device serial.
///
/// Keyed by serial rather than by window label: the core has no concept of
/// windows, and the CLI needs to attach to the same stream. The GUI maps
/// `logcat-{serial}` ↔ serial on its side.
#[derive(Default)]
pub(crate) struct LogcatSessions(pub(crate) Mutex<HashMap<String, mpsc::Sender<()>>>);

pub struct Core {
    pub storage: Arc<Storage>,
    pub ca: Arc<CaStore>,
    pub devices: Arc<DeviceManager>,

    /// Shared serial↔port↔device_id registry, so device wiring and the proxy
    /// engine see the same map.
    pub registry: DevicePortRegistry,

    /// Long-lived event bus. Created once here and never replaced, so
    /// subscribers can attach at any time — see `events.rs` for why that
    /// matters.
    pub events: EventBus,

    pub(crate) proxy_handle: Mutex<Option<EngineHandle>>,

    /// Retained purely to keep the engine's `broadcast::Sender` alive for as
    /// long as the proxy runs. Dropping it (which is what the old code did on
    /// return from `start`) made late subscription impossible.
    pub(crate) engine: Mutex<Option<Arc<dyn ProxyEngine>>>,

    pub(crate) logcat: LogcatSessions,

    data_dir: PathBuf,

    /// Held for the process lifetime when this is an owning instance.
    /// Released by the kernel on exit, including SIGKILL.
    _lock: Option<InstanceLock>,

    /// "Capture this Mac" prior-proxy snapshot. `Some` ⇒ host capture is
    /// active; holds the exact network service and its prior web/secure proxy
    /// config so `host_proxy::disable` restores it verbatim. macOS-only.
    #[cfg(target_os = "macos")]
    pub(crate) host_proxy: Mutex<Option<crate::host_proxy::HostProxySnapshot>>,
}

/// Why `Core::bootstrap` failed.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error("another Pane instance is already running against {data_dir}")]
    AlreadyRunning { data_dir: PathBuf },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<LockError> for BootstrapError {
    fn from(e: LockError) -> Self {
        match e {
            LockError::AlreadyRunning { data_dir } => Self::AlreadyRunning { data_dir },
            other => Self::Other(anyhow::Error::new(other)),
        }
    }
}

impl Core {
    /// Bring up storage, the CA and the device manager against the configured
    /// data directory.
    pub fn bootstrap(config: CoreConfig) -> Result<Self, BootstrapError> {
        let data_dir = config.resolve_data_dir()?;
        std::fs::create_dir_all(&data_dir).map_err(anyhow::Error::from)?;

        // Take the lock before opening the database, so a losing instance
        // never runs migrations against a directory someone else owns.
        let lock = if config.take_instance_lock {
            Some(InstanceLock::acquire(&data_dir)?)
        } else {
            None
        };

        let storage = Arc::new(Storage::open(&data_dir)?);
        let ca = Arc::new(CaStore::open_or_init(&data_dir, &storage)?);
        let registry = DevicePortRegistry::new();
        let devices = Arc::new(DeviceManager::new(storage.clone(), registry.clone()));

        Ok(Self {
            storage,
            ca,
            devices,
            registry,
            events: EventBus::new(),
            proxy_handle: Mutex::new(None),
            engine: Mutex::new(None),
            logcat: LogcatSessions::default(),
            data_dir,
            _lock: lock,
            #[cfg(target_os = "macos")]
            host_proxy: Mutex::new(None),
        })
    }

    /// Attach to a data directory owned by someone else, without migrating it.
    ///
    /// `bootstrap` runs migrations, which is correct for the process that owns
    /// the directory and destructive for one that does not: the upgrade is
    /// one-way, and the installed app aborts on launch when it meets a
    /// migration version it has never seen. A CLI built from a newer checkout
    /// would brick the app just by listing captures — so a guest refuses on
    /// any schema mismatch instead.
    pub fn attach_unowned(config: CoreConfig) -> Result<Self, BootstrapError> {
        let data_dir = config.resolve_data_dir()?;
        let storage = Arc::new(Storage::open_unowned(&data_dir)?);
        let ca = Arc::new(CaStore::open_or_init(&data_dir, &storage)?);
        let registry = DevicePortRegistry::new();
        let devices = Arc::new(DeviceManager::new(storage.clone(), registry.clone()));

        Ok(Self {
            storage,
            ca,
            devices,
            registry,
            events: EventBus::new(),
            proxy_handle: Mutex::new(None),
            engine: Mutex::new(None),
            logcat: LogcatSessions::default(),
            data_dir,
            _lock: None,
            #[cfg(target_os = "macos")]
            host_proxy: Mutex::new(None),
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Whether the MITM proxy is currently running.
    pub fn proxy_running(&self) -> bool {
        self.proxy_handle.lock().is_some()
    }

    /// The address the proxy is listening on, if it is running.
    pub fn proxy_listen(&self) -> Option<String> {
        self.proxy_handle
            .lock()
            .as_ref()
            .map(|h| h.listen.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bootstrapping against a tempdir is the thing that was impossible
    /// before `data_dir` was threaded through `CaStore` — every CA path was
    /// derived from a hardcoded ProjectDirs lookup.
    #[test]
    fn bootstraps_into_an_explicit_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::bootstrap(CoreConfig::owning().with_data_dir(dir.path())).unwrap();

        assert_eq!(core.data_dir(), dir.path());
        assert!(dir.path().join("captures.db").exists());
        assert!(!core.proxy_running());
        assert!(core.proxy_listen().is_none());
        // The CA key fallback must land inside the tempdir, not in the
        // developer's real Application Support directory.
        assert!(dir.path().join("ca-keys").exists());
    }

    #[test]
    fn second_owning_bootstrap_on_the_same_dir_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let _first = Core::bootstrap(CoreConfig::owning().with_data_dir(dir.path())).unwrap();

        match Core::bootstrap(CoreConfig::owning().with_data_dir(dir.path())) {
            Err(BootstrapError::AlreadyRunning { .. }) => {}
            Err(e) => panic!("expected AlreadyRunning, got {e}"),
            Ok(_) => panic!("expected the second owning bootstrap to be refused"),
        }
    }

    /// A non-owning bootstrap must succeed while an owning one holds the
    /// lock — that is what lets read-only CLI queries work while the GUI is
    /// open.
    #[test]
    fn non_owning_bootstrap_coexists_with_the_lock_holder() {
        let dir = tempfile::tempdir().unwrap();
        let _owner = Core::bootstrap(CoreConfig::owning().with_data_dir(dir.path())).unwrap();

        let guest = Core::bootstrap(CoreConfig {
            data_dir: Some(dir.path().to_path_buf()),
            take_instance_lock: false,
        });
        assert!(guest.is_ok(), "non-owning bootstrap must not contend");
    }
}
