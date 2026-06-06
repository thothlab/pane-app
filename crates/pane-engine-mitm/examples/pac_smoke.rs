//! Stand-alone smoke binary: spins up just the PAC server on
//! 127.0.0.1:8889 so the device-side install flow can be exercised
//! without booting the full Tauri app. Run with:
//!
//!     cargo run --example pac_smoke -p pane-engine-mitm
//!
//! Then on the connected device:
//!     adb reverse tcp:8889 tcp:8889
//!     adb shell settings put global http_proxy_pac http://127.0.0.1:8889/proxy.pac
//!     # open a browser — verify it loads sites through the (non-existent) proxy
//!     # then test the unplug scenario:
//!     adb reverse --remove tcp:8889
//!     # browser should fall back to DIRECT and keep loading.

use pane_engine_mitm::__pac_smoke_helper;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    __pac_smoke_helper().await
}
