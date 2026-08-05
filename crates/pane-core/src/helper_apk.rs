//! Locating the bundled Android companion APK.
//!
//! Production builds get the path from Tauri's `resource_dir()`, which only
//! the GUI can resolve — that branch stays in `src-tauri`. The dev-mode probe
//! lives here so headless runs can find it too.

use std::path::{Path, PathBuf};

/// Explicit override, for headless runs and packagers that put the APK
/// somewhere neither probe would look.
pub const HELPER_APK_ENV: &str = "PANE_HELPER_APK";

/// Probe for the helper APK without Tauri.
///
/// Order: `$PANE_HELPER_APK`, then the repo layout relative to the running
/// executable (`target/{debug,release}/pane` → `../../../src-tauri/binaries`).
///
/// Only returns a path that exists *and* is non-empty: the committed
/// placeholder is 0 bytes until CI populates it, and installing that would
/// fail confusingly rather than falling through to "watchdog disabled".
pub fn probe() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(HELPER_APK_ENV) {
        let p = PathBuf::from(p);
        if is_non_empty(&p) {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let repo_root = exe.parent()?.parent()?.parent()?;
    let p = repo_root
        .join("src-tauri")
        .join("binaries")
        .join("pane-helper.apk");
    is_non_empty(&p).then_some(p)
}

/// True when the path exists and holds at least one byte.
pub fn is_non_empty(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn zero_byte_placeholder_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.apk");
        std::fs::File::create(&empty).unwrap();
        assert!(!is_non_empty(&empty));

        let full = dir.path().join("full.apk");
        std::fs::File::create(&full)
            .unwrap()
            .write_all(b"x")
            .unwrap();
        assert!(is_non_empty(&full));
    }

    #[test]
    fn missing_path_is_rejected() {
        assert!(!is_non_empty(Path::new("/nonexistent/pane-helper.apk")));
    }
}
