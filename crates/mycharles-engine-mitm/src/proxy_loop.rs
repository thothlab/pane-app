//! Per-connection loop: parse the first request line, decide CONNECT vs plain
//! HTTP, persist a capture, emit events.
//!
//! Note: this is a deliberately small, line-buffered HTTP/1.1 implementation
//! to keep MVP code reviewable. TLS-side decryption requires a rustls
//! ServerConfig built from the per-SNI leaf in `leaf.rs`. Full TLS termination
//! is a substantial wire-up — for MVP this module proxies HTTPS opaquely
//! (plain CONNECT tunnel) and records request metadata (host, port, started_at,
//! state) without decrypting payloads. A follow-up task enables decryption
//! once the rustls plumbing is in place. The capture model is already wired
//! so the upgrade is a localized change.

use std::net::SocketAddr;
use std::sync::Arc;

use mycharles_engine::EngineEvent;
use mycharles_storage::Storage;
use rusqlite::params;
use time::OffsetDateTime;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::leaf::LeafCache;

pub async fn handle(
    stream: TcpStream,
    peer: SocketAddr,
    storage: Arc<Storage>,
    events: broadcast::Sender<EngineEvent>,
    _leaf_cache: Arc<LeafCache>,
) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let mut request_line = String::new();
    let n = reader.read_line(&mut request_line).await?;
    if n == 0 {
        return Ok(());
    }
    let request_line = request_line.trim_end().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let _version = parts.next().unwrap_or("HTTP/1.1").to_string();

    // Drain remaining headers (we don't yet read body for opaque CONNECT mode).
    let mut headers = Vec::<(String, String)>::new();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.trim_end().split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    let started_at = OffsetDateTime::now_utc();
    let cap_id = crate::new_capture_id();
    let session_id = storage.current_session_id()?.ok_or_else(|| {
        anyhow::anyhow!("no active session — proxy.start should have created one")
    })?;

    if method.eq_ignore_ascii_case("CONNECT") {
        // CONNECT host:port HTTP/1.1
        let (host, port) = match target.split_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(443)),
            None => (target.clone(), 443),
        };

        emit_started(&events, cap_id, &host, &method, "/");
        insert_capture_opening(&storage, cap_id, session_id, peer, &host, port, "https", &method)?;

        // Reply 200 to client.
        write_half
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;

        // Open upstream TCP and proxy bytes opaquely. This preserves end-to-end
        // TLS — no decryption. The capture row stays at state=in_flight until
        // the tunnel closes, then we mark it completed with total_bytes.
        let upstream = match TcpStream::connect((host.as_str(), port)).await {
            Ok(s) => s,
            Err(e) => {
                mark_error(&storage, cap_id, "connection_refused", &e.to_string())?;
                emit_error(&events, cap_id, &host, "connection_refused", &e.to_string());
                return Ok(());
            }
        };

        let (mut up_read, mut up_write) = upstream.into_split();
        let mut downstream_read = reader.into_inner();

        let c2s = tokio::io::copy(&mut downstream_read, &mut up_write);
        let s2c = tokio::io::copy(&mut up_read, &mut write_half);

        let (c2s_n, s2c_n) = tokio::join!(c2s, s2c);
        let total = c2s_n.unwrap_or(0) + s2c_n.unwrap_or(0);
        let ended_at = OffsetDateTime::now_utc();
        let duration_ms = (ended_at - started_at).whole_milliseconds().max(0) as i64;
        mark_completed(&storage, cap_id, total as i64, duration_ms, ended_at)?;
        emit_completed(&events, cap_id, 0, duration_ms as u64, total);
        return Ok(());
    }

    // Plain HTTP: target is an absolute URL like http://host/path
    let (host, port, path) = parse_http_target(&target);
    emit_started(&events, cap_id, &host, &method, &path);
    insert_capture_opening(&storage, cap_id, session_id, peer, &host, port, "http", &method)?;
    update_url_path(&storage, cap_id, &path)?;
    persist_headers(&storage, cap_id, "request", &headers)?;

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let url = format!("http://{host}:{port}{path}");
    let mut builder = client.request(method.parse()?, &url);
    for (k, v) in &headers {
        if k.eq_ignore_ascii_case("host") {
            continue;
        }
        builder = builder.header(k, v);
    }

    let resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            mark_error(&storage, cap_id, "upstream", &e.to_string())?;
            emit_error(&events, cap_id, &host, "upstream", &e.to_string());
            let _ = write_half
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n")
                .await;
            return Ok(());
        }
    };

    let status = resp.status().as_u16();
    let res_headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    persist_headers(&storage, cap_id, "response", &res_headers)?;
    let body = resp.bytes().await?;

    let mime = res_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.as_str());

    let body_id = storage.bodies.put(&body, "identity", mime, storage.conn())?;
    set_res_body(&storage, cap_id, body_id)?;

    let mut head = format!("HTTP/1.1 {status} OK\r\n");
    for (k, v) in &res_headers {
        if k.eq_ignore_ascii_case("transfer-encoding") || k.eq_ignore_ascii_case("content-length")
        {
            continue;
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    write_half.write_all(head.as_bytes()).await?;
    write_half.write_all(&body).await?;

    let ended_at = OffsetDateTime::now_utc();
    let duration_ms = (ended_at - started_at).whole_milliseconds().max(0) as i64;
    mark_completed(&storage, cap_id, body.len() as i64, duration_ms, ended_at)?;
    emit_completed(&events, cap_id, status, duration_ms as u64, body.len() as u64);
    Ok(())
}

fn parse_http_target(target: &str) -> (String, u16, String) {
    if let Some(rest) = target.strip_prefix("http://") {
        let (authority, path) = match rest.split_once('/') {
            Some((a, p)) => (a, format!("/{p}")),
            None => (rest, "/".into()),
        };
        let (h, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().unwrap_or(80)),
            None => (authority.to_string(), 80),
        };
        return (h, port, path);
    }
    (String::from("unknown"), 80, target.to_string())
}

