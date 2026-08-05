use super::{CmdResult, CoreState};
use pane_ipc::{ReplayRecordDto, ReplaySendArgs};

#[tauri::command]
pub async fn send(state: CoreState<'_>, args: ReplaySendArgs) -> CmdResult<ReplayRecordDto> {
    state.replay_send(args).await
}
