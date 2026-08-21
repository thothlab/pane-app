//! Guards the refinery path against a *populated* database.
//!
//! `cargo test` normally migrates a database it just created, which is the one
//! case that can't catch an ALTER that conflicts with existing schema or data.
//! A version collision between parallel branches has already shipped a startup
//! SIGABRT this way. When a real Pane database is present on this machine, run
//! the migrations against a copy of it; otherwise skip.

use std::path::PathBuf;

use pane_storage::Storage;

fn real_db() -> Option<PathBuf> {
    let p = directories::ProjectDirs::from("tech", "thothlab", "pane")?
        .data_dir()
        .join("captures.db");
    p.is_file().then_some(p)
}

#[test]
fn migrations_apply_to_an_existing_database() {
    let Some(src) = real_db() else {
        eprintln!("no local Pane database; skipping");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::copy(&src, dir.path().join("captures.db")).expect("copy db");

    let storage = Storage::open(dir.path()).expect("migrations must apply to a populated db");
    // Reading a capture back exercises the SELECT column lists against the
    // migrated schema — an off-by-one in the projection shows up here and
    // nowhere else.
    let caps = storage
        .list_captures(None, 10, None)
        .expect("list captures");
    for c in &caps {
        assert!(
            !c.server_host.is_empty(),
            "row decoded into the wrong columns"
        );
    }
    // Same check for the rules side. Its projections are the longest in the
    // file and are read positionally (`map_rule_row` takes column indices),
    // so a column appended in the wrong place decodes every field after it
    // into the wrong slot — silently, on the user's real library only.
    //
    // `mode` is checked rather than anything numeric: it sits late in the
    // projection (column 16 of 20) with a closed set of values, so a shifted
    // read lands a timestamp or a JSON blob in it. Row *data* is deliberately
    // not asserted — real libraries hold rules with odd-but-valid values
    // (res_status 0 from a capture that never got a status, say), and a test
    // that fails on those is testing the user's data, not the schema.
    let rules = storage.list_rules().expect("list rules");
    for r in &rules {
        assert!(
            r.mode == "stub" || r.mode == "patch",
            "rule row decoded into the wrong columns: {r:?}"
        );
        assert!(
            !r.created_at.is_empty(),
            "rule row decoded into the wrong columns: {r:?}"
        );
    }
    let collections = storage.list_collections().expect("list collections");
    for c in &collections {
        assert!(
            !c.created_at.is_empty(),
            "collection row decoded into the wrong columns: {c:?}"
        );
    }
    // The engine's own view of the library — a third projection over the same
    // table, easy to update in two places out of three.
    // `None` = the global plane, the same view the proxy takes for traffic
    // it cannot attribute to a device.
    let _ = storage.list_active_rules(None).expect("list active rules");

    // Idempotent: opening again must be a no-op, not a re-run.
    let _ = Storage::open(dir.path()).expect("second open");
}