fn insert_capture_opening(
    storage: &Storage,
    id: Uuid,
    session_id: Uuid,
    peer: SocketAddr,
    host: &str,
    port: u16,
    scheme: &str,
    method: &str,
) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    conn.execute(
        "INSERT INTO capture (id, session_id, started_at, client_addr, server_host, server_port,
                              scheme, http_version, method, url_path, total_bytes, state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '1.1', ?8, '/', 0, 'in_flight')",
        params![
            id.to_string(),
            session_id.to_string(),
            OffsetDateTime::now_utc().unix_timestamp(),
            peer.to_string(),
            host,
            port as i64,
            scheme,
            method,
        ],
    )?;
    Ok(())
}

fn update_url_path(storage: &Storage, id: Uuid, path: &str) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    conn.execute(
        "UPDATE capture SET url_path=?1 WHERE id=?2",
        params![path, id.to_string()],
    )?;
    Ok(())
}

fn persist_headers(
    storage: &Storage,
    id: Uuid,
    direction: &str,
    headers: &[(String, String)],
) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    for (idx, (k, v)) in headers.iter().enumerate() {
        conn.execute(
            "INSERT INTO header (capture_id, direction, name, value, order_idx)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.to_string(), direction, k, v, idx as i64],
        )?;
    }
    Ok(())
}

fn set_res_body(storage: &Storage, id: Uuid, body_id: Uuid) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    conn.execute(
        "UPDATE capture SET res_body_id=?1 WHERE id=?2",
        params![body_id.to_string(), id.to_string()],
    )?;
    Ok(())
}

fn mark_completed(
    storage: &Storage,
    id: Uuid,
    total_bytes: i64,
    duration_ms: i64,
    ended_at: OffsetDateTime,
) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    conn.execute(
        "UPDATE capture SET state='completed', ended_at=?1, duration_ms=?2, total_bytes=?3
         WHERE id=?4",
        params![ended_at.unix_timestamp(), duration_ms, total_bytes, id.to_string()],
    )?;
    Ok(())
}

fn mark_error(storage: &Storage, id: Uuid, kind: &str, _msg: &str) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    conn.execute(
        "UPDATE capture SET state='error', error_kind=?1, ended_at=?2 WHERE id=?3",
        params![kind, OffsetDateTime::now_utc().unix_timestamp(), id.to_string()],
    )?;
    Ok(())
}

fn emit_started(
    events: &broadcast::Sender<EngineEvent>,
    id: Uuid,
    host: &str,
    method: &str,
    path: &str,
) {
    let _ = events.send(EngineEvent::RequestStarted {
        id,
        host: host.into(),
        method: method.into(),
        path: path.into(),
        started_at: OffsetDateTime::now_utc().to_string(),
    });
}

fn emit_completed(
    events: &broadcast::Sender<EngineEvent>,
    id: Uuid,
    status: u16,
    duration_ms: u64,
    total_bytes: u64,
) {
    let _ = events.send(EngineEvent::Completed {
        id,
        status,
        duration_ms,
        total_bytes,
    });
}

fn emit_error(
    events: &broadcast::Sender<EngineEvent>,
    id: Uuid,
    host: &str,
    kind: &str,
    msg: &str,
) {
    let _ = events.send(EngineEvent::Error {
        id,
        host: host.into(),
        error_kind: kind.into(),
        message: msg.into(),
    });
}

#[allow(dead_code)]
async fn _drain<R: AsyncReadExt + Unpin>(r: &mut R) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    r.read_to_end(&mut buf).await?;
    Ok(buf)
}
