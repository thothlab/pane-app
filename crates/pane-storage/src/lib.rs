//! SQLite storage layer.
//!
//! All schema lives in `migrations/`; runtime types here mirror the PRD data
//! model. Writes go through `Storage`; bodies above 64 KB spill to files
//! addressed by sha256 (content-addressed dedup).

mod bodies;
mod filter_dsl;
mod logcat;
mod logcat_filter_dsl;
mod migrations;
mod replay_impl;

pub use bodies::BodyStore;
pub use logcat::LogcatInsert;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use pane_ipc::{
    CaptureBodyDto, CaptureDto, CollectionSetEnabledArgs, CollectionSetPriorityArgs,
    CollectionUpsertArgs, ExportOneResult, FilterDto, HeaderDto, ReplayRecordDto, ReplaySendArgs,
    RuleBulkScope, RuleCollectionDto, RuleConditionDto, RuleDto, RuleHeaderDto, RuleParamDto,
    RulePatchOpDto, RuleSetEnabledArgs, RuleSetPriorityArgs, RuleUpsertArgs,
    RulesSetEnabledBulkArgs, RulesSetEnabledBulkResult, SaveFilterArgs, SessionDto, TlsHealthDto,
};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleMode {
    Stub,
    Patch,
}

/// One mutation applied in `RuleMode::Patch`. Path uses dot-notation over a
/// virtual response tree: `status`, `headers.<name>`, `body.<dot.path>`.
#[derive(Debug, Clone)]
pub enum PatchOp {
    Set {
        path: String,
        value: serde_json::Value,
    },
    Delete {
        path: String,
    },
    Append {
        path: String,
        value: serde_json::Value,
    },
}

/// Comparison operator for a request-body condition. Parsed once at load
/// time from the stored string; unknown operators are dropped (the condition
/// is skipped rather than failing the whole rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
}

impl ConditionOp {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "eq" => Self::Eq,
            "ne" => Self::Ne,
            "gt" => Self::Gt,
            "gte" => Self::Gte,
            "lt" => Self::Lt,
            "lte" => Self::Lte,
            "contains" => Self::Contains,
            _ => return None,
        })
    }
}

/// Engine-side predicate on a request-body field. `path` is a dot/bracket
/// path into the JSON body; `value` is the right-hand side, kept as a string
/// (numeric ops coerce). All conditions on a rule are AND-ed.
#[derive(Debug, Clone)]
pub struct RuleCondition {
    pub path: String,
    pub op: ConditionOp,
    pub value: String,
}

/// Engine-side view of an active rule. Bodies materialized once at load time
/// so the proxy_loop can match + serve without re-querying the DB.
#[derive(Debug, Clone)]
pub struct ActiveRule {
    pub id: Uuid,
    pub name: String,
    pub priority: i64,
    pub mode: RuleMode,
    pub patches: Vec<PatchOp>,
    pub host_glob: Option<String>,
    pub method: Option<String>,
    pub path_glob: Option<String>,
    /// name=value pairs matched against either query string OR top-level JSON
    /// body of the request, depending on which side has the field.
    pub params: Vec<(String, String)>,
    /// Parsed JSON template the engine deep-subset-matches against the
    /// request body (nested-capable). `None` = don't match on the body.
    pub req_body_match: Option<serde_json::Value>,
    /// Predicate conditions on request-body fields (comparison / substring),
    /// AND-ed together and with the other matchers. Empty = no conditions.
    pub conditions: Vec<RuleCondition>,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub body_mime: Option<String>,
    pub delay_ms: u64,
}

pub struct CaRecord {
    pub id: Uuid,
    pub pem: String,
    pub sha256_fp: String,
    pub subject: String,
    pub valid_from: OffsetDateTime,
    pub valid_to: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
}

impl CaRecord {
    pub fn into_dto(self) -> pane_ipc::CaCertificateDto {
        pane_ipc::CaCertificateDto {
            id: self.id,
            serial: self.sha256_fp.chars().take(16).collect(),
            sha256_fp: self.sha256_fp,
            subject: self.subject,
            valid_from: self.valid_from.to_string(),
            valid_to: self.valid_to.to_string(),
            revoked_at: self.revoked_at.map(|t| t.to_string()),
        }
    }
}

/// Parse-check a captures filter DSL string without needing a `Storage`.
///
/// The CLI validates `--filter` locally before opening a session, so a typo
/// fails immediately rather than after a round trip — and, for `tail`, before
/// the caller has already launched the app under test.
pub fn validate_filter(filter: &str) -> Result<()> {
    filter_dsl::compile_to_sql(filter).map(|_| ())
}

/// Format a unix timestamp as RFC 3339.
///
/// `OffsetDateTime::to_string()` yields `2026-08-05 12:31:04.812 +00:00:00`,
/// which is not RFC 3339 and is rejected by `jq`'s `fromdate`, `date -d`, and
/// every other tool a caller reaches for. Anything that ships a timestamp
/// across the wire goes through here instead.
fn rfc3339(unix_seconds: i64) -> String {
    OffsetDateTime::from_unix_timestamp(unix_seconds)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| unix_seconds.to_string())
}

