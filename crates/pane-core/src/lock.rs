//! Single-instance guard.
//!
//! Two Pane processes sharing one data directory is a genuine hazard: both
//! try to bind 8888 (the second fails deep inside engine start, with a
//! confusing message) and both write the same SQLite file. Once the CLI can
//! start a headless instance, the chance of it happening goes up sharply.
//!
//! An advisory file lock is the right tool because **the kernel releases it
//! when the process dies, including on SIGKILL**. That is precisely the
//! property `host_proxy::self_heal_on_start` had to reimplement by hand for
//! the system-proxy pointer — here we get it for free, so there is no stale
//! lock to clean up and no PID file to second-guess.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;

/// Held for the lifetime of an owning `Core`. Dropping it (or the process
/// exiting for any reason) releases the lock.
#[derive(Debug)]
pub struct InstanceLock {
    path: PathBuf,
    // Kept solely to hold the lock; the kernel unlocks on close.
    _file: File,
}

/// Why the lock could not be taken.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("another Pane instance is already using {data_dir}")]
    AlreadyRunning { data_dir: PathBuf },
    #[error("could not open the instance lock at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl InstanceLock {
    pub const FILE_NAME: &'static str = "instance.lock";

    /// Try to take the exclusive lock for `data_dir`, without blocking.
    pub fn acquire(data_dir: &Path) -> Result<Self, LockError> {
        let path = data_dir.join(Self::FILE_NAME);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| LockError::Io {
                path: path.clone(),
                source,
            })?;

        match file.try_lock_exclusive() {
            Ok(()) => Ok(Self { path, _file: file }),
            // fs2 reports contention as an io::Error whose kind is WouldBlock
            // on unix and PermissionDenied on Windows; treat any failure of a
            // *non-blocking* attempt as "someone else holds it", which is the
            // only thing a caller can act on.
            Err(_) => Err(LockError::AlreadyRunning {
                data_dir: data_dir.to_path_buf(),
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_is_refused_and_release_frees_it() {
        let dir = tempfile::tempdir().unwrap();
        let first = InstanceLock::acquire(dir.path()).expect("first lock");

        match InstanceLock::acquire(dir.path()) {
            Err(LockError::AlreadyRunning { .. }) => {}
            other => panic!("expected AlreadyRunning, got {other:?}"),
        }

        drop(first);
        InstanceLock::acquire(dir.path()).expect("lock is reusable once released");
    }

    #[test]
    fn separate_data_dirs_do_not_contend() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let _la = InstanceLock::acquire(a.path()).unwrap();
        let _lb = InstanceLock::acquire(b.path()).unwrap();
    }
}
