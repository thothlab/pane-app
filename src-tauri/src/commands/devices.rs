use super::{to_api, CmdResult};
use crate::state::AppState;
use pane_ipc::{
    AddDeviceArgs, AndroidToolingStatusDto, DeviceDto, DiscoveredDeviceDto, RemoveDeviceArgs,
    RemoveDeviceResult,
};
use tauri::State;
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
#[tauri::command]
pub async fn list_attached_usb(state: State<'_, AppState>) -> CmdResult<Vec<DiscoveredDeviceDto>> {
    Ok(state.devices.discover_attached().await.unwrap_or_else(|e| {
        tracing::debug!(error = %e, "attached-device enumeration failed; showing empty list");
        Vec::new()
    }))
}

/// Pairing (and its UI twin, Re-sync) re-pushes the CA and rebuilds the tunnel,
/// so any earlier "this host won't accept our certificate" verdict was reached
/// against a setup that no longer exists. Clearing here is what makes Re-sync
/// mean something: before, a user who fixed CA trust on the phone still saw
/// every host tunnelled until they thought to restart the proxy.
fn clear_tunnelled_after_pairing(state: &AppState) {
    let forgotten = state.no_mitm.reset();
    if forgotten > 0 {
        tracing::info!(
            hosts = forgotten,
            "device paired/re-synced; cleared tunnelled-host set"
        );
    }
}

#[tauri::command]
pub async fn add_ios_usb(state: State<'_, AppState>, args: AddDeviceArgs) -> CmdResult<DeviceDto> {
    let device = state
        .devices
        .add_ios_usb(&args.serial, state.ca.material())
        .await
        .map_err(to_api("ios_add_failed"))?;
    clear_tunnelled_after_pairing(&state);
    Ok(device)
}

#[tauri::command]
pub async fn add_android_usb(
    state: State<'_, AppState>,
    args: AddDeviceArgs,
) -> CmdResult<DeviceDto> {
    // Refuse to wire up a device while the proxy is stopped — otherwise
    // we'd push http_proxy=127.0.0.1:8888 onto the phone with nothing
    // listening, and the user would lose all internet on the device
    // (the typical symptom: "I added a device and now nothing works").
    if state.proxy_handle.lock().is_none() {
        return Err(to_api("proxy_not_running")(anyhow::anyhow!(
            "Start the proxy first (Start proxy button in the sidebar), \
             then add the device. Otherwise the device would point at a \
             dead 127.0.0.1:8888 and lose internet."
        )));
    }
    let device = state
        .devices
        .add_android_usb(&args.serial, state.ca.material())
        .await
        .map_err(to_api("android_add_failed"))?;
    clear_tunnelled_after_pairing(&state);
    Ok(device)
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

#[tauri::command]
pub async fn android_tooling_status(
    state: State<'_, AppState>,
) -> CmdResult<AndroidToolingStatusDto> {
    Ok(state.devices.android_tooling_status())
}
