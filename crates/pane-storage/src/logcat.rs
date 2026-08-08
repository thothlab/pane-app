//! Logcat persistence: batched inserts, filtered queries, retention.
//!
//! Fed by the `adb logcat` stream (see `src-tauri/src/commands/logcat.rs`),
//! which maps each `pane_android::LogEntry` into [`LogcatInsert`] and hands a
//! whole batch to [`Storage::insert_logcat_batch`] in one transaction. Queries
//! compile the same filter DSL the UI uses (see `logcat_filter_dsl`) to SQL.

use std::io::Write;

use anyhow::Result;
use pane_ipc::LogcatRowDto;
use rusqlite::functions::FunctionFlags;
use rusqlite::{params, Connection, ToSql, TransactionBehavior};
use time::OffsetDateTime;

use crate::Storage;

/// One logcat line ready to insert. Mapped from `pane_android::LogEntry` in the
/// command layer so pane-storage stays free of a pane-android dependency.
pub struct LogcatInsert {
    /// Raw device timestamp "MM-DD HH:MM:SS.mmm" (display only; has no year).
    pub device_ts: String,
    pub pid: u32,
    pub tid: u32,
    /// `LogLevel` discriminant 0..6 (verbose..silent).
    pub level: i64,
    pub tag: String,
    pub message: String,
}

