//! Deciding who executes an operation.
//!
//! Both modes speak the same op vocabulary from `pane_control::dispatch`, so
//! every command has exactly one implementation regardless of whether a GUI is
//! open. That is the whole reason the dispatch table is a separate module.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use pane_control::{Client, ConnectError};
use pane_core::{Core, CoreConfig};

pub enum Session {
    /// Talking to a running instance (GUI or `pane proxy run`) over its
    /// control socket. Shares that instance's live state.
    Attached(Client),
    /// Nothing is running, so we opened the data directory ourselves.
    Direct(Arc<Core>),
}

/// Operations that cannot be satisfied without a live instance, because they
/// need the running proxy engine or its in-memory state.
const NEEDS_INSTANCE: &[&str] = &[
    "proxy.stop",
    "host.enable",
    "host.disable",
    "devices.add_android",
    "devices.add_ios",
    "events.subscribe",
];

impl Session {
    pub async fn open(data_dir: PathBuf) -> Result<Self> {
        match Client::connect(&data_dir).await {
            Ok(c) => Ok(Session::Attached(c)),
            Err(ConnectError::NotRunning) => {
                // attach_unowned, not bootstrap: bootstrap migrates, and
                // migrating a directory owned by an installed app stops that
                // app from launching ever again. A guest reads and writes the
                // schema it finds, or refuses.
                let core = Core::attach_unowned(CoreConfig {
                    data_dir: Some(data_dir),
                    take_instance_lock: false,
                })
                .context("opening the Pane data directory")?;
                Ok(Session::Direct(Arc::new(core)))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn is_attached(&self) -> bool {
        matches!(self, Session::Attached(_))
    }

    /// Run one op, wherever it belongs.
    pub async fn call(&mut self, op: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        match self {
            Session::Attached(c) => c.call(op, params).await,
            Session::Direct(core) => {
                if NEEDS_INSTANCE.contains(&op) {
                    anyhow::bail!(
                        "`{op}` needs a running Pane instance. Start one with \
                         `pane proxy run` (or open the Pane app), then retry."
                    );
                }
                pane_control::dispatch::dispatch(core, op, params)
                    .await
                    .map_err(anyhow::Error::new)
            }
        }
    }
}

/// The data directory this invocation should use: `--data-dir`, then
/// `$PANE_DATA_DIR`, then the platform default.
pub fn resolve_data_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    CoreConfig {
        data_dir: explicit,
        take_instance_lock: false,
    }
    .resolve_data_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_needing_an_instance_are_all_real_op_names() {
        for op in NEEDS_INSTANCE {
            assert!(
                pane_control::dispatch::OPS.contains(op),
                "`{op}` is gated but is not a real op — a rename would silently \
                 un-gate it"
            );
        }
    }

    #[test]
    fn explicit_data_dir_wins() {
        let p = PathBuf::from("/tmp/pane-test-dir");
        assert_eq!(resolve_data_dir(Some(p.clone())).unwrap(), p);
    }
}
