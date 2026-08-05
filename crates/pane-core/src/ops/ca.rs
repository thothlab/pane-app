use base64::Engine as _;

use pane_ipc::{kinds, CaCertificateDto, CaExportResult, CaSaveResult};

use crate::error::{api_err, to_api, CoreResult};
use crate::Core;

impl Core {
    pub async fn ca_current(&self) -> CoreResult<CaCertificateDto> {
        self.ca.current_dto().map_err(to_api(kinds::NO_CA))
    }

    pub async fn ca_rotate(&self) -> CoreResult<CaCertificateDto> {
        self.ca.rotate().map_err(to_api(kinds::ROTATE_FAILED))
    }

    /// Export the CA in `pem` | `der` | `qr` | `mobileconfig`.
    pub async fn ca_export(&self, format: &str) -> CoreResult<CaExportResult> {
        self.ca.export(format).map_err(to_api(kinds::EXPORT_FAILED))
    }

    /// Export and write to disk in one step.
    pub async fn ca_save_to_file(&self, format: &str, path: &str) -> CoreResult<CaSaveResult> {
        let exported = self.ca_export(format).await?;
        let b64 = exported
            .data_base64
            .ok_or_else(|| api_err("no_data", "exporter produced no data"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(to_api(kinds::DECODE))?;
        std::fs::write(path, &bytes).map_err(to_api(kinds::WRITE))?;
        Ok(CaSaveResult {
            path: path.to_string(),
            bytes_written: bytes.len() as u64,
        })
    }
}