impl Storage {
    /// Insert one batch of logcat lines in a single transaction. `created_at_ms`
    /// (ingest wall clock) is stamped once by the caller and shared across the
    /// batch — it drives retention and ties-break within the same rowid burst.
    /// Uses INSERT OR IGNORE so re-dumped lines are dropped; returns the count
    /// of *newly inserted* rows (excludes duplicates ignored by the dedup index).
    pub fn insert_logcat_batch(
        &self,
        serial: &str,
        created_at_ms: i64,
        rows: &[LogcatInsert],
    ) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut conn = self.logcat_write.lock();
        // IMMEDIATE, not the default DEFERRED. There are two writer
        // connections on this database now (this one and `conn`), and a
        // deferred transaction that reads before it writes gets an instant
        // SQLITE_BUSY on the upgrade — the busy_timeout never gets a chance to
        // wait it out. Taking the write lock at BEGIN keeps the timeout in
        // play, so a collision with a capture insert is a short wait rather
        // than a dropped batch.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // OR IGNORE against the logcat_dedup unique index (V011): when adb
        // re-dumps the device ring buffer (every window reopen / reconnect),
        // the replayed lines collide with what's already stored and are
        // dropped. `execute` returns rows-changed (0 on ignore), so summing
        // gives the count of *actually new* rows — the caller uses it to skip
        // the "rows appended" ping when a whole batch was a replay.
        let mut inserted = 0usize;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO logcat_entry
                     (serial, created_at, device_ts, pid, tid, level, tag, message)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for r in rows {
                inserted += stmt.execute(params![
                    serial,
                    created_at_ms,
                    r.device_ts,
                    r.pid,
                    r.tid,
                    r.level,
                    r.tag,
                    r.message,
                ])?;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Query the most recent `limit` entries for `serial` matching `filter`
    /// (DSL string) plus the frontend-resolved `app:` PID lists. Two-step
    /// ordering (newest N via DESC+LIMIT, then flipped to ASC) so the UI
    /// renders oldest-on-top like a terminal log. `limit` is clamped.
    pub fn query_logcat(
        &self,
        serial: &str,
        filter: Option<&str>,
        include_pids: &[u32],
        exclude_pids: &[u32],
        limit: u32,
    ) -> Result<Vec<LogcatRowDto>> {
        let limit = limit.min(5000) as i64;
        let (where_sql, mut params) = logcat_where(serial, filter, include_pids, exclude_pids)?;
        let sql = format!(
            "SELECT id, created_at, device_ts, pid, tid, level, tag, message
             FROM (
               SELECT * FROM logcat_entry
               WHERE {where_sql}
               ORDER BY id DESC LIMIT ?
             )
             ORDER BY id ASC"
        );
        params.push(Box::new(limit));
        let conn = self.logcat_read.lock();
        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), map_logcat_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Query the newest `limit` entries *older* than `before_id` for `serial`
    /// (matching the same filter + app: PID lists). Powers "load older on
    /// scroll-up": the frontend passes the id of its oldest loaded row and
    /// prepends the result. Same two-step DESC+LIMIT→ASC ordering as
    /// `query_logcat`; the only addition is the `id < ?` bound.
    pub fn query_logcat_before(
        &self,
        serial: &str,
        filter: Option<&str>,
        include_pids: &[u32],
        exclude_pids: &[u32],
        before_id: i64,
        limit: u32,
    ) -> Result<Vec<LogcatRowDto>> {
        let limit = limit.min(5000) as i64;
        let (where_sql, mut params) = logcat_where(serial, filter, include_pids, exclude_pids)?;
        let sql = format!(
            "SELECT id, created_at, device_ts, pid, tid, level, tag, message
             FROM (
               SELECT * FROM logcat_entry
               WHERE {where_sql} AND id < ?
               ORDER BY id DESC LIMIT ?
             )
             ORDER BY id ASC"
        );
        params.push(Box::new(before_id));
        params.push(Box::new(limit));
        let conn = self.logcat_read.lock();
        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), map_logcat_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Count matching entries newer than `after_id` — drives the "+N new"
    /// badge while the view is frozen (tail off).
    pub fn count_logcat_new(
        &self,
        serial: &str,
        filter: Option<&str>,
        include_pids: &[u32],
        exclude_pids: &[u32],
        after_id: i64,
    ) -> Result<i64> {
        let (where_sql, mut params) = logcat_where(serial, filter, include_pids, exclude_pids)?;
        let sql = format!("SELECT COUNT(*) FROM logcat_entry WHERE {where_sql} AND id > ?");
        params.push(Box::new(after_id));
        let conn = self.logcat_read.lock();
        let refs: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let n: i64 = conn.query_row(&sql, refs.as_slice(), |r| r.get(0))?;
        Ok(n)
    }

    /// Export the full (uncapped) filtered set for a device to `path` in
    /// threadtime format. Streams row-by-row so a day's worth never has to
    /// materialize in memory. Returns the line count written.
    pub fn export_logcat(
        &self,
        serial: &str,
        filter: Option<&str>,
        include_pids: &[u32],
        exclude_pids: &[u32],
        path: &str,
    ) -> Result<usize> {
        let (where_sql, params) = logcat_where(serial, filter, include_pids, exclude_pids)?;
        let sql = format!(
            "SELECT device_ts, pid, tid, level, tag, message
             FROM logcat_entry WHERE {where_sql} ORDER BY id ASC"
        );
        let conn = self.logcat_read.lock();
        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let mut rows = stmt.query(refs.as_slice())?;
        let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut count = 0usize;
        while let Some(r) = rows.next()? {
            let device_ts: String = r.get(0)?;
            let pid: i64 = r.get(1)?;
            let tid: i64 = r.get(2)?;
            let level: i64 = r.get(3)?;
            let tag: String = r.get(4)?;
            let message: String = r.get(5)?;
            writeln!(
                w,
                "{device_ts} {pid:>5} {tid:>5} {} {tag}: {message}",
                level_char(level)
            )?;
            count += 1;
        }
        w.flush()?;
        Ok(count)
    }

    /// Delete all logcat rows for one device (the Clear button).
    pub fn clear_logcat(&self, serial: &str) -> Result<usize> {
        let conn = self.logcat_write.lock();
        let n = conn.execute(
            "DELETE FROM logcat_entry WHERE serial = ?1",
            params![serial],
        )?;
        Ok(n)
    }

    /// Rows deleted per transaction by [`Storage::prune_logcat`].
    ///
    /// Retention used to run as one big transaction: a `DELETE` across the
    /// whole table plus, per device, a `DELETE ... id <= (SELECT ... OFFSET
    /// cap)` whose subquery walked `cap` index entries. At a 2M-row cap on a
    /// GB-sized database that held the write lock for many seconds every five
    /// minutes, and every UI command queued behind it. Chunking bounds how
    /// long the lock is held at a time; the loop still deletes everything it
    /// set out to, it just lets ingest and readers interleave.
    const PRUNE_CHUNK: usize = 10_000;

    /// Retention: drop rows older than `retention_ms` (by ingest time), then
    /// trim each device down to its newest `per_device_cap` rows. Returns rows
    /// deleted.
    ///
    /// Runs on the logcat write connection, in bounded chunks, releasing the
    /// lock between each one.
    pub fn prune_logcat(&self, retention_ms: i64, per_device_cap: i64) -> Result<usize> {
        let now_ms = (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        let cutoff = now_ms.saturating_sub(retention_ms);
        let mut deleted = 0usize;

        // Age-based sweep. `logcat_created_at` indexes the predicate, so each
        // chunk is an index range scan, not a table scan.
        loop {
            let n = {
                let conn = self.logcat_write.lock();
                conn.execute(
                    "DELETE FROM logcat_entry WHERE id IN (
                         SELECT id FROM logcat_entry WHERE created_at < ?1 LIMIT ?2
                     )",
                    params![cutoff, Self::PRUNE_CHUNK as i64],
                )?
            };
            deleted += n;
            if n < Self::PRUNE_CHUNK {
                break;
            }
        }

        let serials: Vec<String> = {
            let conn = self.logcat_write.lock();
            let mut stmt = conn.prepare("SELECT DISTINCT serial FROM logcat_entry")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        for s in serials {
            // How many rows this device is over its cap. Counted once: rows
            // ingested while we delete are newer than everything we are about
            // to drop, so they can't change which rows are the oldest N.
            //
            // `id` is a *global* rowid, not per-device, so the newest `cap`
            // rows for one serial are not "id > max_id - cap" — with two
            // devices streaming, their ids interleave and that would trim the
            // quieter one to far less than its cap. Counting per serial keeps
            // the cap meaning what it says. `logcat_serial_id` makes both the
            // count and the oldest-first read index-only.
            let mut excess: i64 = {
                let conn = self.logcat_write.lock();
                let n: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM logcat_entry WHERE serial = ?1",
                    params![s],
                    |r| r.get(0),
                )?;
                n - per_device_cap
            };
            while excess > 0 {
                let take = excess.min(Self::PRUNE_CHUNK as i64);
                let n = {
                    let conn = self.logcat_write.lock();
                    // ORDER BY id ASC LIMIT n walks the front of the
                    // (serial, id) index directly — no OFFSET scan.
                    conn.execute(
                        "DELETE FROM logcat_entry WHERE id IN (
                             SELECT id FROM logcat_entry
                             WHERE serial = ?1 ORDER BY id ASC LIMIT ?2
                         )",
                        params![s, take],
                    )?
                };
                deleted += n;
                if n == 0 {
                    break;
                }
                excess -= n as i64;
            }
        }

        // A firehose can outpace autocheckpoint, so the -wal file still needs
        // truncating occasionally — but TRUNCATE rewrites the whole WAL and
        // fsyncs, which is far too expensive to do on every pass. Only when it
        // has actually grown.
        if self.logcat_wal_bytes() > WAL_TRUNCATE_THRESHOLD_BYTES {
            let conn = self.logcat_write.lock();
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        }
        Ok(deleted)
    }

