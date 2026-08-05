//! `pane proxy run` — a foreground instance with no GUI.
//!
//! It hosts its own control socket, which is the point: `pane captures tail`
//! in another terminal behaves identically whether it is talking to the
//! desktop app or to this. `tail` therefore has exactly one implementation.
//!
//! Foreground, not a daemon: no double-fork, no PID file to reconcile, no
//! orphan cleanup, no "which log file". Agents and CI already background
//! processes perfectly well.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use pane_control::{ControlServer, InstanceKind};
use pane_core::{BootstrapError, Core, CoreConfig};
use serde_json::json;

use crate::output::{exit, note};

pub async fn run_foreground(data_dir: &Path, host: &str, port: u16, no_proxy: bool) -> Result<i32> {
    let core = match Core::bootstrap(CoreConfig {
        data_dir: Some(data_dir.to_path_buf()),
        take_instance_lock: true,
    }) {
        Ok(c) => Arc::new(c),
        Err(BootstrapError::AlreadyRunning { data_dir }) => {
            note(format!(
                "another Pane instance already owns {} — use it instead of starting a second one",
                data_dir.display()
            ));
            return Ok(exit::CONFLICT);
        }
        Err(e) => return Err(anyhow::Error::new(e)).context("bootstrapping the core"),
    };

    let (server, _serve) = ControlServer::bind(core.clone(), InstanceKind::Headless).await?;

    if !no_proxy {
        let session = core
            .proxy_start(pane_ipc::ProxyStartArgs {
                host: Some(host.to_string()),
                port: Some(port),
            })
            .await
            .map_err(anyhow::Error::new)?;
        note(format!("proxy listening on {}", session.listen));
    }

    // The same maintenance loops the GUI runs, so a headless instance
    // reconciles reconnected devices and prunes logcat identically.
    let watchdog = core.clone();
    tokio::spawn(async move { pane_core::background::device_watchdog(watchdog).await });
    let retention = core.clone();
    tokio::spawn(async move { pane_core::background::logcat_retention(retention).await });

    note(format!(
        "control socket at {}",
        pane_control::Discovery::socket_path_in(data_dir).display()
    ));
    note("ready — Ctrl-C to stop");

    // Machine-readable readiness on stdout, so a supervising script can block
    // on one line instead of polling for the socket to appear.
    crate::output::print_ndjson_line(&json!({
        "event": "ready",
        "kind": "headless",
        "proxy": !no_proxy,
        "data_dir": data_dir,
    }));

    wait_for_shutdown().await;
    note("shutting down");

    // Same teardown order as the GUI's RunEvent::Exit: stop the engine, clear
    // device proxy settings, revert the system proxy, then drop the socket.
    // Skipping this strands paired phones pointing at a dead 127.0.0.1:8888.
    if let Err(e) = core.proxy_stop().await {
        note(format!("stopping the proxy failed: {e}"));
    }
    server.cleanup();
    Ok(exit::OK)
}

/// Block until the process is asked to stop.
///
/// SIGTERM matters as much as Ctrl-C here: `kill`, systemd and CI job
/// cancellation all send it, and without handling it the teardown below never
/// runs — leaving paired phones pointing at a dead proxy and a stale socket
/// for the next start to clean up.
async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "cannot listen for SIGTERM; Ctrl-C only");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
