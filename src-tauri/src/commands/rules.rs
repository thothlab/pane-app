use super::{CmdResult, CoreState};
use pane_ipc::{
    CollectionSetEnabledArgs, CollectionSetPriorityArgs, CollectionUpsertArgs, RuleCollectionDto,
    RuleDto, RuleSetEnabledArgs, RuleSetPriorityArgs, RuleUpsertArgs,
};
use uuid::Uuid;

#[tauri::command]
pub async fn rules_list(state: CoreState<'_>) -> CmdResult<Vec<RuleDto>> {
    state.rules_list().await
}

#[tauri::command]
pub async fn rule_get(state: CoreState<'_>, id: Uuid) -> CmdResult<RuleDto> {
    state.rule_get(id).await
}

#[tauri::command]
pub async fn rule_upsert(state: CoreState<'_>, args: RuleUpsertArgs) -> CmdResult<RuleDto> {
    state.rule_upsert(args).await
}

#[tauri::command]
pub async fn rule_delete(state: CoreState<'_>, id: Uuid) -> CmdResult<()> {
    state.rule_delete(id).await
}

#[tauri::command]
pub async fn rule_set_enabled(state: CoreState<'_>, args: RuleSetEnabledArgs) -> CmdResult<()> {
    state.rule_set_enabled(args).await
}

/// Flip a whole scope of rules in one call. The collection header checkbox
/// used to fan out one call per rule because this did not exist.
#[tauri::command]
pub async fn rules_set_enabled_bulk(
    state: CoreState<'_>,
    args: pane_ipc::RulesSetEnabledBulkArgs,
) -> CmdResult<pane_ipc::RulesSetEnabledBulkResult> {
    state.rules_set_enabled_bulk(args).await
}

#[tauri::command]
pub async fn rule_set_priority(state: CoreState<'_>, args: RuleSetPriorityArgs) -> CmdResult<()> {
    state.rule_set_priority(args).await
}

#[tauri::command]
pub async fn collections_list(state: CoreState<'_>) -> CmdResult<Vec<RuleCollectionDto>> {
    state.collections_list().await
}

#[tauri::command]
pub async fn collection_upsert(
    state: CoreState<'_>,
    args: CollectionUpsertArgs,
) -> CmdResult<RuleCollectionDto> {
    state.collection_upsert(args).await
}

#[tauri::command]
pub async fn collection_delete(state: CoreState<'_>, id: Uuid) -> CmdResult<()> {
    state.collection_delete(id).await
}

#[tauri::command]
pub async fn collection_set_enabled(
    state: CoreState<'_>,
    args: CollectionSetEnabledArgs,
) -> CmdResult<()> {
    state.collection_set_enabled(args).await
}

#[tauri::command]
pub async fn collection_set_priority(
    state: CoreState<'_>,
    args: CollectionSetPriorityArgs,
) -> CmdResult<()> {
    state.collection_set_priority(args).await
}

/// Write a text payload to a user-chosen path. Same shape as
/// `logcat_write_export` — keeps the renderer out of plugin-fs's
/// per-capability scope rules. Used by the Rules import/export feature.
#[tauri::command]
pub async fn rules_export_write(path: String, content: String) -> CmdResult<usize> {
    pane_core::write_text_file(&path, &content)
}

/// Read a text payload back, for the import flow.
#[tauri::command]
pub async fn rules_import_read(path: String) -> CmdResult<String> {
    pane_core::read_text_file(&path)
}
