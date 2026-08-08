use super::{CmdResult, CoreState};
use pane_ipc::{
    AddDeviceArgs, AndroidToolingStatusDto, DeviceDto, DiscoveredDeviceDto, RemoveDeviceArgs,
    RemoveDeviceResult,
};
use uuid::Uuid;

/// Devices plugged in right now, for the "Attached over USB" list.
///
/// Deliberately degrades to an empty list instead of surfacing the error.
/// `DeviceManager::discover_attached` propagates adb failures because the
/// watchdog genuinely needs to tell "adb hiccuped" from "nothing is plugged
/// in" — but this screen already has a dedicated channel for that: the
/// `android_tooling_status` banner, which explains a missing platform-tools
/// install far better than a failed list would.
///
/// It also can't afford to fail. The frontend reads this through a bare
/// `createResource` with no ErrorBoundary above it, so a rejected promise
/// takes the whole Devices view down — including the very banner that would
/// tell the user what to install.
///
/// The degrade lives here rather than in `Core::devices_attached` on purpose:
/// it is a property of this one screen, not of the operation. `pane devices
/// attached` still gets the `TOOLING_MISSING` error, because a CLI that
/// printed an empty list when adb is missing would be lying to a script.
#[tauri::command]
pub async fn list_attached_usb(state: CoreState<'_>) -> CmdResult<Vec<DiscoveredDeviceDto>> {
    Ok(state.devices_attached().await.unwrap_or_else(|e| {
        tracing::debug!(error = %e, "attached-device enumeration failed; showing empty list");
        Vec::new()
    }))
}

#[tauri::command]
pub async fn add_ios_usb(state: CoreState<'_>, args: AddDeviceArgs) -> CmdResult<DeviceDto> {
    state.device_add_ios(&args.serial).await
}

#[tauri::command]
pub async fn add_android_usb(state: CoreState<'_>, args: AddDeviceArgs) -> CmdResult<DeviceDto> {
    state.device_add_android(&args.serial).await
}

#[tauri::command]
pub async fn remove(state: CoreState<'_>, args: RemoveDeviceArgs) -> CmdResult<RemoveDeviceResult> {
    state.device_remove(args.id).await
}

#[tauri::command]
pub async fn devices_get(state: CoreState<'_>, id: Uuid) -> CmdResult<DeviceDto> {
    state.device_get(id).await
}

#[tauri::command]
pub async fn devices_list(state: CoreState<'_>) -> CmdResult<Vec<DeviceDto>> {
    state.devices_list().await
}

#[tauri::command]
pub async fn android_tooling_status(state: CoreState<'_>) -> CmdResult<AndroidToolingStatusDto> {
    state.android_tooling_status().await
}
