use super::{CmdResult, CoreState};
use pane_ipc::{
    CaptureBodyDto, CaptureDto, ClearArgs, ClearResult, ExportOneArgs, ExportOneResult,
    GetBodyArgs, ListCapturesArgs,
};
use uuid::Uuid;

#[tauri::command]
pub async fn captures_list(
    state: CoreState<'_>,
    args: ListCapturesArgs,
) -> CmdResult<Vec<CaptureDto>> {
    state.captures_list(args).await
}

#[tauri::command]
pub async fn captures_get(state: CoreState<'_>, id: Uuid) -> CmdResult<CaptureDto> {
    state.capture_get(id).await
}

#[tauri::command]
pub async fn get_body(state: CoreState<'_>, args: GetBodyArgs) -> CmdResult<CaptureBodyDto> {
    state.capture_body(args.body_id, args.max_bytes).await
}

#[tauri::command]
pub async fn clear(state: CoreState<'_>, args: ClearArgs) -> CmdResult<ClearResult> {
    state.captures_clear(args.older_than).await
}

#[tauri::command]
pub async fn export_one(state: CoreState<'_>, args: ExportOneArgs) -> CmdResult<ExportOneResult> {
    state.capture_export(args.id, &args.format).await
}

/// Write a text payload to a user-chosen path. Routing the write through Rust
/// keeps the renderer out of plugin-fs's per-capability scope rules. Used by
/// the Captures multi-select "Export" action.
#[tauri::command]
pub async fn captures_export_write(path: String, content: String) -> CmdResult<usize> {
    pane_core::write_text_file(&path, &content)
}