/// Open the logcat write connection. Same pragmas as the main writer — WAL is
/// a database-level property, but `synchronous` and `busy_timeout` are
/// per-connection and have to be set again here.
fn open_logcat_write(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

pub struct Storage {
    conn: Mutex<Connection>,
    /// Dedicated connection reserved for logcat SELECTs. WAL lets it read
    /// concurrently with the main write connection, so the firehose ingest
    /// doesn't serialize against the UI's filtered queries. The REGEXP scalar
    /// function is registered only here.
    logcat_read: Mutex<Connection>,
    /// Dedicated connection for logcat INSERTs and retention deletes.
    ///
    /// These are the highest-volume writes in the process by two orders of
    /// magnitude — a chatty emulator sustains ~80 rows/s, and retention then
    /// deletes them in bulk. While they shared `conn` with everything else,
    /// that single process-wide `Mutex` — not SQLite — was what stalled the
    /// UI: `captures_get`, `get_body` and the list poll all queued behind a
    /// batch insert or a prune. WAL already lets readers run against a
    /// separate connection while a writer commits, so splitting the mutex is
    /// enough to decouple them. Writer-vs-writer (proxy captures vs logcat)
    /// still serializes, but inside SQLite via `busy_timeout`, which waits its
    /// turn per statement instead of holding the whole process hostage.
    logcat_write: Mutex<Connection>,
    data_dir: PathBuf,
    pub bodies: Arc<BodyStore>,
}

impl Storage {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("captures.db");
        let mut conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // Same reasoning as `logcat_read` below, but this connection needs it
        // more: once the CLI can write to the same database from a second
        // process, a concurrent writer without a busy_timeout gets an instant
        // SQLITE_BUSY instead of waiting its turn. WAL already makes concurrent
        // *readers* safe; this covers the writer side.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        migrations::runner()
            .run(&mut conn)
            .context("migrations failed")?;

        // Second connection for logcat reads (schema now exists). Give it a
        // busy_timeout so a checkpoint/write burst yields a wait, not an error.
        let logcat_read = Connection::open(&db_path)?;
        logcat_read.busy_timeout(std::time::Duration::from_secs(5))?;
        logcat::register_regexp(&logcat_read)?;

        let logcat_write = open_logcat_write(&db_path)?;

        let bodies = Arc::new(BodyStore::new(data_dir.join("bodies"))?);
        Ok(Self {
            conn: Mutex::new(conn),
            logcat_read: Mutex::new(logcat_read),
            logcat_write: Mutex::new(logcat_write),
            data_dir: data_dir.to_path_buf(),
            bodies,
        })
    }

    /// Open a database this process does **not** own, without migrating it.
    ///
    /// `open` runs migrations, which is right for the process that owns the
    /// data directory and catastrophic for one that does not: a refinery
    /// upgrade is one-way, and the app that *does* own it aborts on startup
    /// the moment it finds a migration version it has never heard of. A CLI
    /// built from a newer checkout would therefore brick the installed app
    /// just by listing captures.
    ///
    /// So: refuse on any schema mismatch and say which direction it is in,
    /// rather than "helpfully" converging the schema.
    pub fn open_unowned(data_dir: &Path) -> Result<Self> {
        let db_path = data_dir.join("captures.db");
        if !db_path.exists() {
            return Err(anyhow!(
                "no Pane database at {} — start Pane (or `pane proxy run`) once to create it",
                db_path.display()
            ));
        }
        let conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        let db_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM refinery_schema_history",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let ours = migrations::latest_version();

        match db_version.cmp(&ours) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => {
                return Err(anyhow!(
                    "this database is at schema v{db_version}, newer than this build understands \
                     (v{ours}) — update the pane CLI"
                ))
            }
            std::cmp::Ordering::Less => {
                return Err(anyhow!(
                    "this database is at schema v{db_version} and this build expects v{ours}. \
                     Migrating it would stop the installed Pane app from launching, so it is \
                     left alone. Update the Pane app to match, or point the CLI somewhere else \
                     with --data-dir / PANE_DATA_DIR."
                ))
            }
        }

        let logcat_read = Connection::open(&db_path)?;
        logcat_read.busy_timeout(std::time::Duration::from_secs(5))?;
        logcat::register_regexp(&logcat_read)?;

        let logcat_write = open_logcat_write(&db_path)?;

        let bodies = Arc::new(BodyStore::new(data_dir.join("bodies"))?);
        Ok(Self {
            conn: Mutex::new(conn),
            logcat_read: Mutex::new(logcat_read),
            logcat_write: Mutex::new(logcat_write),
            data_dir: data_dir.to_path_buf(),
            bodies,
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    // ---------- CA ----------

    pub fn insert_ca(
        &self,
        id: Uuid,
        pem: &str,
        sha: &str,
        subject: &str,
        nb: OffsetDateTime,
        na: OffsetDateTime,
    ) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO ca_certificate (id, serial, sha256_fp, subject, valid_from, valid_to, pem)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id.to_string(),
                &sha[..16.min(sha.len())],
                sha,
                subject,
                nb.unix_timestamp(),
                na.unix_timestamp(),
                pem
            ],
        )?;
        Ok(())
    }

    pub fn revoke_ca(&self, id: Uuid) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE ca_certificate SET revoked_at=?1 WHERE id=?2",
            params![OffsetDateTime::now_utc().unix_timestamp(), id.to_string()],
        )?;
        Ok(())
    }

    pub fn current_ca_record(&self) -> Result<Option<CaRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, pem, sha256_fp, subject, valid_from, valid_to, revoked_at
             FROM ca_certificate
             WHERE revoked_at IS NULL
             ORDER BY valid_from DESC LIMIT 1",
        )?;
        let row = stmt
            .query_row([], |r| {
                Ok(CaRecord {
                    id: Uuid::parse_str(&r.get::<_, String>(0)?).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                    pem: r.get(1)?,
                    sha256_fp: r.get(2)?,
                    subject: r.get(3)?,
                    valid_from: OffsetDateTime::from_unix_timestamp(r.get(4)?).unwrap(),
                    valid_to: OffsetDateTime::from_unix_timestamp(r.get(5)?).unwrap(),
                    revoked_at: r
                        .get::<_, Option<i64>>(6)?
                        .map(|t| OffsetDateTime::from_unix_timestamp(t).unwrap()),
                })
            })
            .optional()?;
        Ok(row)
    }

    // ---------- Sessions ----------

    pub fn session_record(&self, listen: std::net::SocketAddr) -> Result<SessionDto> {
        let conn = self.conn.lock();
        let ca_id: String = conn.query_row(
            "SELECT id FROM ca_certificate WHERE revoked_at IS NULL ORDER BY valid_from DESC LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        let id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc();
        conn.execute(
            "INSERT INTO session (id, started_at, listen, ca_id, status)
             VALUES (?1, ?2, ?3, ?4, 'running')",
            params![
                id.to_string(),
                now.unix_timestamp(),
                listen.to_string(),
                ca_id
            ],
        )?;
        Ok(SessionDto {
            id,
            started_at: rfc3339(now.unix_timestamp()),
            listen: listen.to_string(),
            status: "running".into(),
            ca_id: Uuid::parse_str(&ca_id)?,
        })
    }

    pub fn current_session_id(&self) -> Result<Option<Uuid>> {
        let conn = self.conn.lock();
        let id: Option<String> = conn
            .query_row(
                "SELECT id FROM session WHERE stopped_at IS NULL ORDER BY started_at DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id.and_then(|s| Uuid::parse_str(&s).ok()))
    }

    /// Is the device trusting our CA at all, in the session running right now?
    ///
    /// Some hosts tunnelling is normal — release builds and pinned apps have
    /// always done that. The state worth warning about is *nothing* decrypting
    /// while several hosts tunnel, which is what a CA that was never installed
    /// (or was installed on a different machine, since each Pane install has
    /// its own root) looks like from here.
    ///
    /// Both halves are scoped to the current session, and the decrypted count
    /// is scoped to `https` — plain-HTTP captures need no trust at all, so
    /// counting them would mask exactly the case this is meant to catch.
    pub fn tls_health(&self) -> Result<TlsHealthDto> {
        let Some(session_id) = self.current_session_id()? else {
            return Ok(TlsHealthDto {
                tunneled_hosts: 0,
                decrypted_https: 0,
            });
        };
        let conn = self.conn.lock();
        let sid = session_id.to_string();
        let tunneled_hosts: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT server_host) FROM capture
             WHERE session_id=?1 AND error_kind='tunneled'",
            params![sid],
            |r| r.get(0),
        )?;
        let decrypted_https: i64 = conn.query_row(
            "SELECT COUNT(*) FROM capture
             WHERE session_id=?1 AND scheme='https' AND error_kind IS NULL
               AND state IN ('completed', 'patched', 'stubbed')",
            params![sid],
            |r| r.get(0),
        )?;
        Ok(TlsHealthDto {
            tunneled_hosts: tunneled_hosts.max(0) as u32,
            decrypted_https: decrypted_https.max(0) as u32,
        })
    }

    // ---------- Captures ----------

    /// Does capture `id` match `filter`?
    ///
    /// `compile_to_sql` produces a SQL WHERE fragment, so a filter cannot be
    /// evaluated in memory against an event payload. Re-asking the database
    /// about one row is how `captures tail --filter` reuses the exact filter
    /// semantics of the Captures view for free, at the cost of one indexed
    /// lookup per completed request.
    pub fn capture_matches(&self, id: Uuid, filter: &str) -> Result<bool> {
        if filter.trim().is_empty() {
            return Ok(true);
        }
        let (where_sql, params_vec) = filter_dsl::compile_to_sql(filter)?;
        let conn = self.conn.lock();
        let sql = format!("SELECT 1 FROM capture WHERE id = ? AND ({where_sql}) LIMIT 1");
        let mut stmt = conn.prepare(&sql)?;
        let id_str = id.to_string();
        let param_refs: Vec<&dyn rusqlite::ToSql> =
            std::iter::once(&id_str as &dyn rusqlite::ToSql)
                .chain(params_vec.iter().map(|b| b.as_ref()))
                .collect();
        let found = stmt
            .query_row(param_refs.as_slice(), |_| Ok(()))
            .optional()?
            .is_some();
        Ok(found)
    }

    /// Parse-check a captures filter DSL string without running a query.
    ///
    /// Lets callers report a malformed filter as its own error kind instead
    /// of folding it into a generic database error — `list_captures` returns
    /// a single `anyhow::Error` for both, and the CLI maps error kinds onto
    /// exit codes, so the two need telling apart.
    pub fn validate_capture_filter(&self, filter: &str) -> Result<()> {
        validate_filter(filter)
    }

    pub fn captures_count(&self) -> Result<i64> {
        let conn = self.conn.lock();
        Ok(conn.query_row("SELECT COUNT(*) FROM capture", [], |r| r.get(0))?)
    }

    pub fn list_captures(
        &self,
        filter: Option<&str>,
        limit: u32,
        _before: Option<String>,
    ) -> Result<Vec<CaptureDto>> {
        let limit = limit.min(2000) as i64;
        let conn = self.conn.lock();

        let (where_sql, params_vec): (String, Vec<Box<dyn rusqlite::ToSql>>) = match filter {
            Some(q) if !q.trim().is_empty() => filter_dsl::compile_to_sql(q)?,
            _ => ("1=1".into(), Vec::new()),
        };

        // Two-step ordering: take the newest N captures (DESC + LIMIT), then
        // flip to ASC so the UI renders oldest-on-top, newest-at-bottom —
        // terminal-log feel. Using LIMIT directly with ASC would return the
        // OLDEST N rows, not the most recent ones.
        let sql = format!(
            "SELECT id, session_id, started_at, ended_at, client_addr, server_host, server_port,
                    scheme, http_version, method, url_path, status, req_body_id, res_body_id,
                    total_bytes, duration_ms, state, error_kind, error_detail, device_id,
                    matched_rule_id, matched_rule_name
             FROM (
               SELECT * FROM capture
               WHERE {where_sql}
               ORDER BY started_at DESC LIMIT ?
             )
             ORDER BY started_at ASC, id ASC"
        );

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec
            .iter()
            .map(|b| b.as_ref())
            .chain(std::iter::once(&limit as &dyn rusqlite::ToSql))
            .collect();
        let rows = stmt.query_map(param_refs.as_slice(), Self::map_capture_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_capture(&self, id: Uuid) -> Result<CaptureDto> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, started_at, ended_at, client_addr, server_host, server_port,
                    scheme, http_version, method, url_path, status, req_body_id, res_body_id,
                    total_bytes, duration_ms, state, error_kind, error_detail, device_id,
                    matched_rule_id, matched_rule_name
             FROM capture WHERE id=?1",
        )?;
        let mut cap = stmt.query_row(params![id.to_string()], Self::map_capture_row)?;

        let mut h_stmt = conn.prepare(
            "SELECT name, value, direction FROM header WHERE capture_id=?1 ORDER BY order_idx",
        )?;
        let mut req = Vec::new();
        let mut res = Vec::new();
        let rows = h_stmt.query_map(params![id.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (name, value, dir) = row?;
            let h = HeaderDto { name, value };
            if dir == "request" {
                req.push(h);
            } else {
                res.push(h);
            }
        }
        cap.req_headers = Some(req);
        cap.res_headers = Some(res);
        Ok(cap)
    }

    fn map_capture_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CaptureDto> {
        let id: String = r.get(0)?;
        let session_id: String = r.get(1)?;
        let started_at: i64 = r.get(2)?;
        let ended_at: Option<i64> = r.get(3)?;
        let req_body_id: Option<String> = r.get(12)?;
        let res_body_id: Option<String> = r.get(13)?;
        Ok(CaptureDto {
            id: Uuid::parse_str(&id).unwrap(),
            session_id: Uuid::parse_str(&session_id).unwrap(),
            started_at: rfc3339(started_at),
            ended_at: ended_at.map(rfc3339),
            client_addr: r.get(4)?,
            server_host: r.get(5)?,
            server_port: r.get::<_, i64>(6)? as u16,
            scheme: r.get(7)?,
            http_version: r.get(8)?,
            method: r.get(9)?,
            url_path: r.get(10)?,
            status: r.get::<_, Option<i64>>(11)?.map(|v| v as u16),
            req_body_id: req_body_id.and_then(|s| Uuid::parse_str(&s).ok()),
            res_body_id: res_body_id.and_then(|s| Uuid::parse_str(&s).ok()),
            total_bytes: r.get::<_, i64>(14)? as u64,
            duration_ms: r.get::<_, Option<i64>>(15)?.map(|v| v as u64),
            state: r.get(16)?,
            error_kind: r.get(17)?,
            error_detail: r.get(18)?,
            device_id: r.get(19)?,
            matched_rule_id: r.get(20)?,
            matched_rule_name: r.get(21)?,
            req_headers: None,
            res_headers: None,
        })
    }

    pub fn get_body(&self, body_id: Uuid, max_bytes: Option<u64>) -> Result<CaptureBodyDto> {
        self.bodies.get(body_id, max_bytes, &self.conn)
    }

    pub fn clear_captures(&self, _older_than: Option<String>) -> Result<usize> {
        let conn = self.conn.lock();
        // `replay_record.source/result_capture_id` reference `capture(id)`
        // without an ON DELETE rule (the schema treats replay history as
        // narrative-standalone). With foreign_keys=ON enforced at startup,
        // `DELETE FROM capture` errors out as soon as any replay row points
        // at a soon-to-be-deleted capture. Detach those pointers first.
        conn.execute(
            "UPDATE replay_record SET source_capture_id=NULL WHERE source_capture_id IS NOT NULL",
            [],
        )?;
        conn.execute(
            "UPDATE replay_record SET result_capture_id=NULL WHERE result_capture_id IS NOT NULL",
            [],
        )?;
        let n = conn.execute("DELETE FROM capture", [])?;
        // Body GC: after wiping captures, no row references any blob row
        // anymore. Drop the orphans so the bodies/ folder doesn't grow
        // unboundedly across Clear cycles.
        //
        // `rule.res_body_id` (V002) also references capture_body — when a
        // user creates a stub from an existing response, the rule reuses
        // the same body blob to avoid duplicating bytes. So body GC has
        // to exclude rule-held bodies too, otherwise Clear hits a FOREIGN
        // KEY violation as soon as any stub-from-response rule exists.
        conn.execute(
            "DELETE FROM capture_body
              WHERE id NOT IN (SELECT req_body_id FROM capture WHERE req_body_id IS NOT NULL
                               UNION
                               SELECT res_body_id FROM capture WHERE res_body_id IS NOT NULL
                               UNION
                               SELECT res_body_id FROM rule WHERE res_body_id IS NOT NULL)",
            [],
        )?;
        Ok(n)
    }

    pub fn export_one(&self, id: Uuid, format: &str) -> Result<ExportOneResult> {
        let cap = self.get_capture(id)?;
        match format {
            "curl" => {
                let mut s = format!(
                    "curl -X {} '{}://{}:{}{}'",
                    cap.method, cap.scheme, cap.server_host, cap.server_port, cap.url_path
                );
                if let Some(hs) = &cap.req_headers {
                    for h in hs {
                        let v = h.value.replace('\'', "'\\''");
                        s.push_str(&format!(" -H '{}: {}'", h.name, v));
                    }
                }
                Ok(ExportOneResult {
                    text: s,
                    mime: "text/plain".into(),
                })
            }
            "har_single" => {
                let har = serde_json::json!({
                    "log": { "version": "1.2", "creator": {"name": "Pane", "version": env!("CARGO_PKG_VERSION")},
                        "entries": [ {
                            "startedDateTime": cap.started_at,
                            "time": cap.duration_ms.unwrap_or(0),
                            "request": {
                                "method": cap.method,
                                "url": format!("{}://{}:{}{}", cap.scheme, cap.server_host, cap.server_port, cap.url_path),
                                "httpVersion": cap.http_version,
                                "headers": cap.req_headers.unwrap_or_default(),
                                "queryString": [], "cookies": [], "headersSize": -1, "bodySize": -1
                            },
                            "response": {
                                "status": cap.status.unwrap_or(0),
                                "statusText": "",
                                "httpVersion": cap.http_version,
                                "headers": cap.res_headers.unwrap_or_default(),
                                "cookies": [], "content": {"size": 0, "mimeType": ""},
                                "redirectURL": "", "headersSize": -1, "bodySize": -1
                            },
                            "cache": {}, "timings": {"send": 0, "wait": 0, "receive": 0}
                        }]
                    }
                });
                Ok(ExportOneResult {
                    text: serde_json::to_string_pretty(&har)?,
                    mime: "application/json".into(),
                })
            }
            other => Err(anyhow!("unsupported format: {other}")),
        }
    }

    // ---------- Replay ----------

    pub async fn replay_send(&self, args: ReplaySendArgs) -> Result<ReplayRecordDto> {
        replay_impl::send(self, args).await
    }

    pub(crate) fn insert_replay_record(
        &self,
        source: Option<Uuid>,
        result: Option<Uuid>,
    ) -> Result<ReplayRecordDto> {
        let id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO replay_record (id, source_capture_id, result_capture_id, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                id.to_string(),
                source.map(|u| u.to_string()),
                result.map(|u| u.to_string()),
                now.unix_timestamp()
            ],
        )?;
        Ok(ReplayRecordDto {
            id,
            source_capture_id: source,
            result_capture_id: result,
            created_at: now.to_string(),
        })
    }

    // ---------- Filters ----------

    pub fn save_filter(&self, args: SaveFilterArgs) -> Result<FilterDto> {
        let conn = self.conn.lock();
        // Resolve the target row id. An explicit id (the UI detected an
        // update) wins. Otherwise reuse any existing filter with the same
        // (name, kind) so re-saving under an existing name overwrites it
        // instead of creating a duplicate — the UI's name-collision check
        // can miss when its filter list hasn't finished loading yet, so the
        // dedup is enforced here at the data layer regardless of timing.
        let id = match args.id {
            Some(id) => id,
            None => conn
                .query_row(
                    "SELECT id FROM saved_filter
                     WHERE lower(trim(name)) = lower(trim(?1)) AND kind = ?2",
                    params![&args.name, &args.kind],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .and_then(|s| Uuid::parse_str(&s).ok())
                .unwrap_or_else(Uuid::new_v4),
        };
        conn.execute(
            "INSERT INTO saved_filter (id, name, query, color, pinned, kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, query=excluded.query,
                color=excluded.color, pinned=excluded.pinned, kind=excluded.kind",
            params![
                id.to_string(),
                &args.name,
                &args.query,
                &args.color,
                args.pinned as i64,
                &args.kind,
            ],
        )?;
        Ok(FilterDto {
            id,
            name: args.name,
            query: args.query,
            color: args.color,
            pinned: args.pinned,
            kind: args.kind,
        })
    }

    /// Lists saved filters, optionally restricted to a single `kind`
    /// ("captures" / "logcat"). Pass `None` to list every kind — only
    /// useful for tooling/debug; production callers always pass the
    /// view's own kind so the two scopes don't bleed into each other.
    pub fn list_filters(&self, kind: Option<&str>) -> Result<Vec<FilterDto>> {
        let conn = self.conn.lock();
        let (sql, with_kind) = match kind {
            Some(_) => (
                "SELECT id, name, query, color, pinned, kind FROM saved_filter
                 WHERE kind = ?1 ORDER BY pinned DESC, name",
                true,
            ),
            None => (
                "SELECT id, name, query, color, pinned, kind FROM saved_filter
                 ORDER BY pinned DESC, name",
                false,
            ),
        };
        let mut stmt = conn.prepare(sql)?;
        let map_row = |r: &rusqlite::Row| {
            Ok(FilterDto {
                id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap(),
                name: r.get(1)?,
                query: r.get(2)?,
                color: r.get(3)?,
                pinned: r.get::<_, i64>(4)? != 0,
                kind: r.get(5)?,
            })
        };
        let rows: Vec<FilterDto> = if with_kind {
            stmt.query_map(params![kind.unwrap()], map_row)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([], map_row)?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }

    pub fn delete_filter(&self, id: Uuid) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM saved_filter WHERE id=?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    // ---------- Rule collections ----------

    pub fn list_collections(&self) -> Result<Vec<RuleCollectionDto>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.name, c.enabled, c.priority, c.created_at, c.updated_at,
                    (SELECT COUNT(*) FROM rule r WHERE r.collection_id = c.id)
             FROM rule_collection c
             ORDER BY c.priority ASC, c.created_at ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(RuleCollectionDto {
                id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap(),
                name: r.get(1)?,
                enabled: r.get::<_, i64>(2)? != 0,
                priority: r.get(3)?,
                created_at: r.get::<_, i64>(4)?.to_string(),
                updated_at: r.get::<_, i64>(5)?.to_string(),
                rule_count: r.get::<_, i64>(6)? as u64,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn upsert_collection(&self, args: CollectionUpsertArgs) -> Result<RuleCollectionDto> {
        let id = args.id.unwrap_or_else(Uuid::new_v4);
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let conn = self.conn.lock();
        let existing_created: Option<i64> = conn
            .query_row(
                "SELECT created_at FROM rule_collection WHERE id=?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let created_at = existing_created.unwrap_or(now);
        conn.execute(
            "INSERT INTO rule_collection (id, name, enabled, priority, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name, enabled=excluded.enabled,
                priority=excluded.priority, updated_at=excluded.updated_at",
            params![
                id.to_string(),
                &args.name,
                args.enabled as i64,
                args.priority,
                created_at,
                now,
            ],
        )?;
        drop(conn);
        self.get_collection(id)
    }

    pub fn get_collection(&self, id: Uuid) -> Result<RuleCollectionDto> {
        let conn = self.conn.lock();
        let dto = conn.query_row(
            "SELECT c.id, c.name, c.enabled, c.priority, c.created_at, c.updated_at,
                    (SELECT COUNT(*) FROM rule r WHERE r.collection_id = c.id)
             FROM rule_collection c WHERE c.id=?1",
            params![id.to_string()],
            |r| {
                Ok(RuleCollectionDto {
                    id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap(),
                    name: r.get(1)?,
                    enabled: r.get::<_, i64>(2)? != 0,
                    priority: r.get(3)?,
                    created_at: r.get::<_, i64>(4)?.to_string(),
                    updated_at: r.get::<_, i64>(5)?.to_string(),
                    rule_count: r.get::<_, i64>(6)? as u64,
                })
            },
        )?;
        Ok(dto)
    }

    /// Delete a collection, moving its rules to Ungrouped rather than taking
    /// them with it.
    ///
    /// The two statements run in one transaction. Separately, a crash between
    /// them left the rules detached with the collection row still standing —
    /// a group that looks intact in the list but reports zero rules, and whose
    /// members have quietly scattered into Ungrouped.
    pub fn delete_collection(&self, id: Uuid) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        // Detach rules first (so they end up in Ungrouped instead of tripping
        // the foreign key, which has no ON DELETE clause).
        tx.execute(
            "UPDATE rule SET collection_id = NULL WHERE collection_id=?1",
            params![id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM rule_collection WHERE id=?1",
            params![id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_collection_enabled(&self, args: CollectionSetEnabledArgs) -> Result<()> {
        let conn = self.conn.lock();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        conn.execute(
            "UPDATE rule_collection SET enabled=?1, updated_at=?2 WHERE id=?3",
            params![args.enabled as i64, now, args.id.to_string()],
        )?;
        Ok(())
    }

    pub fn set_collection_priority(&self, args: CollectionSetPriorityArgs) -> Result<()> {
        let conn = self.conn.lock();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        conn.execute(
            "UPDATE rule_collection SET priority=?1, updated_at=?2 WHERE id=?3",
            params![args.priority, now, args.id.to_string()],
        )?;
        Ok(())
    }

    // ---------- Rules (response stubbing) ----------

    pub fn list_rules(&self) -> Result<Vec<RuleDto>> {
        let mut dtos = {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare(
                "SELECT id, name, enabled, priority,
                        match_host_glob, match_method, match_path_glob, match_query,
                        res_status, res_headers, res_body_id, res_delay_ms,
                        created_at, updated_at, collection_id, mode, patches,
                        match_req_body, match_conditions
                 FROM rule
                 ORDER BY priority ASC, created_at ASC",
            )?;
            let rows = stmt.query_map([], Self::map_rule_row)?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };
        for dto in dtos.iter_mut() {
            if let Some(bid) = dto.res_body_id {
                let (mime, bytes) = self
                    .bodies
                    .get_raw(bid, &self.conn)
                    .unwrap_or((None, vec![]));
                dto.res_body_mime = mime;
                dto.res_body_size = bytes.len() as u64;
            }
        }
        Ok(dtos)
    }

    pub fn get_rule(&self, id: Uuid) -> Result<RuleDto> {
        let conn = self.conn.lock();
        let mut dto = conn.query_row(
            "SELECT id, name, enabled, priority,
                    match_host_glob, match_method, match_path_glob, match_query,
                    res_status, res_headers, res_body_id, res_delay_ms,
                    created_at, updated_at, collection_id, mode, patches,
                    match_req_body, match_conditions
             FROM rule WHERE id=?1",
            params![id.to_string()],
            Self::map_rule_row,
        )?;
        drop(conn);
        if let Some(bid) = dto.res_body_id {
            let (mime, bytes) = self
                .bodies
                .get_raw(bid, &self.conn)
                .unwrap_or((None, vec![]));
            dto.res_body_mime = mime;
            dto.res_body_size = bytes.len() as u64;
        }
        Ok(dto)
    }

    pub fn upsert_rule(&self, args: RuleUpsertArgs) -> Result<RuleDto> {
        // Resolve the body: inline base64 wins only if body_id is absent.
        let body_id = match (args.res_body_id, args.res_body_base64.as_deref()) {
            (Some(id), _) => Some(id),
            (None, Some(b64)) if !b64.is_empty() => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .map_err(|e| anyhow!("invalid res_body_base64: {e}"))?;
                let id = self.bodies.put(
                    &bytes,
                    "identity",
                    args.res_body_mime.as_deref(),
                    &self.conn,
                )?;
                Some(id)
            }
            _ => None,
        };

        let id = args.id.unwrap_or_else(Uuid::new_v4);
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let match_params = serde_json::to_string(&args.match_params)?;
        // Blank/whitespace → NULL so "no body matching" is one canonical
        // state (the matcher and the editor both treat NULL as "skip").
        let match_req_body = args
            .match_req_body
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let res_headers = serde_json::to_string(&args.res_headers)?;
        let patches_json = serde_json::to_string(&args.patches)?;
        // Empty list → NULL, the canonical "no conditions" state.
        let match_conditions = if args.match_conditions.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&args.match_conditions)?)
        };
        let mode = match args.mode.as_str() {
            "patch" => "patch",
            _ => "stub",
        };
        let conn = self.conn.lock();
        // Preserve created_at on update.
        let existing_created: Option<i64> = conn
            .query_row(
                "SELECT created_at FROM rule WHERE id=?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let created_at = existing_created.unwrap_or(now);
        conn.execute(
            "INSERT INTO rule (id, name, enabled, priority,
                    match_host_glob, match_method, match_path_glob, match_query,
                    res_status, res_headers, res_body_id, res_delay_ms,
                    created_at, updated_at, collection_id, mode, patches, match_req_body,
                    match_conditions)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name, enabled=excluded.enabled, priority=excluded.priority,
                match_host_glob=excluded.match_host_glob, match_method=excluded.match_method,
                match_path_glob=excluded.match_path_glob, match_query=excluded.match_query,
                res_status=excluded.res_status, res_headers=excluded.res_headers,
                res_body_id=excluded.res_body_id, res_delay_ms=excluded.res_delay_ms,
                collection_id=excluded.collection_id, mode=excluded.mode,
                patches=excluded.patches, match_req_body=excluded.match_req_body,
                match_conditions=excluded.match_conditions,
                updated_at=excluded.updated_at",
            params![
                id.to_string(),
                &args.name,
                args.enabled as i64,
                args.priority,
                args.match_host_glob,
                args.match_method,
                args.match_path_glob,
                match_params,
                args.res_status as i64,
                res_headers,
                body_id.map(|u| u.to_string()),
                args.res_delay_ms as i64,
                created_at,
                now,
                args.collection_id.map(|u| u.to_string()),
                mode,
                patches_json,
                match_req_body,
                match_conditions,
            ],
        )?;
        drop(conn);
        self.get_rule(id)
    }

    pub fn delete_rule(&self, id: Uuid) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM rule WHERE id=?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn set_rule_enabled(&self, args: RuleSetEnabledArgs) -> Result<()> {
        let conn = self.conn.lock();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        conn.execute(
            "UPDATE rule SET enabled=?1, updated_at=?2 WHERE id=?3",
            params![args.enabled as i64, now, args.id.to_string()],
        )?;
        Ok(())
    }

    /// Flip `enabled` on a whole scope of rules in one statement.
    ///
    /// The GUI used to fan out one IPC call per rule because there was no
    /// batch endpoint; from the CLI the same job meant one process launch per
    /// rule, and each of those re-listed the entire library to resolve its
    /// selector. On a 622-rule library that is 622 round trips to answer
    /// "turn everything off" — slow enough that people stopped doing it, which
    /// is how a run ends up with rules live that nobody meant to leave on.
    pub fn set_rules_enabled_bulk(
        &self,
        args: RulesSetEnabledBulkArgs,
    ) -> Result<RulesSetEnabledBulkResult> {
        let conn = self.conn.lock();
        let now = OffsetDateTime::now_utc().unix_timestamp();

        // Built as a fragment rather than interpolating user input: the only
        // thing that varies is the shape of the predicate, and the collection
        // id still travels as a bound parameter.
        let (predicate, coll): (&str, Option<String>) = match &args.scope {
            RuleBulkScope::All => ("1=1", None),
            RuleBulkScope::Ungrouped => ("collection_id IS NULL", None),
            RuleBulkScope::Collection { id } => ("collection_id = ?3", Some(id.to_string())),
        };

        let matched: u64 = {
            let sql = format!("SELECT COUNT(*) FROM rule WHERE {predicate}");
            match &coll {
                Some(c) => conn.query_row(&sql.replace("?3", "?1"), params![c], |r| r.get(0))?,
                None => conn.query_row(&sql, [], |r| r.get(0))?,
            }
        };

        // `AND enabled<>?1` keeps `changed` honest and skips rewriting rows
        // that already hold the target value.
        let sql =
            format!("UPDATE rule SET enabled=?1, updated_at=?2 WHERE {predicate} AND enabled<>?1");
        let changed = match &coll {
            Some(c) => conn.execute(&sql, params![args.enabled as i64, now, c])?,
            None => conn.execute(&sql, params![args.enabled as i64, now])?,
        };

        Ok(RulesSetEnabledBulkResult {
            matched,
            changed: changed as u64,
        })
    }

    pub fn set_rule_priority(&self, args: RuleSetPriorityArgs) -> Result<()> {
        let conn = self.conn.lock();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        conn.execute(
            "UPDATE rule SET priority=?1, updated_at=?2 WHERE id=?3",
            params![args.priority, now, args.id.to_string()],
        )?;
        Ok(())
    }

    /// Load active rules with their bodies materialized for the engine
    /// matcher. A rule is active on its own `enabled` flag alone. Ordered by
    /// collection priority then rule priority then created_at.
    ///
    /// **The checkbox is the only truth.** There is deliberately no second
    /// switch on the collection that can also silence a rule. A collection is
    /// grouping and ordering, nothing more.
    ///
    /// This was tried the other way and rejected: with a cascade, a rule can
    /// sit there ticked and still not fire, and nothing about the rule itself
    /// explains why. Every "my mock isn't working" then has two places to look
    /// instead of one. Switching scenarios does not need the extra state —
    /// `set_rules_enabled_bulk` over a collection scope ticks and unticks the
    /// boxes directly, which is both visible in the UI and the same mechanism
    /// the user operates by hand.
    ///
    /// `rule_collection.enabled` still exists in the schema (V003) and is still
    /// carried in the DTO for export/import round-trips, but nothing reads it
    /// to decide what serves traffic. Do not reintroduce it here.
    pub fn list_active_rules(&self) -> Result<Vec<ActiveRule>> {
        let dtos = {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare(
                "SELECT r.id, r.name, r.enabled, r.priority,
                        r.match_host_glob, r.match_method, r.match_path_glob, r.match_query,
                        r.res_status, r.res_headers, r.res_body_id, r.res_delay_ms,
                        r.created_at, r.updated_at, r.collection_id, r.mode, r.patches,
                        r.match_req_body, r.match_conditions
                 FROM rule r
                 LEFT JOIN rule_collection c ON c.id = r.collection_id
                 WHERE r.enabled=1
                 ORDER BY COALESCE(c.priority, 0) ASC,
                          r.priority ASC,
                          r.created_at ASC",
            )?;
            let rows = stmt.query_map([], Self::map_rule_row)?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };
        let mut out = Vec::with_capacity(dtos.len());
        for dto in dtos {
            let (mime, body) = match dto.res_body_id {
                Some(bid) => self
                    .bodies
                    .get_raw(bid, &self.conn)
                    .unwrap_or((None, vec![])),
                None => (None, vec![]),
            };
            let mode = match dto.mode.as_str() {
                "patch" => RuleMode::Patch,
                _ => RuleMode::Stub,
            };
            let patches = dto
                .patches
                .into_iter()
                .filter_map(|p| match p.op.as_str() {
                    "set" => Some(PatchOp::Set {
                        path: p.path,
                        value: p.value.unwrap_or(serde_json::Value::Null),
                    }),
                    "delete" => Some(PatchOp::Delete { path: p.path }),
                    "append" => Some(PatchOp::Append {
                        path: p.path,
                        value: p.value.unwrap_or(serde_json::Value::Null),
                    }),
                    _ => None,
                })
                .collect();
            out.push(ActiveRule {
                id: dto.id,
                name: dto.name,
                priority: dto.priority,
                mode,
                patches,
                host_glob: dto.match_host_glob,
                method: dto.match_method,
                path_glob: dto.match_path_glob,
                params: dto
                    .match_params
                    .into_iter()
                    .map(|q| (q.name, q.value))
                    .collect(),
                // Parse the JSON template once at load. A malformed
                // template degrades to "no body matching" rather than
                // blocking the rule.
                req_body_match: dto
                    .match_req_body
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
                // Parse op strings → enum once; conditions with an unknown
                // operator are dropped (skipped) rather than failing the rule.
                conditions: dto
                    .match_conditions
                    .into_iter()
                    .filter_map(|c| {
                        ConditionOp::parse(&c.op).map(|op| RuleCondition {
                            path: c.path,
                            op,
                            value: c.value,
                        })
                    })
                    .collect(),
                status: dto.res_status,
                headers: dto
                    .res_headers
                    .into_iter()
                    .map(|h| (h.name, h.value))
                    .collect(),
                body_mime: mime,
                body,
                delay_ms: dto.res_delay_ms,
            });
        }
        Ok(out)
    }

    fn map_rule_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RuleDto> {
        let id: String = r.get(0)?;
        let match_params_json: String = r.get(7)?;
        let res_headers: String = r.get(9)?;
        let body_id: Option<String> = r.get(10)?;
        let created_at: i64 = r.get(12)?;
        let updated_at: i64 = r.get(13)?;
        let collection_id: Option<String> = r.get(14)?;
        let mode: String = r.get(15)?;
        let patches_json: String = r.get(16)?;
        let match_req_body: Option<String> = r.get(17)?;
        let match_conditions_json: Option<String> = r.get(18)?;
        Ok(RuleDto {
            id: Uuid::parse_str(&id).unwrap(),
            name: r.get(1)?,
            enabled: r.get::<_, i64>(2)? != 0,
            priority: r.get(3)?,
            collection_id: collection_id.and_then(|s| Uuid::parse_str(&s).ok()),
            mode,
            patches: serde_json::from_str::<Vec<RulePatchOpDto>>(&patches_json).unwrap_or_default(),
            match_host_glob: r.get(4)?,
            match_method: r.get(5)?,
            match_path_glob: r.get(6)?,
            match_params: serde_json::from_str::<Vec<RuleParamDto>>(&match_params_json)
                .unwrap_or_default(),
            match_req_body,
            match_conditions: match_conditions_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<RuleConditionDto>>(s).ok())
                .unwrap_or_default(),
            res_status: r.get::<_, i64>(8)? as u16,
            res_headers: serde_json::from_str::<Vec<RuleHeaderDto>>(&res_headers)
                .unwrap_or_default(),
            res_body_id: body_id.and_then(|s| Uuid::parse_str(&s).ok()),
            res_body_mime: None,
            res_body_size: 0,
            res_delay_ms: r.get::<_, i64>(11)? as u64,
            created_at: created_at.to_string(),
            updated_at: updated_at.to_string(),
        })
    }

    pub fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }
}

#[cfg(test)]
mod filter_dedup_tests {
    use super::*;
    use pane_ipc::SaveFilterArgs;
    use tempfile::tempdir;

    fn args(name: &str, query: &str, kind: &str) -> SaveFilterArgs {
        SaveFilterArgs {
            id: None,
            name: name.into(),
            query: query.into(),
            color: "#fff".into(),
            pinned: false,
            kind: kind.into(),
        }
    }

    #[test]
    fn resaving_same_name_updates_not_duplicates() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        let a = s.save_filter(args("rc1", "host:rc1", "captures")).unwrap();
        // Re-save under the same name (no id, as a fresh save would) — must
        // reuse the row, not create a second "rc1".
        let b = s
            .save_filter(args("rc1", "host:rc1 status:200", "captures"))
            .unwrap();
        assert_eq!(a.id, b.id, "same name+kind must reuse the row");
        let list = s.list_filters(Some("captures")).unwrap();
        assert_eq!(list.iter().filter(|f| f.name == "rc1").count(), 1);
        assert_eq!(
            list.iter().find(|f| f.name == "rc1").unwrap().query,
            "host:rc1 status:200"
        );
    }

    #[test]
    fn same_name_different_kind_stays_separate() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        let a = s.save_filter(args("dev", "host:dev", "captures")).unwrap();
        let b = s.save_filter(args("dev", "tag:dev", "logcat")).unwrap();
        assert_ne!(a.id, b.id, "different kinds are independent scopes");
    }

    #[test]
    fn name_match_is_case_and_whitespace_insensitive() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();
        let a = s.save_filter(args("RC1", "host:rc1", "captures")).unwrap();
        let b = s.save_filter(args("  rc1 ", "host:x", "captures")).unwrap();
        assert_eq!(a.id, b.id, "trim + lowercase name should match");
    }
}

