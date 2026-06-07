use super::{to_api, CmdResult};
use crate::state::AppState;
use pane_ipc::{ReplayRecordDto, ReplaySendArgs};
use tauri::State;

#[tauri::command]
pub async fn send(state: State<'_, AppState>, args: ReplaySendArgs) -> CmdResult<ReplayRecordDto> {
    state
        .storage
        .replay_send(args)
        .await
        .map_err(to_api("replay_failed"))
}
