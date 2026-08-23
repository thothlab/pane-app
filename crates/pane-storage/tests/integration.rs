//! Integration tests for the storage layer: schema + body roundtrip + filter DSL.

use pane_ipc::{RuleBulkScope, RuleUpsertArgs, RulesAddTagsBulkArgs};
use pane_storage::Storage;
use tempfile::tempdir;

#[test]
fn opens_and_runs_migrations() {
    let dir = tempdir().unwrap();
    let _storage = Storage::open(dir.path()).unwrap();
    let again = Storage::open(dir.path()).unwrap();
    assert_eq!(again.captures_count().unwrap(), 0);
}

#[test]
fn empty_filter_returns_all() {
    let dir = tempdir().unwrap();
    let storage = Storage::open(dir.path()).unwrap();
    let rows = storage.list_captures(None, 10, None).unwrap();
    assert!(rows.is_empty());
}

/// A bulk tag MERGES. The thing worth pinning is what it must not do: a rule
/// that already carries labels keeps them, a label that differs only in case is
/// not added a second time, and a rule outside the scope is not touched.
#[test]
fn add_tags_bulk_merges_without_erasing() {
    let dir = tempdir().unwrap();
    let storage = Storage::open(dir.path()).unwrap();

    let rule = |name: &str, tags: Vec<String>| RuleUpsertArgs {
        id: None,
        name: name.into(),
        enabled: true,
        enabled_scope: None,
        devices: None,
        priority: 0,
        collection_id: None,
        mode: "stub".into(),
        patches: vec![],
        match_host_glob: Some("api.example.com".into()),
        match_method: None,
        match_path_glob: None,
        match_params: vec![],
        match_req_body: None,
        match_conditions: vec![],
        tags,
        res_status: 200,
        res_headers: vec![],
        res_body_id: None,
        res_body_base64: None,
        res_body_mime: None,
        res_delay_ms: 0,
    };

    let tagged = storage
        .upsert_rule(rule("already-labelled", vec!["Smoke".into()]))
        .unwrap();
    let bare = storage.upsert_rule(rule("bare", vec![])).unwrap();
    let outside = storage.upsert_rule(rule("outside", vec![])).unwrap();

    let res = storage
        .add_rules_tags_bulk(RulesAddTagsBulkArgs {
            scope: RuleBulkScope::Ids {
                ids: vec![tagged.id, bare.id],
            },
            // "smoke" only differs from the existing label by case, so the
            // first rule gains exactly one tag, not two.
            tags: vec!["smoke".into(), "ios".into()],
        })
        .unwrap();
    assert_eq!(res.matched, 2);
    assert_eq!(res.changed, 2);

    assert_eq!(
        storage.get_rule(tagged.id).unwrap().tags,
        vec!["Smoke".to_string(), "ios".to_string()]
    );
    assert_eq!(
        storage.get_rule(bare.id).unwrap().tags,
        vec!["smoke".to_string(), "ios".to_string()]
    );
    assert!(storage.get_rule(outside.id).unwrap().tags.is_empty());

    // Running it again changes nothing — `matched` still reports the scope was
    // real, so a caller can tell this from a stale id list.
    let again = storage
        .add_rules_tags_bulk(RulesAddTagsBulkArgs {
            scope: RuleBulkScope::Ids {
                ids: vec![tagged.id, bare.id],
            },
            tags: vec!["smoke".into(), "ios".into()],
        })
        .unwrap();
    assert_eq!(again.matched, 2);
    assert_eq!(again.changed, 0);
}