    fn logcat_wal_bytes(&self) -> u64 {
        std::fs::metadata(self.data_dir().join("captures.db-wal"))
            .map(|m| m.len())
            .unwrap_or(0)
    }
}

/// Truncate the WAL only once it passes this. Below it, SQLite's own
/// autocheckpoint keeps the file bounded for free.
const WAL_TRUNCATE_THRESHOLD_BYTES: u64 = 64 * 1024 * 1024;

/// Build `WHERE serial=? AND (<dsl>) [AND pid IN(..)] [AND pid NOT IN(..)]`
/// plus the ordered bind params (without the trailing LIMIT).
fn logcat_where(
    serial: &str,
    filter: Option<&str>,
    include_pids: &[u32],
    exclude_pids: &[u32],
) -> Result<(String, Vec<Box<dyn ToSql>>)> {
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    params.push(Box::new(serial.to_string()));
    parts.push("serial = ?".to_string());

    if let Some(f) = filter {
        if !f.trim().is_empty() {
            let (dsl_sql, dsl_params) = crate::logcat_filter_dsl::compile_to_sql(f)?;
            parts.push(format!("({dsl_sql})"));
            params.extend(dsl_params);
        }
    }
    if !include_pids.is_empty() {
        let qs = vec!["?"; include_pids.len()].join(", ");
        for p in include_pids {
            params.push(Box::new(*p as i64));
        }
        parts.push(format!("pid IN ({qs})"));
    }
    if !exclude_pids.is_empty() {
        let qs = vec!["?"; exclude_pids.len()].join(", ");
        for p in exclude_pids {
            params.push(Box::new(*p as i64));
        }
        parts.push(format!("pid NOT IN ({qs})"));
    }
    Ok((parts.join(" AND "), params))
}

