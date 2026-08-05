//! "Capture this Mac". See [`crate::host_proxy`] for the platform work.

use pane_ipc::{kinds, HostCaptureStatusDto};

use crate::error::{to_api, CoreResult};
use crate::Core;

impl Core {
    pub async fn host_capture_enable(&self) -> CoreResult<HostCaptureStatusDto> {
        let service =
            crate::host_proxy::enable(self).map_err(to_api(kinds::HOST_CAPTURE_ENABLE))?;
        Ok(HostCaptureStatusDto {
            enabled: true,
            service: Some(service),
        })
    }

    pub async fn host_capture_disable(&self) -> CoreResult<()> {
        crate::host_proxy::disable(self).map_err(to_api(kinds::HOST_CAPTURE_DISABLE))
    }

    pub async fn host_capture_status(&self) -> CoreResult<HostCaptureStatusDto> {
        let (enabled, service) = crate::host_proxy::status(self);
        Ok(HostCaptureStatusDto { enabled, service })
    }
}
