//! Tauri command adapters.
//!
//! Every operation lives in `pane_core::Core`. These functions exist only to
//! expose it to the webview: unwrap the managed state, call the core method,
//! hand back the result. Anything with real logic in it belongs in the core,
//! where the CLI and control server can reach it too.

pub mod ca;
pub mod captures;
pub mod devices;
pub mod filters;
pub mod host;
pub mod logcat;
pub mod passthrough;
pub mod proxy;
pub mod replay;
pub mod rules;

use pane_ipc::ApiError;

pub type CmdResult<T> = Result<T, ApiError>;

/// The managed state handed to every command.
pub type CoreState<'a> = tauri::State<'a, std::sync::Arc<pane_core::Core>>;