fn map_logcat_row(r: &rusqlite::Row) -> rusqlite::Result<LogcatRowDto> {
    Ok(LogcatRowDto {
        id: r.get(0)?,
        created_at: r.get(1)?,
        timestamp: r.get(2)?,
        pid: r.get::<_, i64>(3)? as u32,
        tid: r.get::<_, i64>(4)? as u32,
        level: level_to_str(r.get::<_, i64>(5)?),
        tag: r.get(6)?,
        message: r.get(7)?,
    })
}

/// LogLevel discriminant → lowercase string. ⚠ Must stay in sync with
/// `pane_android::logcat::LogLevel` (pane-storage can't see that enum).
fn level_to_str(n: i64) -> String {
    match n {
        0 => "verbose",
        1 => "debug",
        2 => "info",
        3 => "warn",
        4 => "error",
        5 => "fatal",
        6 => "silent",
        _ => "info",
    }
    .to_string()
}

fn level_char(n: i64) -> char {
    match n {
        0 => 'V',
        1 => 'D',
        2 => 'I',
        3 => 'W',
        4 => 'E',
        5 => 'F',
        6 => 'S',
        _ => 'I',
    }
}

/// Register the `REGEXP` scalar function (used by `col REGEXP ?`). The compiled
/// `Regex` is cached per pattern via `get_or_create_aux` — the pattern is
/// constant within a query, so this compiles once, not per row.
pub(crate) fn register_regexp(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "regexp",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let regex = ctx.get_or_create_aux(
                0,
                |vr| -> std::result::Result<
                    regex::Regex,
                    Box<dyn std::error::Error + Send + Sync + 'static>,
                > { Ok(regex::Regex::new(vr.as_str()?)?) },
            )?;
            let text: String = ctx.get(1)?;
            Ok(regex.is_match(&text))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Storage;
    use tempfile::tempdir;

    fn ins(tag: &str, msg: &str, pid: u32, level: i64) -> LogcatInsert {
        LogcatInsert {
            device_ts: "07-09 12:00:00.000".into(),
            pid,
            tid: 1,
            level,
            tag: tag.into(),
            message: msg.into(),
        }
    }

    #[test]
    fn insert_query_filter_regex_and_prune_roundtrip() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        let rows = vec![
            ins("AnalyticsEvent", "Сбор open", 100, 2), // info
            ins("OkHttp", "GET /x", 100, 2),
            ins("AnalyticsEvent", "Сбор close", 200, 4), // error
        ];
        assert_eq!(s.insert_logcat_batch("SERIAL_A", 1_000, &rows).unwrap(), 3);
        // Another device — must stay isolated.
        s.insert_logcat_batch("SERIAL_B", 1_000, &[ins("AnalyticsEvent", "other", 9, 2)])
            .unwrap();

        // tag filter, scoped to device A.
        let got = s
            .query_logcat("SERIAL_A", Some("tag:AnalyticsEvent"), &[], &[], 100)
            .unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|r| r.tag == "AnalyticsEvent"));
        // Oldest-first ordering.
        assert!(got[0].id < got[1].id);
        assert_eq!(got[0].level, "info");
        assert_eq!(got[1].level, "error");

        // level range.
        let errs = s
            .query_logcat("SERIAL_A", Some("level:E"), &[], &[], 100)
            .unwrap();
        assert_eq!(errs.len(), 1);

        // regex over message (exercises the REGEXP fn on the read conn).
        let re = s
            .query_logcat("SERIAL_A", Some("~close$"), &[], &[], 100)
            .unwrap();
        assert_eq!(re.len(), 1);
        assert!(re[0].message.ends_with("close"));

        // app: resolved PIDs come in as include list.
        let by_pid = s.query_logcat("SERIAL_A", None, &[200], &[], 100).unwrap();
        assert_eq!(by_pid.len(), 1);
        assert_eq!(by_pid[0].pid, 200);

        // Device isolation.
        assert_eq!(
            s.query_logcat("SERIAL_B", None, &[], &[], 100)
                .unwrap()
                .len(),
            1
        );

        // Badge count: everything newer than id 0.
        assert_eq!(
            s.count_logcat_new("SERIAL_A", None, &[], &[], 0).unwrap(),
            3
        );

        // Retention: nothing older than "now - huge" ; then a tiny retention
        // window (created_at=1000 is far in the past) drops all of A+B.
        assert_eq!(s.prune_logcat(i64::MAX, i64::MAX).unwrap(), 0);
        let deleted = s.prune_logcat(0, i64::MAX).unwrap();
        assert_eq!(deleted, 4);
        assert_eq!(
            s.query_logcat("SERIAL_A", None, &[], &[], 100)
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn insert_or_ignore_drops_redumped_lines() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        let batch = vec![ins("T", "hello", 1, 2), ins("T", "world", 1, 2)];
        // First ingest: both rows are new.
        assert_eq!(s.insert_logcat_batch("DEV", 1_000, &batch).unwrap(), 2);
        // adb re-dumps the same buffer (reopen/reconnect): every line collides
        // with the dedup index → nothing new, so the caller skips the ping.
        assert_eq!(s.insert_logcat_batch("DEV", 2_000, &batch).unwrap(), 0);
        // A genuinely new line still lands.
        assert_eq!(
            s.insert_logcat_batch("DEV", 3_000, &[ins("T", "fresh", 1, 2)])
                .unwrap(),
            1
        );
        // The table holds exactly the 3 distinct lines, no duplicates.
        assert_eq!(s.query_logcat("DEV", None, &[], &[], 100).unwrap().len(), 3);
    }

    #[test]
    fn migration_v011_dedups_then_indexes() {
        // Reproduce a pre-V011 (duplicate-laden) table and run the REAL
        // migration SQL against it — a fresh Storage would already have the
        // unique index, so this is the only way to prove the migration's
        // delete-then-index ordering survives a DB that already has dupes.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../migrations/V010__logcat.sql"))
            .unwrap();
        let insert = "INSERT INTO logcat_entry
            (serial, created_at, device_ts, pid, tid, level, tag, message)
            VALUES ('DEV', 1, '07-09 12:00:00.000', 1, 1, 2, 'T', 'hello')";
        conn.execute(insert, []).unwrap(); // id 1
        conn.execute(insert, []).unwrap(); // id 2 — duplicate
        conn.execute(insert, []).unwrap(); // id 3 — duplicate
        conn.execute(
            "INSERT INTO logcat_entry
             (serial, created_at, device_ts, pid, tid, level, tag, message)
             VALUES ('DEV', 1, '07-09 12:00:00.000', 1, 1, 2, 'T', 'world')",
            [],
        )
        .unwrap(); // id 4 — distinct (message differs)

        // The unique index cannot be built while duplicates exist — this is
        // exactly the failure the migration's DELETE-first ordering prevents.
        assert!(conn
            .execute_batch(
                "CREATE UNIQUE INDEX probe ON logcat_entry
                 (serial, device_ts, pid, tid, level, tag, message)"
            )
            .is_err());

        // Apply V011: dedup then index.
        conn.execute_batch(include_str!("../migrations/V011__logcat_dedup.sql"))
            .unwrap();

        // Duplicates collapsed to the earliest ingested (MIN id = 1); the
        // distinct line survives.
        let ids: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT id FROM logcat_entry ORDER BY id")
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap()
        };
        assert_eq!(ids, vec![1, 4]);

        // The index now enforces dedup: a replayed line is ignored.
        let n = conn
            .execute(
                "INSERT OR IGNORE INTO logcat_entry
                 (serial, created_at, device_ts, pid, tid, level, tag, message)
                 VALUES ('DEV', 1, '07-09 12:00:00.000', 1, 1, 2, 'T', 'hello')",
                [],
            )
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn query_before_paginates_older() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        // 100 rows, ids 1..=100 (rowid is monotonic per insert order).
        let batch: Vec<LogcatInsert> = (0..100).map(|i| ins("T", &format!("m{i}"), 1, 2)).collect();
        s.insert_logcat_batch("DEV", 1_000, &batch).unwrap();

        // The 20 newest rows older than id 50 → ids 30..=49 (ASC).
        let page = s
            .query_logcat_before("DEV", None, &[], &[], 50, 20)
            .unwrap();
        assert_eq!(page.len(), 20);
        assert_eq!(page[0].id, 30);
        assert_eq!(page[19].id, 49);

        // Filter is honoured on the older page too (all rows match here).
        let filtered = s
            .query_logcat_before("DEV", Some("tag:T"), &[], &[], 50, 5)
            .unwrap();
        assert_eq!(filtered.len(), 5);
        assert_eq!(filtered[4].id, 49);

        // Reaching the start returns a short page (fewer than requested).
        let head = s.query_logcat_before("DEV", None, &[], &[], 5, 20).unwrap();
        assert_eq!(head.len(), 4); // ids 1..=4
        assert_eq!(head[0].id, 1);
    }

    #[test]
    fn per_device_cap_trims_oldest() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        let batch: Vec<LogcatInsert> = (0..10).map(|i| ins("T", &format!("m{i}"), 1, 2)).collect();
        s.insert_logcat_batch("DEV", 10_000_000_000_000, &batch)
            .unwrap();
        // Keep only the newest 3 (retention window huge so only the cap bites).
        s.prune_logcat(i64::MAX, 3).unwrap();
        let got = s.query_logcat("DEV", None, &[], &[], 100).unwrap();
        assert_eq!(got.len(), 3);
        // The survivors are the last three inserted.
        assert_eq!(got[0].message, "m7");
        assert_eq!(got[2].message, "m9");
    }

    /// The cap is applied per device, and `id` is a *global* rowid. Trimming
    /// by "id > max_id - cap" would charge one device for the other's rows, so
    /// the quiet device would end up under its cap. Interleave two serials and
    /// check both keep exactly `cap`.
    #[test]
    fn per_device_cap_is_not_charged_for_another_device() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        // Alternate serials so their rowids interleave.
        for i in 0..20 {
            s.insert_logcat_batch("A", 10_000_000_000_000, &[ins("T", &format!("a{i}"), 1, 2)])
                .unwrap();
            s.insert_logcat_batch("B", 10_000_000_000_000, &[ins("T", &format!("b{i}"), 1, 2)])
                .unwrap();
        }
        s.prune_logcat(i64::MAX, 5).unwrap();

        let a = s.query_logcat("A", None, &[], &[], 100).unwrap();
        let b = s.query_logcat("B", None, &[], &[], 100).unwrap();
        assert_eq!(a.len(), 5, "device A kept the wrong number of rows");
        assert_eq!(b.len(), 5, "device B kept the wrong number of rows");
        assert_eq!(a[0].message, "a15");
        assert_eq!(b[0].message, "b15");
    }

    /// Retention deletes in bounded chunks now, so an excess larger than one
    /// chunk has to keep going rather than stopping after the first pass.
    #[test]
    fn prune_loops_until_the_cap_is_reached_across_chunks() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        // Two and a half chunks over the cap.
        let total = Storage::PRUNE_CHUNK * 2 + Storage::PRUNE_CHUNK / 2 + 7;
        let batch: Vec<LogcatInsert> = (0..total)
            .map(|i| ins("T", &format!("m{i}"), 1, 2))
            .collect();
        s.insert_logcat_batch("DEV", 10_000_000_000_000, &batch)
            .unwrap();

        let deleted = s.prune_logcat(i64::MAX, 7).unwrap();
        assert_eq!(deleted, total - 7);
        let got = s.query_logcat("DEV", None, &[], &[], 100).unwrap();
        assert_eq!(got.len(), 7);
        assert_eq!(got[6].message, format!("m{}", total - 1));
    }

    /// The age sweep is chunked too — same requirement, different predicate.
    #[test]
    fn age_sweep_clears_more_than_one_chunk() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        let old = Storage::PRUNE_CHUNK + 500;
        let stale: Vec<LogcatInsert> = (0..old)
            .map(|i| ins("T", &format!("old{i}"), 1, 2))
            .collect();
        // Ingest-stamped in 1970; anything but an enormous window is "old".
        s.insert_logcat_batch("DEV", 1_000, &stale).unwrap();
        let fresh_at = (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        s.insert_logcat_batch("DEV", fresh_at, &[ins("T", "fresh", 1, 2)])
            .unwrap();

        // Retain one hour: every "old" row is past it, the fresh one is not.
        let deleted = s.prune_logcat(60 * 60 * 1000, i64::MAX).unwrap();
        assert_eq!(deleted, old);
        let got = s.query_logcat("DEV", None, &[], &[], 100).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].message, "fresh");
    }
}
