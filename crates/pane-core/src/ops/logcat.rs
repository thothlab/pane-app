//! Logcat streaming and queries.
//!
//! The `adb logcat` child used to be spawned by `logcat_open`, which also
//! built a `WebviewWindow` — so the stream was only reachable from the GUI.
//! The stream half lives here now, keyed by device serial; window creation
//! stays in `src-tauri`. That is what lets `pane logcat tail` attach to the
//! same session the GUI window is showing.
//!
//! The firehose still never crosses IPC: each batch is persisted to SQLite
//! and only a `{serial, inserted}` count is published on the bus. Consumers
//! re-query. `INSERT OR IGNORE` means adb's ring-buffer re-dump on reconnect
//! inserts 0 rows and suppresses the ping entirely, so nothing churns.

use std::collections::HashMap;

use pane_android::logcat::{spawn as spawn_logcat, LogcatConfig, LogcatEvent};
use pane_android::AndroidPlatform;
use time::OffsetDateTime;

use pane_ipc::{
    kinds, ClearResult, LogcatExportArgs, LogcatNewCountArgs, LogcatQueryArgs,
    LogcatQueryOlderArgs, LogcatRowDto,
};
use pane_storage::LogcatInsert;

use crate::error::{to_api, CoreResult};
use crate::events::topics;
use crate::Core;

/// Result of [`Core::logcat_start`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogcatStartOutcome {
    /// True when a stream for this serial was already running, so nothing new
    /// was spawned. Callers use it to focus an existing window instead of
    /// opening a second one.
    pub reused: bool,
}

impl Core {
    /// Start streaming `adb logcat` for a device, persisting batches.
    ///
    /// Idempotent per serial — a second call while a stream is live returns
    /// `reused: true` and does not spawn another `adb` child.
    pub fn logcat_start(&self, serial: &str) -> CoreResult<LogcatStartOutcome> {
        if self.logcat.0.lock().contains_key(serial) {
            return Ok(LogcatStartOutcome { reused: true });
        }

        let storage = self.storage.clone();
        let bus = self.events.clone();
        let serial_owned = serial.to_string();
        let cfg = LogcatConfig {
            serial: serial.to_string(),
            ..Default::default()
        };

        let shutdown_tx = spawn_logcat(cfg, move |ev| match ev {
            LogcatEvent::Batch(entries) => {
                // Persist first — the DB is the source of truth. One
                // created_at stamp shared across the batch.
                let created_at_ms =
                    (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
                let rows: Vec<LogcatInsert> = entries
                    .iter()
                    .map(|e| LogcatInsert {
                        device_ts: e.timestamp.clone(),
                        pid: e.pid,
                        tid: e.tid,
                        level: e.level as i64,
                        tag: e.tag.clone(),
                        message: e.message.clone(),
                    })
                    .collect();
                let inserted =
                    match storage.insert_logcat_batch(&serial_owned, created_at_ms, &rows) {
                        Ok(n) => n,
                        Err(e) => {
                            tracing::warn!(error = %e, "logcat: db insert failed");
                            0
                        }
                    };
                if inserted > 0 {
                    bus.publish_topic(
                        topics::LOGCAT_APPENDED,
                        serde_json::json!({ "serial": serial_owned, "inserted": inserted }),
                    );
                }
            }
            LogcatEvent::Error(msg) => {
                bus.publish_topic(
                    topics::LOGCAT_ERROR,
                    serde_json::json!({ "serial": serial_owned, "message": msg }),
                );
            }
        })
        .map_err(to_api(kinds::LOGCAT_SPAWN))?;

        self.logcat.0.lock().insert(serial.to_string(), shutdown_tx);
        Ok(LogcatStartOutcome { reused: false })
    }

    /// Stop the stream for a device. No-op when nothing is running.
    ///
    /// `try_send` rather than `send` so this is safe to call from a window
    /// event handler; the task wakes on its next select tick regardless, and
    /// `kill_on_drop` covers the case where it never does.
    pub fn logcat_stop(&self, serial: &str) {
        let tx = self.logcat.0.lock().remove(serial);
        if let Some(tx) = tx {
            let _ = tx.try_send(());
        }
    }

    pub fn logcat_is_active(&self, serial: &str) -> bool {
        self.logcat.0.lock().contains_key(serial)
    }

    /// Serials with a live stream.
    pub fn logcat_active_serials(&self) -> Vec<String> {
        self.logcat.0.lock().keys().cloned().collect()
    }

    pub async fn logcat_query(&self, args: LogcatQueryArgs) -> CoreResult<Vec<LogcatRowDto>> {
        self.storage
            .query_logcat(
                &args.serial,
                args.filter.as_deref(),
                &args.include_pids,
                &args.exclude_pids,
                args.limit,
            )
            .map_err(to_api(kinds::DB))
    }

    pub async fn logcat_query_older(
        &self,
        args: LogcatQueryOlderArgs,
    ) -> CoreResult<Vec<LogcatRowDto>> {
        self.storage
            .query_logcat_before(
                &args.serial,
                args.filter.as_deref(),
                &args.include_pids,
                &args.exclude_pids,
                args.before_id,
                args.limit,
            )
            .map_err(to_api(kinds::DB))
    }

    pub async fn logcat_new_count(&self, args: LogcatNewCountArgs) -> CoreResult<i64> {
        self.storage
            .count_logcat_new(
                &args.serial,
                args.filter.as_deref(),
                &args.include_pids,
                &args.exclude_pids,
                args.after_id,
            )
            .map_err(to_api(kinds::DB))
    }

    pub async fn logcat_clear(&self, serial: &str) -> CoreResult<ClearResult> {
        let n = self
            .storage
            .clear_logcat(serial)
            .map_err(to_api(kinds::DB))?;
        Ok(ClearResult { deleted: n as u64 })
    }

    pub async fn logcat_export(&self, args: LogcatExportArgs) -> CoreResult<usize> {
        self.storage
            .export_logcat(
                &args.serial,
                args.filter.as_deref(),
                &args.include_pids,
                &args.exclude_pids,
                &args.path,
            )
            .map_err(to_api(kinds::IO))
    }

    /// PID → process-name snapshot, for labelling rows with the package they
    /// came from and for resolving `app:` filter terms to PIDs.
    pub async fn android_pid_names(&self, serial: &str) -> CoreResult<HashMap<u32, String>> {
        let android = AndroidPlatform::new();
        android.pid_names(serial).await.map_err(to_api(kinds::ADB))
    }
}
