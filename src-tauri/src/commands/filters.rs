use super::{CmdResult, CoreState};
use pane_ipc::{FilterDto, SaveFilterArgs};
use uuid::Uuid;

#[tauri::command]
pub async fn filters_save(state: CoreState<'_>, args: SaveFilterArgs) -> CmdResult<FilterDto> {
    state.filter_save(args).await
}

#[tauri::command]
pub async fn filters_list(state: CoreState<'_>, kind: Option<String>) -> CmdResult<Vec<FilterDto>> {
    state.filters_list(kind.as_deref()).await
}

#[tauri::command]
pub async fn filters_delete(state: CoreState<'_>, id: Uuid) -> CmdResult<serde_json::Value> {
    state.filter_delete(id).await?;
    Ok(serde_json::json!({ "deleted": true }))
}
