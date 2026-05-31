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
use pane_ipc::{
    CaptureBodyDto, CaptureDto, ExportOneResult, FilterDto, HeaderDto, ReplayRecordDto,
    ReplaySendArgs, SaveFilterArgs, SessionDto,
};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use time::OffsetDateTime;
use uuid::Uuid;

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

        migrations::runner().run(&mut conn).context("migrations failed")?;

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
                    id: Uuid::parse_str(&r.get::<_, String>(0)?)
                        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?,
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
            params![id.to_string(), now.unix_timestamp(), listen.to_string(), ca_id],
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
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
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
            started_at: OffsetDateTime::from_unix_timestamp(started_at).unwrap().to_string(),
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
        conn.execute(
            "DELETE FROM capture_body
              WHERE id NOT IN (SELECT req_body_id FROM capture WHERE req_body_id IS NOT NULL
                               UNION
                               SELECT res_body_id FROM capture WHERE res_body_id IS NOT NULL)",
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
                Ok(ExportOneResult { text: s, mime: "text/plain".into() })
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
                Ok(ExportOneResult { text: serde_json::to_string_pretty(&har)?, mime: "application/json".into() })
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
        conn.execute("DELETE FROM saved_filter WHERE id=?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }
}
