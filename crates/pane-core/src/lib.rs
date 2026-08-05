//! Pane's headless core.
//!
//! Everything Pane can do, with no GUI attached: storage, the root CA, device
//! management, the MITM proxy engine, and the ~50 operations behind them.
//!
//! # Why this crate exists
//!
//! The operations used to live inline in `#[tauri::command]` functions, which
//! made them reachable only from the webview. Lifting them here lets three
//! front ends share one implementation:
//!
//!   - the Tauri commands in `src-tauri`, now thin adapters;
//!   - the local control server, so an external process can drive a running
//!     instance;
//!   - the `pane` CLI, which either forwards to a running instance or brings
//!     up its own [`Core`].
//!
//! The domain crates below it (`pane-storage`, `pane-ca`, `pane-devices`,
//! `pane-engine*`) were already GUI-independent; this crate is the seam that
//! makes that reusable rather than incidental.

pub mod background;
pub mod config;
pub mod error;
pub mod events;
pub mod helper_apk;
pub mod host_proxy;
pub mod lock;

mod core;
mod ops;

pub use crate::core::{BootstrapError, Core};
pub use config::{default_data_dir, CoreConfig, DATA_DIR_ENV};
pub use error::{api_err, to_api, CoreResult};
pub use events::{topics, CoreEvent, EventBus};
pub use helper_apk::HELPER_APK_ENV;
pub use lock::{InstanceLock, LockError};
/// Stateless file helpers the GUI routes through Rust to sidestep plugin-fs
/// capability scopes.
pub use ops::{read_text_file, write_text_file, LogcatStartOutcome};
