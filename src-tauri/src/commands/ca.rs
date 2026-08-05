use super::{CmdResult, CoreState};
use pane_ipc::{CaCertificateDto, CaExportArgs, CaExportResult, CaSaveArgs, CaSaveResult};

#[tauri::command]
pub async fn current(state: CoreState<'_>) -> CmdResult<CaCertificateDto> {
    state.ca_current().await
}

#[tauri::command]
pub async fn rotate(state: CoreState<'_>) -> CmdResult<CaCertificateDto> {
    state.ca_rotate().await
}

#[tauri::command]
pub async fn export(state: CoreState<'_>, args: CaExportArgs) -> CmdResult<CaExportResult> {
    state.ca_export(&args.format).await
}

#[tauri::command]
pub async fn save_to_file(state: CoreState<'_>, args: CaSaveArgs) -> CmdResult<CaSaveResult> {
    state.ca_save_to_file(&args.format, &args.path).await
}
