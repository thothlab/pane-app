use super::{to_api, CmdResult};
use crate::state::AppState;
use base64::Engine as _;
use pane_ipc::{CaCertificateDto, CaExportArgs, CaExportResult, CaSaveArgs, CaSaveResult};
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
pub async fn export(state: State<'_, AppState>, args: CaExportArgs) -> CmdResult<CaExportResult> {
    state
        .ca
        .export(&args.format)
        .map_err(to_api("export_failed"))
}

#[tauri::command]
pub async fn save_to_file(state: State<'_, AppState>, args: CaSaveArgs) -> CmdResult<CaSaveResult> {
    let exported = state
        .ca
        .export(&args.format)
        .map_err(to_api("export_failed"))?;
    let b64 = exported.data_base64.ok_or_else(|| pane_ipc::ApiError {
        kind: "no_data".into(),
        message: "exporter produced no data".into(),
        details: None,
    })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(to_api("decode"))?;
    std::fs::write(&args.path, &bytes).map_err(to_api("write"))?;
    Ok(CaSaveResult {
        path: args.path,
        bytes_written: bytes.len() as u64,
    })
}
