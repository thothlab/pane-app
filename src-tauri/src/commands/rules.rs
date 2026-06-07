use super::{to_api, CmdResult};
use crate::state::AppState;
use pane_ipc::{
    CollectionSetEnabledArgs, CollectionUpsertArgs, RuleCollectionDto, RuleDto, RuleSetEnabledArgs,
    RuleUpsertArgs,
};
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
pub async fn rule_upsert(state: State<'_, AppState>, args: RuleUpsertArgs) -> CmdResult<RuleDto> {
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

#[tauri::command]
pub async fn collections_list(state: State<'_, AppState>) -> CmdResult<Vec<RuleCollectionDto>> {
    state.storage.list_collections().map_err(to_api("db"))
}

#[tauri::command]
pub async fn collection_upsert(
    state: State<'_, AppState>,
    args: CollectionUpsertArgs,
) -> CmdResult<RuleCollectionDto> {
    state.storage.upsert_collection(args).map_err(to_api("db"))
}

#[tauri::command]
pub async fn collection_delete(state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    state.storage.delete_collection(id).map_err(to_api("db"))
}

#[tauri::command]
pub async fn collection_set_enabled(
    state: State<'_, AppState>,
    args: CollectionSetEnabledArgs,
) -> CmdResult<()> {
    state
        .storage
        .set_collection_enabled(args)
        .map_err(to_api("db"))
}
