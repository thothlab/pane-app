//! Tauri commands for "Capture this Mac" — trust the CA and route the Mac's
//! own system proxy through Pane. See `crate::host_proxy` for the platform
//! implementation (macOS) and stubs (elsewhere).

use super::{to_api, CmdResult};
use crate::host_proxy;
use crate::state::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct HostCaptureStatusDto {
    pub enabled: bool,
    pub service: Option<String>,
}

#[tauri::command]
pub async fn host_capture_enable(state: State<'_, AppState>) -> CmdResult<HostCaptureStatusDto> {
    let service = host_proxy::enable(&state).map_err(to_api("host_capture_enable"))?;
    Ok(HostCaptureStatusDto {
        enabled: true,
        service: Some(service),
    })
}

#[tauri::command]
pub async fn host_capture_disable(state: State<'_, AppState>) -> CmdResult<()> {
    host_proxy::disable(&state).map_err(to_api("host_capture_disable"))
}

#[tauri::command]
pub async fn host_capture_status(state: State<'_, AppState>) -> CmdResult<HostCaptureStatusDto> {
    let (enabled, service) = host_proxy::status(&state);
    Ok(HostCaptureStatusDto { enabled, service })
}
