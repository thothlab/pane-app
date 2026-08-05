use super::{to_api, CmdResult};
use crate::state::AppState;
use pane_ipc::kinds;
use pane_ipc::{
    CaptureBodyDto, CaptureDto, ClearArgs, ClearResult, ExportOneArgs, ExportOneResult,
    GetBodyArgs, ListCapturesArgs,
};
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub async fn captures_list(
    state: State<'_, AppState>,
    args: ListCapturesArgs,
) -> CmdResult<Vec<CaptureDto>> {
    state
        .storage
        .list_captures(args.filter.as_deref(), args.limit, args.before)
        .map_err(to_api(kinds::DB))
}

#[tauri::command]
pub async fn captures_get(state: State<'_, AppState>, id: Uuid) -> CmdResult<CaptureDto> {
    state
        .storage
        .get_capture(id)
        .map_err(to_api(kinds::NOT_FOUND))
}

#[tauri::command]
pub async fn get_body(state: State<'_, AppState>, args: GetBodyArgs) -> CmdResult<CaptureBodyDto> {
    state
        .storage
        .get_body(args.body_id, args.max_bytes)
        .map_err(to_api(kinds::NOT_FOUND))
}

#[tauri::command]
pub async fn clear(state: State<'_, AppState>, args: ClearArgs) -> CmdResult<ClearResult> {
    let n = state
        .storage
        .clear_captures(args.older_than)
        .map_err(to_api(kinds::DB))?;
    Ok(ClearResult { deleted: n as u64 })
}

#[tauri::command]
pub async fn export_one(
    state: State<'_, AppState>,
    args: ExportOneArgs,
) -> CmdResult<ExportOneResult> {
    state
        .storage
        .export_one(args.id, &args.format)
        .map_err(to_api(kinds::EXPORT_FAILED))
}

/// Write a text payload to a user-chosen path. Mirrors
/// `rules_export_write` - routing the write through Rust keeps the
/// renderer out of plugin-fs's per-capability scope rules. Used by the
/// Captures multi-select "Export" action, which builds a concatenated
/// OkHttp-style dump of the checked rows on the frontend and hands the
/// finished text here.
#[tauri::command]
pub async fn captures_export_write(path: String, content: String) -> CmdResult<usize> {
    let bytes = content.len();
    std::fs::write(&path, content).map_err(to_api(kinds::IO))?;
    Ok(bytes)
}
