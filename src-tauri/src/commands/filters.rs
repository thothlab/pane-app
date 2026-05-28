use super::{to_api, CmdResult};
use crate::state::AppState;
use mycharles_ipc::{FilterDto, SaveFilterArgs};
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub async fn save(state: State<'_, AppState>, args: SaveFilterArgs) -> CmdResult<FilterDto> {
    state.storage.save_filter(args).map_err(to_api("db"))
}

#[tauri::command]
pub async fn list(state: State<'_, AppState>) -> CmdResult<Vec<FilterDto>> {
    state.storage.list_filters().map_err(to_api("db"))
}

#[tauri::command]
pub async fn delete(state: State<'_, AppState>, id: Uuid) -> CmdResult<serde_json::Value> {
    state.storage.delete_filter(id).map_err(to_api("db"))?;
    Ok(serde_json::json!({ "deleted": true }))
}
