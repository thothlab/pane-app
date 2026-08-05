//! Where Pane keeps its state on disk, and how a caller can override it.

use std::path::PathBuf;

use anyhow::{anyhow, Result};

/// Environment override for the data directory. Mirrors the convention
/// `apps/web` already uses. Primarily for tests and for running a second
/// instance against a scratch directory without touching the user's real
/// captures.
pub const DATA_DIR_ENV: &str = "PANE_DATA_DIR";

/// How to bring up a [`Core`](crate::Core).
#[derive(Debug, Clone, Default)]
pub struct CoreConfig {
    /// Override the data directory. `None` means: read `$PANE_DATA_DIR`, and
    /// fall back to the platform default.
    ///
    /// This exists because `directories::ProjectDirs` used to be called at
    /// four separate sites, which made it impossible to point a test at a
    /// tempdir — anything touching the CA would scribble into the developer's
    /// real Application Support directory.
    pub data_dir: Option<PathBuf>,

    /// Take the exclusive single-instance lock. The GUI and a headless run
    /// both want this; a read-only CLI query does not, because it has to work
    /// while the GUI holds the lock.
    pub take_instance_lock: bool,
}

impl CoreConfig {
    /// Config for a full owning instance — the GUI, or `pane proxy run`.
    pub fn owning() -> Self {
        Self {
            data_dir: None,
            take_instance_lock: true,
        }
    }

    /// Point at an explicit directory (tests, scratch instances).
    pub fn with_data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }

    /// Resolve the effective data directory without creating it.
    pub fn resolve_data_dir(&self) -> Result<PathBuf> {
        if let Some(dir) = &self.data_dir {
            return Ok(dir.clone());
        }
        if let Some(dir) = std::env::var_os(DATA_DIR_ENV) {
            let dir = PathBuf::from(dir);
            if !dir.as_os_str().is_empty() {
                return Ok(dir);
            }
        }
        default_data_dir()
    }
}

/// The platform data directory: `~/Library/Application Support/tech.thothlab.pane`
/// on macOS, `~/.local/share/pane` on Linux, `%APPDATA%\thothlab\pane\data` on
/// Windows.
pub fn default_data_dir() -> Result<PathBuf> {
    directories::ProjectDirs::from("tech", "thothlab", "pane")
        .map(|d| d.data_dir().to_path_buf())
        .ok_or_else(|| anyhow!("no project dirs"))
}
