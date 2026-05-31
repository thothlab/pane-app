use super::{to_api, CmdResult};
use crate::state::AppState;
use pane_ipc::{RuleDto, RuleSetEnabledArgs, RuleUpsertArgs};
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub async fn rules_list(state: State<'_, AppState>) -> CmdResult<Vec<RuleDto>> {
    state.storage.list_rules().map_err(to_api("db"))
}

#[tauri::command]
pub async fn rule_get(state: State<'_, AppState>, id: Uuid) -> CmdResult<RuleDto> {
    state.storage.get_rule(id).map_err(to_api("not_found"))
}

#[tauri::command]
pub async fn rule_upsert(
    state: State<'_, AppState>,
    args: RuleUpsertArgs,
) -> CmdResult<RuleDto> {
    state.storage.upsert_rule(args).map_err(to_api("db"))
}

#[tauri::command]
pub async fn rule_delete(state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    state.storage.delete_rule(id).map_err(to_api("db"))
}

#[tauri::command]
pub async fn rule_set_enabled(
    state: State<'_, AppState>,
    args: RuleSetEnabledArgs,
) -> CmdResult<()> {
    state.storage.set_rule_enabled(args).map_err(to_api("db"))
}
