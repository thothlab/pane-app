//! The operation surface.
//!
//! One module per namespace, mirroring `src/ipc/client.ts` so the GUI, the
//! CLI and the control protocol all use the same vocabulary. Each is an
//! `impl Core` block; the bodies came straight out of the old
//! `#[tauri::command]` functions, which are now adapters over these.

mod ca;
mod captures;
mod devices;
mod filters;
mod host;
mod logcat;
mod proxy;
mod replay;
mod rules;

pub use captures::{read_text_file, write_text_file};
pub use logcat::LogcatStartOutcome;

use pane_storage::Storage;

impl crate::Core {
    /// Run a blocking `Storage` call off the async runtime's worker threads.
    ///
    /// Every operation here is `async fn` but rusqlite is synchronous, and
    /// `Storage` serializes on a `parking_lot::Mutex` that does not yield. A
    /// direct call therefore parks a runtime worker for the whole duration of
    /// the query *and* of any wait for the lock. With a logcat firehose
    /// holding that lock and the UI polling on a timer, all workers ended up
    /// parked within seconds and no command — not even a clipboard write —
    /// could start. That was the "app freezes when I right-click Copy" bug:
    /// copying is a four-call chain, so it was simply the most likely thing to
    /// be caught by it.
    ///
    /// The blocking pool is sized for exactly this (512 threads by default),
    /// so waiting there costs a cheap thread instead of a scarce worker.
    ///
    /// Returns the storage error untouched so call sites keep their own
    /// `to_api(kind)` mapping; a panic in the closure surfaces as an error
    /// rather than taking the process down.
    pub(crate) async fn db<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&Storage) -> anyhow::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || f(&storage))
            .await
            .map_err(|e| anyhow::anyhow!("database task failed: {e}"))?
    }
}
