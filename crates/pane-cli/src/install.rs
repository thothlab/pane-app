//! `pane install` — put this binary on PATH under the name users type.
//!
//! The cargo bin is `pane-cli` because src-tauri's package is already named
//! `pane` and two bin targets cannot share an output filename. Users type
//! `pane`, so the gap is closed with a symlink — the same split VS Code uses
//! between its bundled binary and the `code` command.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::output::{exit, note};

const LINK_NAME: &str = "pane";

pub fn install(dir: Option<&Path>) -> Result<i32> {
    let exe = std::env::current_exe().context("locating this executable")?;
    let target_dir = match dir {
        Some(d) => d.to_path_buf(),
        None => pick_dir()?,
    };
    std::fs::create_dir_all(&target_dir)
        .with_context(|| format!("creating {}", target_dir.display()))?;

    let link = target_dir.join(LINK_NAME);
    // Replace an existing link so re-running after an upgrade is a no-op
    // rather than an error. A *regular file* there is not ours to remove.
    if let Ok(meta) = std::fs::symlink_metadata(&link) {
        if meta.file_type().is_symlink() {
            std::fs::remove_file(&link).ok();
        } else {
            anyhow::bail!(
                "{} already exists and is not a symlink — remove it first",
                link.display()
            );
        }
    }

    symlink(&exe, &link)
        .with_context(|| format!("linking {} → {}", link.display(), exe.display()))?;

    note(format!("{} → {}", link.display(), exe.display()));
    if !on_path(&target_dir) {
        note(format!(
            "{} is not on your PATH — add it, e.g. `export PATH=\"{}:$PATH\"`",
            target_dir.display(),
            target_dir.display()
        ));
    }
    Ok(exit::OK)
}

/// Prefer a system location, but never fail just because it needs root.
fn pick_dir() -> Result<PathBuf> {
    let system = PathBuf::from("/usr/local/bin");
    if is_writable(&system) {
        return Ok(system);
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local").join("bin"))
}

fn is_writable(dir: &Path) -> bool {
    dir.is_dir()
        && std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(".pane-write-probe"))
            .map(|_| {
                let _ = std::fs::remove_file(dir.join(".pane-write-probe"));
                true
            })
            .unwrap_or(false)
}

fn on_path(dir: &Path) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|entry| entry == dir))
        .unwrap_or(false)
}

#[cfg(unix)]
fn symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(windows)]
fn symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(src, dst)
}
