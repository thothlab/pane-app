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
    let caps = storage.list_captures(None, 10, None).expect("list captures");
    for c in &caps {
        assert!(!c.server_host.is_empty(), "row decoded into the wrong columns");
    }
    // Idempotent: opening again must be a no-op, not a re-run.
    let _ = Storage::open(dir.path()).expect("second open");
}
