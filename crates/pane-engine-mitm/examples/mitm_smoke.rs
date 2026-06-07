//! Headless smoke: spins up the full MITM engine + PAC server with a
//! freshly-generated CA, so you can `adb reverse` to it and verify
//! end-to-end traffic without booting the Tauri shell.
//!
//!     cargo run --example mitm_smoke -p pane-engine-mitm
//!
//! On another terminal:
//!     adb reverse tcp:8888 tcp:8888
//!     adb reverse tcp:8889 tcp:8889
//!     adb shell settings put global http_proxy_pac http://127.0.0.1:8889/proxy.pac
//!
//! Watch tracing output. Ctrl-C to shut down.

use std::sync::Arc;

use pane_ca::CaStore;
use pane_engine::{EngineConfig, ProxyEngine};
use pane_engine_mitm::MitmEngine;
use pane_storage::Storage;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let tmp = std::env::temp_dir().join("pane-mitm-smoke");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let storage = Arc::new(Storage::open(&tmp)?);
    let ca = Arc::new(CaStore::open_or_init(&tmp, &storage)?);

    let listen: std::net::SocketAddr = "127.0.0.1:8888".parse().unwrap();
    // Session row must exist before proxy_loop attaches captures to it.
    storage.session_record(listen)?;

    let engine: Arc<dyn ProxyEngine> = Arc::new(MitmEngine::new(storage.clone()));
    let handle = engine
        .start(EngineConfig {
            listen,
            ca: ca.material(),
            pac_listen: Some("127.0.0.1:8889".parse().unwrap()),
        })
        .await?;

    eprintln!(
        "[mitm-smoke] up listen={} pac={:?} data_dir={}",
        handle.listen,
        handle.pac_listen,
        tmp.display()
    );
    tokio::signal::ctrl_c().await?;
    handle.shutdown().await?;
    Ok(())
}
