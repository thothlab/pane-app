use uuid::Uuid;

use pane_ipc::{
    kinds, CollectionSetEnabledArgs, CollectionSetPriorityArgs, CollectionUpsertArgs,
    RuleCollectionDto, RuleDto, RuleSetEnabledArgs, RuleSetPriorityArgs, RuleUpsertArgs,
    RulesAddTagsBulkArgs, RulesAddTagsBulkResult, RulesDeleteBulkArgs, RulesDeleteBulkResult,
    RulesSetEnabledBulkArgs, RulesSetEnabledBulkResult,
};

use crate::error::{to_api, CoreResult};
use crate::Core;

impl Core {
    pub async fn rules_list(&self) -> CoreResult<Vec<RuleDto>> {
        self.db(|s| s.list_rules()).await.map_err(to_api(kinds::DB))
    }

    pub async fn rule_get(&self, id: Uuid) -> CoreResult<RuleDto> {
        self.db(move |s| s.get_rule(id))
            .await
            .map_err(to_api(kinds::NOT_FOUND))
    }

    /// Create or update a rule.
    ///
    /// Takes effect immediately, including for a proxy running in another
    /// process: `proxy_loop` calls `list_active_rules()` per request rather
    /// than caching.
    pub async fn rule_upsert(&self, args: RuleUpsertArgs) -> CoreResult<RuleDto> {
        self.db(move |s| s.upsert_rule(args))
            .await
            .map_err(to_api(kinds::DB))
    }

    pub async fn rule_delete(&self, id: Uuid) -> CoreResult<()> {
        self.db(move |s| s.delete_rule(id))
            .await
            .map_err(to_api(kinds::DB))
    }

    pub async fn rule_set_enabled(&self, args: RuleSetEnabledArgs) -> CoreResult<()> {
        self.db(move |s| s.set_rule_enabled(args))
            .await
            .map_err(to_api(kinds::DB))
    }

    /// Enable or disable a whole scope of rules at once.
    pub async fn rules_set_enabled_bulk(
        &self,
        args: RulesSetEnabledBulkArgs,
    ) -> CoreResult<RulesSetEnabledBulkResult> {
        self.db(move |s| s.set_rules_enabled_bulk(args))
            .await
            .map_err(to_api(kinds::DB))
    }

    /// Delete a whole scope of rules at once — one transaction, so a big
    /// "clear the library" cannot half-finish.
    pub async fn rules_delete_bulk(
        &self,
        args: RulesDeleteBulkArgs,
    ) -> CoreResult<RulesDeleteBulkResult> {
        self.db(move |s| s.delete_rules_bulk(args))
            .await
            .map_err(to_api(kinds::DB))
    }

    /// Add labels to a whole scope of rules at once — the bulk form of the
    /// editor's Tags field, used by "tag the selected rules".
    pub async fn rules_add_tags_bulk(
        &self,
        args: RulesAddTagsBulkArgs,
    ) -> CoreResult<RulesAddTagsBulkResult> {
        self.db(move |s| s.add_rules_tags_bulk(args))
            .await
            .map_err(to_api(kinds::DB))
    }

    pub async fn rule_set_priority(&self, args: RuleSetPriorityArgs) -> CoreResult<()> {
        self.db(move |s| s.set_rule_priority(args))
            .await
            .map_err(to_api(kinds::DB))
    }

    pub async fn collections_list(&self) -> CoreResult<Vec<RuleCollectionDto>> {
        self.db(|s| s.list_collections())
            .await
            .map_err(to_api(kinds::DB))
    }

    pub async fn collection_upsert(
        &self,
        args: CollectionUpsertArgs,
    ) -> CoreResult<RuleCollectionDto> {
        self.db(move |s| s.upsert_collection(args))
            .await
            .map_err(to_api(kinds::DB))
    }

    pub async fn collection_delete(&self, id: Uuid) -> CoreResult<()> {
        self.db(move |s| s.delete_collection(id))
            .await
            .map_err(to_api(kinds::DB))
    }

    pub async fn collection_set_enabled(&self, args: CollectionSetEnabledArgs) -> CoreResult<()> {
        self.db(move |s| s.set_collection_enabled(args))
            .await
            .map_err(to_api(kinds::DB))
    }

    pub async fn collection_set_priority(&self, args: CollectionSetPriorityArgs) -> CoreResult<()> {
        self.db(move |s| s.set_collection_priority(args))
            .await
            .map_err(to_api(kinds::DB))
    }
}
