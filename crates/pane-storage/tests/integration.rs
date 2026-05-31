//! Integration tests for the storage layer: schema + body roundtrip + filter DSL.

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
