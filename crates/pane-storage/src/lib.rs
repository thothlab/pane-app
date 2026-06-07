//! SQLite storage layer.
//!
//! All schema lives in `migrations/`; runtime types here mirror the PRD data
//! model. Writes go through `Storage`; bodies above 64 KB spill to files
//! addressed by sha256 (content-addressed dedup).

mod bodies;
mod filter_dsl;
mod migrations;
mod replay_impl;

pub use bodies::BodyStore;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use pane_ipc::{
    CaptureBodyDto, CaptureDto, CollectionSetEnabledArgs, CollectionUpsertArgs, ExportOneResult,
    FilterDto, HeaderDto, ReplayRecordDto, ReplaySendArgs, RuleCollectionDto, RuleDto,
    RuleHeaderDto, RuleParamDto, RulePatchOpDto, RuleSetEnabledArgs, RuleUpsertArgs,
    SaveFilterArgs, SessionDto,
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

pub struct Storage {
    conn: Mutex<Connection>,
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

        migrations::runner()
            .run(&mut conn)
            .context("migrations failed")?;

        let bodies = Arc::new(BodyStore::new(data_dir.join("bodies"))?);
        Ok(Self {
            conn: Mutex::new(conn),
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
            started_at: now.to_string(),
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

    // ---------- Captures ----------

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
                    total_bytes, duration_ms, state, error_kind
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
                    total_bytes, duration_ms, state, error_kind
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
            started_at: OffsetDateTime::from_unix_timestamp(started_at)
                .unwrap()
                .to_string(),
            ended_at: ended_at.map(|t| OffsetDateTime::from_unix_timestamp(t).unwrap().to_string()),
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
        let id = args.id.unwrap_or_else(Uuid::new_v4);
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO saved_filter (id, name, query, color, pinned)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, query=excluded.query,
                color=excluded.color, pinned=excluded.pinned",
            params![
                id.to_string(),
                &args.name,
                &args.query,
                &args.color,
                args.pinned as i64
            ],
        )?;
        Ok(FilterDto {
            id,
            name: args.name,
            query: args.query,
            color: args.color,
            pinned: args.pinned,
        })
    }

    pub fn list_filters(&self) -> Result<Vec<FilterDto>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, query, color, pinned FROM saved_filter ORDER BY pinned DESC, name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(FilterDto {
                id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap(),
                name: r.get(1)?,
                query: r.get(2)?,
                color: r.get(3)?,
                pinned: r.get::<_, i64>(4)? != 0,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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

    pub fn delete_collection(&self, id: Uuid) -> Result<()> {
        let conn = self.conn.lock();
        // Detach rules first (so they end up in Ungrouped instead of cascading delete).
        conn.execute(
            "UPDATE rule SET collection_id = NULL WHERE collection_id=?1",
            params![id.to_string()],
        )?;
        conn.execute(
            "DELETE FROM rule_collection WHERE id=?1",
            params![id.to_string()],
        )?;
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

    // ---------- Rules (response stubbing) ----------

    pub fn list_rules(&self) -> Result<Vec<RuleDto>> {
        let mut dtos = {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare(
                "SELECT id, name, enabled, priority,
                        match_host_glob, match_method, match_path_glob, match_query,
                        res_status, res_headers, res_body_id, res_delay_ms,
                        created_at, updated_at, collection_id, mode, patches
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
                    created_at, updated_at, collection_id, mode, patches
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
        let res_headers = serde_json::to_string(&args.res_headers)?;
        let patches_json = serde_json::to_string(&args.patches)?;
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
                    created_at, updated_at, collection_id, mode, patches)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name, enabled=excluded.enabled, priority=excluded.priority,
                match_host_glob=excluded.match_host_glob, match_method=excluded.match_method,
                match_path_glob=excluded.match_path_glob, match_query=excluded.match_query,
                res_status=excluded.res_status, res_headers=excluded.res_headers,
                res_body_id=excluded.res_body_id, res_delay_ms=excluded.res_delay_ms,
                collection_id=excluded.collection_id, mode=excluded.mode,
                patches=excluded.patches, updated_at=excluded.updated_at",
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

    /// Load active rules with their bodies materialized for the engine
    /// matcher. A rule is active solely based on its own `enabled` flag;
    /// collections are purely a UI grouping. Ordered by collection priority
    /// then rule priority then created_at.
    pub fn list_active_rules(&self) -> Result<Vec<ActiveRule>> {
        let dtos = {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare(
                "SELECT r.id, r.name, r.enabled, r.priority,
                        r.match_host_glob, r.match_method, r.match_path_glob, r.match_query,
                        r.res_status, r.res_headers, r.res_body_id, r.res_delay_ms,
                        r.created_at, r.updated_at, r.collection_id, r.mode, r.patches
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
