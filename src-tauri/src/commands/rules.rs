use super::{to_api, CmdResult};
use crate::state::AppState;
use pane_ipc::{
    CollectionSetEnabledArgs, CollectionSetPriorityArgs, CollectionUpsertArgs, RuleCollectionDto,
    RuleDto, RuleSetEnabledArgs, RuleSetPriorityArgs, RuleUpsertArgs,
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
pub async fn rule_set_priority(
    state: State<'_, AppState>,
    args: RuleSetPriorityArgs,
) -> CmdResult<()> {
    state.storage.set_rule_priority(args).map_err(to_api("db"))
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

#[tauri::command]
pub async fn collection_set_priority(
    state: State<'_, AppState>,
    args: CollectionSetPriorityArgs,
) -> CmdResult<()> {
    state
        .storage
        .set_collection_priority(args)
        .map_err(to_api("db"))
}

/// Write a text payload to a user-chosen path. Same shape as
/// `logcat_write_export` — we keep the renderer out of plugin-fs's
/// per-capability scope rules by routing through a thin Rust command.
/// Used by the Rules import/export feature to dump the curated set
/// as a `.pane-rules.json` the user can share.
#[tauri::command]
pub async fn rules_export_write(path: String, content: String) -> CmdResult<usize> {
    let bytes = content.len();
    std::fs::write(&path, content).map_err(to_api("io"))?;
    Ok(bytes)
}

/// Read a text payload from a user-chosen path. Paired with
/// `rules_export_write` so the import flow can pull the JSON back in
/// without plugin-fs scope rules.
#[tauri::command]
pub async fn rules_import_read(path: String) -> CmdResult<String> {
    std::fs::read_to_string(&path).map_err(to_api("io"))
}
