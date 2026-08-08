use uuid::Uuid;

use pane_ipc::{
    kinds, CollectionSetEnabledArgs, CollectionSetPriorityArgs, CollectionUpsertArgs,
    RuleCollectionDto, RuleDto, RuleSetEnabledArgs, RuleSetPriorityArgs, RuleUpsertArgs,
    RulesSetEnabledBulkArgs, RulesSetEnabledBulkResult,
};

use crate::error::{to_api, CoreResult};
use crate::Core;

impl Core {
    pub async fn rules_list(&self) -> CoreResult<Vec<RuleDto>> {
        self.storage.list_rules().map_err(to_api(kinds::DB))
    }

    pub async fn rule_get(&self, id: Uuid) -> CoreResult<RuleDto> {
        self.storage.get_rule(id).map_err(to_api(kinds::NOT_FOUND))
    }

    /// Create or update a rule.
    ///
    /// Takes effect immediately, including for a proxy running in another
    /// process: `proxy_loop` calls `list_active_rules()` per request rather
    /// than caching.
    pub async fn rule_upsert(&self, args: RuleUpsertArgs) -> CoreResult<RuleDto> {
        self.storage.upsert_rule(args).map_err(to_api(kinds::DB))
    }

    pub async fn rule_delete(&self, id: Uuid) -> CoreResult<()> {
        self.storage.delete_rule(id).map_err(to_api(kinds::DB))
    }

    pub async fn rule_set_enabled(&self, args: RuleSetEnabledArgs) -> CoreResult<()> {
        self.storage
            .set_rule_enabled(args)
            .map_err(to_api(kinds::DB))
    }

    /// Enable or disable a whole scope of rules at once.
    pub async fn rules_set_enabled_bulk(
        &self,
        args: RulesSetEnabledBulkArgs,
    ) -> CoreResult<RulesSetEnabledBulkResult> {
        self.storage
            .set_rules_enabled_bulk(args)
            .map_err(to_api(kinds::DB))
    }

    pub async fn rule_set_priority(&self, args: RuleSetPriorityArgs) -> CoreResult<()> {
        self.storage
            .set_rule_priority(args)
            .map_err(to_api(kinds::DB))
    }

    pub async fn collections_list(&self) -> CoreResult<Vec<RuleCollectionDto>> {
        self.storage.list_collections().map_err(to_api(kinds::DB))
    }

    pub async fn collection_upsert(
        &self,
        args: CollectionUpsertArgs,
    ) -> CoreResult<RuleCollectionDto> {
        self.storage
            .upsert_collection(args)
            .map_err(to_api(kinds::DB))
    }

    pub async fn collection_delete(&self, id: Uuid) -> CoreResult<()> {
        self.storage
            .delete_collection(id)
            .map_err(to_api(kinds::DB))
    }

    pub async fn collection_set_enabled(&self, args: CollectionSetEnabledArgs) -> CoreResult<()> {
        self.storage
            .set_collection_enabled(args)
            .map_err(to_api(kinds::DB))
    }

    pub async fn collection_set_priority(&self, args: CollectionSetPriorityArgs) -> CoreResult<()> {
        self.storage
            .set_collection_priority(args)
            .map_err(to_api(kinds::DB))
    }
}
