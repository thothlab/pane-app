use uuid::Uuid;

use pane_ipc::{
    kinds, AndroidToolingStatusDto, DeviceDto, DiscoveredDeviceDto, RemoveDeviceResult,
};

use crate::error::{api_err, to_api, CoreResult};
use crate::Core;

impl Core {
    /// What is plugged in right now and could be paired.
    pub async fn devices_attached(&self) -> CoreResult<Vec<DiscoveredDeviceDto>> {
        self.devices
            .discover_attached()
            .await
            .map_err(to_api(kinds::TOOLING_MISSING))
    }

    pub async fn device_add_ios(&self, serial: &str) -> CoreResult<DeviceDto> {
        self.devices
            .add_ios_usb(serial, self.ca.material())
            .await
            .map_err(to_api(kinds::IOS_ADD_FAILED))
    }

    pub async fn device_add_android(&self, serial: &str) -> CoreResult<DeviceDto> {
        // Refuse to wire up a device while the proxy is stopped — otherwise
        // we'd push http_proxy=127.0.0.1:8888 onto the phone with nothing
        // listening, and the user would lose all internet on the device (the
        // typical symptom: "I added a device and now nothing works").
        if !self.proxy_running() {
            return Err(api_err(
                kinds::PROXY_NOT_RUNNING,
                "Start the proxy first, then add the device. Otherwise the \
                 device would point at a dead 127.0.0.1:8888 and lose internet.",
            ));
        }
        self.devices
            .add_android_usb(serial, self.ca.material())
            .await
            .map_err(to_api(kinds::ANDROID_ADD_FAILED))
    }

    pub async fn device_remove(&self, id: Uuid) -> CoreResult<RemoveDeviceResult> {
        self.devices
            .remove(id)
            .await
            .map_err(to_api(kinds::REMOVE_FAILED))
    }

    // `DeviceManager` reads through `Storage`'s connection mutex like
    // everything else, so these two go to the blocking pool for the same
    // reason the `Core::db` helper exists.
    pub async fn device_get(&self, id: Uuid) -> CoreResult<DeviceDto> {
        let devices = self.devices.clone();
        tokio::task::spawn_blocking(move || devices.get(id))
            .await
            .map_err(to_api(kinds::DB))?
            .map_err(to_api(kinds::NOT_FOUND))
    }

    pub async fn devices_list(&self) -> CoreResult<Vec<DeviceDto>> {
        let devices = self.devices.clone();
        tokio::task::spawn_blocking(move || devices.list())
            .await
            .map_err(to_api(kinds::DB))?
            .map_err(to_api(kinds::DB))
    }

    pub async fn android_tooling_status(&self) -> CoreResult<AndroidToolingStatusDto> {
        Ok(self.devices.android_tooling_status())
    }
}
