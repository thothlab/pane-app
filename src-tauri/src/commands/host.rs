//! "Capture this Mac" — trust the CA and route the Mac's own system proxy
//! through Pane. Platform work lives in `pane_core::host_proxy`.

use super::{CmdResult, CoreState};
use pane_ipc::HostCaptureStatusDto;

#[tauri::command]
pub async fn host_capture_enable(state: CoreState<'_>) -> CmdResult<HostCaptureStatusDto> {
    state.host_capture_enable().await
}

#[tauri::command]
pub async fn host_capture_disable(state: CoreState<'_>) -> CmdResult<()> {
    state.host_capture_disable().await
}

#[tauri::command]
pub async fn host_capture_status(state: CoreState<'_>) -> CmdResult<HostCaptureStatusDto> {
    state.host_capture_status().await
}
