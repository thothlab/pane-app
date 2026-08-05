//! Finding a running instance, and cleaning up after one that died badly.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::protocol::PROTOCOL_VERSION;

pub const FILE_NAME: &str = "control.json";
pub const SOCKET_NAME: &str = "control.sock";

/// Conservative ceiling for `sun_path`: macOS allows 104 bytes including the
/// NUL, Linux 108. Staying a few bytes under both keeps one rule for all
/// platforms.
const MAX_SOCKET_PATH: usize = 96;

/// What kind of process owns the endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceKind {
    /// The desktop app.
    Gui,
    /// `pane proxy run`.
    Headless,
}

/// Contents of `<data_dir>/control.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovery {
    pub protocol: u32,
    pub pid: u32,
    pub app_version: String,
    pub kind: InstanceKind,
    /// Absolute path of the listening socket.
    pub endpoint: PathBuf,
    /// Which data directory this instance owns, so a client can confirm it is
    /// about to talk to the database it expects before falling back to
    /// reading that database directly.
    pub data_dir: PathBuf,
    pub started_at: String,
}

impl Discovery {
    pub fn path_in(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// Where the socket for `data_dir` lives.
    ///
    /// Normally right next to the database. But `sockaddr_un.sun_path` is 104
    /// bytes on macOS and 108 on Linux, and `bind()` fails outright when the
    /// path does not fit — which is easy to hit with a deep data directory
    /// (CI scratch dirs, sandboxes, a long username). Past the limit we fall
    /// back to a short per-user path under the temp directory.
    ///
    /// Clients never compute this themselves: `control.json` carries the
    /// actual path, so the fallback is invisible to them.
    pub fn socket_path_in(data_dir: &Path) -> PathBuf {
        let preferred = data_dir.join(SOCKET_NAME);
        if preferred.as_os_str().len() <= MAX_SOCKET_PATH {
            return preferred;
        }
        short_socket_path(data_dir)
    }

    pub fn read(data_dir: &Path) -> Result<Option<Self>> {
        let path = Self::path_in(data_dir);
        match std::fs::read_to_string(&path) {
            Ok(s) => Ok(Some(serde_json::from_str(&s).with_context(|| {
                format!("{} is not valid control metadata", path.display())
            })?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).context("reading control metadata"),
        }
    }

    /// Write atomically (temp file + rename) so a client never reads a
    /// half-written file, and 0600 so only the owning user can see the path.
    pub fn write(&self, data_dir: &Path) -> Result<()> {
        let final_path = Self::path_in(data_dir);
        let tmp = final_path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        set_owner_only(&tmp)?;
        std::fs::rename(&tmp, &final_path)?;
        Ok(())
    }

    pub fn is_compatible(&self) -> bool {
        self.protocol <= PROTOCOL_VERSION
    }
}

/// A short, stable, per-(user, data_dir) socket path under the temp directory.
///
/// Keyed by a hash of the data directory so two instances on different
/// directories cannot collide, and by uid so two users on one machine cannot
/// either. On macOS `$TMPDIR` is already per-user and 0700.
fn short_socket_path(data_dir: &Path) -> PathBuf {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in data_dir.as_os_str().as_encoded_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let uid = current_uid();
    std::env::temp_dir().join(format!("pane-{uid}-{hash:016x}.sock"))
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid() is always safe — it takes no arguments, touches no
    // memory and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(path)?.permissions();
    perm.set_mode(0o600);
    std::fs::set_permissions(path, perm)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

/// Remove a stale socket and metadata file before binding.
///
/// A Unix socket leaves its inode behind when the process is SIGKILLed, and
/// `bind()` then fails with `EADDRINUSE` — the same class of crash-residue
/// problem `host_proxy::self_heal_on_start` exists to solve for the system
/// proxy pointer. The caller must already hold the instance lock, which is
/// what proves no live process owns these files.
pub fn clear_stale(data_dir: &Path) {
    let sock = Discovery::socket_path_in(data_dir);
    if sock.exists() {
        if let Err(e) = std::fs::remove_file(&sock) {
            tracing::warn!(path = %sock.display(), error = %e, "could not remove stale control socket");
        }
    }
    let meta = Discovery::path_in(data_dir);
    if meta.exists() {
        let _ = std::fs::remove_file(&meta);
    }
}

/// Remove both files on clean shutdown.
pub fn cleanup(data_dir: &Path) {
    clear_stale(data_dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path) -> Discovery {
        Discovery {
            protocol: PROTOCOL_VERSION,
            pid: 1234,
            app_version: "0.2.8".into(),
            kind: InstanceKind::Gui,
            endpoint: Discovery::socket_path_in(dir),
            data_dir: dir.to_path_buf(),
            started_at: "2026-08-05T09:00:00Z".into(),
        }
    }

    #[test]
    fn round_trips_through_the_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Discovery::read(dir.path()).unwrap().is_none());

        sample(dir.path()).write(dir.path()).unwrap();
        let back = Discovery::read(dir.path()).unwrap().unwrap();
        assert_eq!(back.pid, 1234);
        assert_eq!(back.kind, InstanceKind::Gui);
        assert!(back.is_compatible());
    }

    #[cfg(unix)]
    #[test]
    fn metadata_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path()).write(dir.path()).unwrap();
        let mode = std::fs::metadata(Discovery::path_in(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_long_data_dir_falls_back_to_a_short_socket_path() {
        // Deep paths are common in CI scratch dirs and sandboxes, and bind()
        // fails hard rather than degrading, so this has to be handled.
        let deep = PathBuf::from(format!("/tmp/{}/data", "nested-directory-name/".repeat(8)));
        let sock = Discovery::socket_path_in(&deep);
        assert!(
            sock.as_os_str().len() <= MAX_SOCKET_PATH,
            "fallback path is still too long: {}",
            sock.display()
        );
        assert_ne!(sock, deep.join(SOCKET_NAME));
    }

    #[test]
    fn short_data_dirs_keep_the_socket_beside_the_database() {
        let dir = PathBuf::from("/tmp/pane");
        assert_eq!(Discovery::socket_path_in(&dir), dir.join(SOCKET_NAME));
    }

    #[test]
    fn different_data_dirs_get_different_fallback_paths() {
        let a = PathBuf::from(format!("/tmp/{}/a", "x".repeat(120)));
        let b = PathBuf::from(format!("/tmp/{}/b", "x".repeat(120)));
        assert_ne!(Discovery::socket_path_in(&a), Discovery::socket_path_in(&b));
    }

    #[test]
    fn a_newer_protocol_is_reported_as_incompatible() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = sample(dir.path());
        d.protocol = PROTOCOL_VERSION + 1;
        assert!(!d.is_compatible());
    }

    #[test]
    fn clear_stale_removes_both_files_and_tolerates_absence() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path()).write(dir.path()).unwrap();
        std::fs::write(Discovery::socket_path_in(dir.path()), b"").unwrap();

        clear_stale(dir.path());
        assert!(!Discovery::path_in(dir.path()).exists());
        assert!(!Discovery::socket_path_in(dir.path()).exists());

        clear_stale(dir.path()); // idempotent
    }
}
