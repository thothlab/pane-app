refinery::embed_migrations!("migrations");

pub fn runner() -> refinery::Runner {
    migrations::runner()
}

/// Highest migration version compiled into this binary.
///
/// Compared against what a database actually has, so a client can refuse to
/// touch a database owned by a different build instead of silently migrating
/// it — an upgrade is one-way, and an older app aborts on startup when it
/// finds a migration it does not know.
pub fn latest_version() -> i32 {
    migrations::runner()
        .get_migrations()
        .iter()
        .map(|m| m.version() as i32)
        .max()
        .unwrap_or(0)
}
