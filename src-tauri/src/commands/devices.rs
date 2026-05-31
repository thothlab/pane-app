use super::{to_api, CmdResult};
use crate::state::AppState;
use pane_ipc::{
    AddDeviceArgs, DeviceDto, DiscoveredDeviceDto, RemoveDeviceArgs, RemoveDeviceResult,
};
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub async fn list_attached_usb(state: State<'_, AppState>) -> CmdResult<Vec<DiscoveredDeviceDto>> {
    state
        .devices
        .discover_attached()
        .await
        .map_err(to_api("tooling_missing"))
}

#[tauri::command]
pub async fn add_ios_usb(
    state: State<'_, AppState>,
    args: AddDeviceArgs,
) -> CmdResult<DeviceDto> {
    state
        .devices
        .add_ios_usb(&args.serial, state.ca.material())
        .await
        .map_err(to_api("ios_add_failed"))
}

#[tauri::command]
pub async fn add_android_usb(
    state: State<'_, AppState>,
    args: AddDeviceArgs,
) -> CmdResult<DeviceDto> {
    state
        .devices
        .add_android_usb(&args.serial, state.ca.material())
        .await
        .map_err(to_api("android_add_failed"))
}

#[tauri::command]
pub async fn remove(
    state: State<'_, AppState>,
    args: RemoveDeviceArgs,
) -> CmdResult<RemoveDeviceResult> {
    state
        .devices
        .remove(args.id)
        .await
        .map_err(to_api("remove_failed"))
}

#[tauri::command]
pub async fn devices_get(state: State<'_, AppState>, id: Uuid) -> CmdResult<DeviceDto> {
    state.devices.get(id).map_err(to_api("not_found"))
}

#[tauri::command]
pub async fn devices_list(state: State<'_, AppState>) -> CmdResult<Vec<DeviceDto>> {
    state.devices.list().map_err(to_api("db"))
}