#[cfg(test)]
mod rule_enablement_tests {
    use super::*;
    use pane_ipc::{CollectionUpsertArgs, RuleUpsertArgs};
    use tempfile::tempdir;

    fn collection(s: &Storage, name: &str, enabled: bool) -> Uuid {
        s.upsert_collection(CollectionUpsertArgs {
            id: None,
            name: name.into(),
            enabled,
            priority: 0,
        })
        .unwrap()
        .id
    }

    fn rule(s: &Storage, name: &str, collection_id: Option<Uuid>) -> Uuid {
        s.upsert_rule(RuleUpsertArgs {
            id: None,
            name: name.into(),
            enabled: true,
            priority: 0,
            collection_id,
            mode: "stub".into(),
            patches: vec![],
            match_host_glob: Some("api.example.com".into()),
            match_method: None,
            match_path_glob: None,
            match_params: vec![],
            match_req_body: None,
            match_conditions: vec![],
            res_status: 200,
            res_headers: vec![],
            res_body_id: None,
            res_body_base64: None,
            res_body_mime: None,
            res_delay_ms: 0,
        })
        .unwrap()
        .id
    }

    fn active_names(s: &Storage) -> Vec<String> {
        s.list_active_rules()
            .unwrap()
            .into_iter()
            .map(|r| r.name)
            .collect()
    }

