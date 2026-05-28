use super::{to_api, CmdResult};
use crate::state::AppState;
use mycharles_ipc::{CaCertificateDto, CaExportArgs, CaExportResult};
use tauri::State;

#[tauri::command]
pub async fn current(state: State<'_, AppState>) -> CmdResult<CaCertificateDto> {
    state.ca.current_dto().map_err(to_api("no_ca"))
}

#[tauri::command]
pub async fn rotate(state: State<'_, AppState>) -> CmdResult<CaCertificateDto> {
    state.ca.rotate().map_err(to_api("rotate_failed"))
}

#[tauri::command]
pub async fn export(
    state: State<'_, AppState>,
    args: CaExportArgs,
) -> CmdResult<CaExportResult> {
    state.ca.export(&args.format).map_err(to_api("export_failed"))
}
