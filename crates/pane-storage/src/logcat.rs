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
use rusqlite::{params, Connection, ToSql};
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
    /// Returns the number of rows written.
    pub fn insert_logcat_batch(
        &self,
        serial: &str,
        created_at_ms: i64,
        rows: &[LogcatInsert],
    ) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO logcat_entry
                     (serial, created_at, device_ts, pid, tid, level, tag, message)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for r in rows {
                stmt.execute(params![
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
        Ok(rows.len())
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
        let conn = self.conn.lock();
        let n = conn.execute("DELETE FROM logcat_entry WHERE serial = ?1", params![serial])?;
        Ok(n)
    }

    /// Retention: drop rows older than `retention_ms` (by ingest time), then
    /// trim each device down to its newest `per_device_cap` rows. Runs on the
    /// main write connection. Returns rows deleted.
    pub fn prune_logcat(&self, retention_ms: i64, per_device_cap: i64) -> Result<usize> {
        let now_ms = (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        let cutoff = now_ms - retention_ms;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let mut deleted =
            tx.execute("DELETE FROM logcat_entry WHERE created_at < ?1", params![cutoff])?;
        let serials: Vec<String> = {
            let mut stmt = tx.prepare("SELECT DISTINCT serial FROM logcat_entry")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for s in serials {
            // The subquery is the id of the cap-th newest row; when a device
            // has fewer than `cap` rows it returns NULL and `id < NULL` is
            // never true — a correct no-op, no guard needed.
            deleted += tx.execute(
                "DELETE FROM logcat_entry WHERE serial = ?1 AND id < (
                     SELECT id FROM logcat_entry WHERE serial = ?1
                     ORDER BY id DESC LIMIT 1 OFFSET ?2
                 )",
                params![s, per_device_cap],
            )?;
        }
        tx.commit()?;
        // A firehose can outpace autocheckpoint; truncate the WAL so the
        // -wal file doesn't grow without bound.
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        Ok(deleted)
    }
}

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
