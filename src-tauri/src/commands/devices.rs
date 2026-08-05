use super::{CmdResult, CoreState};
use pane_ipc::{
    AddDeviceArgs, AndroidToolingStatusDto, DeviceDto, DiscoveredDeviceDto, RemoveDeviceArgs,
    RemoveDeviceResult,
};
use uuid::Uuid;

#[tauri::command]
pub async fn list_attached_usb(state: CoreState<'_>) -> CmdResult<Vec<DiscoveredDeviceDto>> {
    state.devices_attached().await
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