    /// The checkbox is the only truth. A collection's own flag must never
    /// mask a ticked rule — that was tried and reverted, because it produced
    /// rules that looked enabled in the list and silently never fired, with
    /// nothing on the rule itself to explain why.
    #[test]
    fn a_collections_flag_never_masks_its_rules() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();

        let off = collection(&s, "flag-is-off", false);
        rule(&s, "ticked", Some(off));

        assert_eq!(
            active_names(&s),
            vec!["ticked"],
            "rule.enabled alone decides whether a rule serves traffic"
        );
    }

    #[test]
    fn ungrouped_rules_need_only_their_own_flag() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();

        rule(&s, "ungrouped", None);

        assert_eq!(active_names(&s), vec!["ungrouped"]);
    }

    /// What `collections only <sel>` does now: clear every box, then tick the
    /// one collection. Same outcome as before, expressed in the state the user
    /// can actually see.
    #[test]
    fn switching_scenarios_moves_the_checkboxes() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();

        let a = collection(&s, "scenario-a", true);
        let b = collection(&s, "scenario-b", true);
        rule(&s, "rule-a", Some(a));
        rule(&s, "rule-b", Some(b));

        s.set_rules_enabled_bulk(pane_ipc::RulesSetEnabledBulkArgs {
            enabled: false,
            scope: RuleBulkScope::All,
        })
        .unwrap();
        s.set_rules_enabled_bulk(pane_ipc::RulesSetEnabledBulkArgs {
            enabled: true,
            scope: RuleBulkScope::Collection { id: b },
        })
        .unwrap();

        assert_eq!(
            active_names(&s),
            vec!["rule-b"],
            "switching collections must switch which rules serve traffic"
        );
    }

    #[test]
    fn bulk_disable_all_clears_the_whole_library() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();

        let c = collection(&s, "scenario", true);
        rule(&s, "a", Some(c));
        rule(&s, "b", Some(c));
        rule(&s, "c", None);

        let r = s
            .set_rules_enabled_bulk(pane_ipc::RulesSetEnabledBulkArgs {
                enabled: false,
                scope: RuleBulkScope::All,
            })
            .unwrap();
        assert_eq!((r.matched, r.changed), (3, 3));
        assert!(active_names(&s).is_empty());

        // Re-running is a no-op, and says so rather than reporting work.
        let again = s
            .set_rules_enabled_bulk(pane_ipc::RulesSetEnabledBulkArgs {
                enabled: false,
                scope: RuleBulkScope::All,
            })
            .unwrap();
        assert_eq!((again.matched, again.changed), (3, 0));
    }

    #[test]
    fn bulk_scope_collection_leaves_the_rest_alone() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();

        let a = collection(&s, "scenario-a", true);
        let b = collection(&s, "scenario-b", true);
        rule(&s, "in-a", Some(a));
        rule(&s, "in-b", Some(b));
        rule(&s, "ungrouped", None);

        let r = s
            .set_rules_enabled_bulk(pane_ipc::RulesSetEnabledBulkArgs {
                enabled: false,
                scope: RuleBulkScope::Collection { id: a },
            })
            .unwrap();
        assert_eq!((r.matched, r.changed), (1, 1));

        let mut live = active_names(&s);
        live.sort();
        assert_eq!(live, vec!["in-b", "ungrouped"]);
    }

    #[test]
    fn bulk_scope_ungrouped_touches_only_orphans() {
        let dir = tempdir().unwrap();
        let s = Storage::open(dir.path()).unwrap();

        let c = collection(&s, "scenario", true);
        rule(&s, "grouped", Some(c));
        rule(&s, "orphan", None);

        let r = s
            .set_rules_enabled_bulk(pane_ipc::RulesSetEnabledBulkArgs {
                enabled: false,
                scope: RuleBulkScope::Ungrouped,
            })
            .unwrap();
        assert_eq!((r.matched, r.changed), (1, 1));
        assert_eq!(active_names(&s), vec!["grouped"]);
    }
}

#[cfg(test)]
mod timestamp_tests {
    use super::*;

    /// `jq`'s `fromdate` and `date -d` both reject the default
    /// `OffsetDateTime::to_string()` shape, so anything an agent parses has to
    /// be RFC 3339.
    #[test]
    fn wire_timestamps_are_rfc3339() {
        let s = rfc3339(1_754_395_864);
        assert!(s.ends_with('Z'), "expected a Z-suffixed instant, got {s}");
        assert!(s.contains('T'), "expected a T separator, got {s}");
        assert!(!s.contains(' '), "RFC 3339 has no spaces, got {s}");
        assert!(
            time::OffsetDateTime::parse(&s, &time::format_description::well_known::Rfc3339).is_ok(),
            "{s} does not round-trip as RFC 3339"
        );
    }

    #[test]
    fn an_unrepresentable_timestamp_degrades_instead_of_panicking() {
        assert_eq!(rfc3339(i64::MAX), i64::MAX.to_string());
    }
}
