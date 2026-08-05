use super::{CmdResult, CoreState};
use pane_ipc::{ProxyStartArgs, ProxyStatusDto, SessionDto};

// No AppHandle here any more: engine events are pumped onto the core event
// bus by `Core::proxy_start`, and `lib.rs` runs a single forwarder that
// relays the bus to the webview. That also means a second subscriber (the
// CLI's `tail`) can attach at any time — which the old per-start forwarder
// made impossible.

#[tauri::command]
pub async fn start(state: CoreState<'_>, args: ProxyStartArgs) -> CmdResult<SessionDto> {
    state.proxy_start(args).await
}

#[tauri::command]
pub async fn stop(state: CoreState<'_>) -> CmdResult<serde_json::Value> {
    state.proxy_stop().await
}

#[tauri::command]
pub async fn status(state: CoreState<'_>) -> CmdResult<ProxyStatusDto> {
    state.proxy_status().await
}
