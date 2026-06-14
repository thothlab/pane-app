//! Per-connection loop: parse the first request line, decide CONNECT vs plain
//! HTTP, persist a capture, emit events.
//!
//! HTTPS path:
//!   1. Read the CONNECT request headers, persist an in_flight capture row
//!      tagged scheme="https".
//!   2. Reply `200 Connection Established` on the raw TCP socket.
//!   3. Upgrade the socket via tokio-rustls; the cert resolver inside
//!      `LeafCache` mints a leaf cert keyed by SNI.
//!   4. Parse the inner HTTP/1.1 request, forward upstream over HTTPS via
//!      reqwest, write the response back over the TLS stream.
//!
//! Single request per TLS connection (the client reopens for the next one).
//! HTTP/2 is suppressed via ALPN. Chunked request bodies are not yet read —
//! Content-Length is the common case for typical mobile-app POSTs.

use std::net::SocketAddr;
use std::sync::Arc;

use pane_engine::EngineEvent;
use pane_storage::Storage;
use rusqlite::params;
use time::OffsetDateTime;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio_rustls::TlsAcceptor;
use uuid::Uuid;

pub async fn handle(
    mut stream: TcpStream,
    peer: SocketAddr,
    storage: Arc<Storage>,
    events: broadcast::Sender<EngineEvent>,
    tls_acceptor: TlsAcceptor,
) -> anyhow::Result<()> {
    // Read raw bytes into a Vec until we see the end-of-headers marker, so we
    // know exactly how much has been consumed off the socket. Using BufReader
    // here is dangerous on the CONNECT path: it over-reads, and handing the
    // wrapped TcpStream to TlsAcceptor would silently drop a pipelined
    // ClientHello.
    let (head, extra) = match read_request_head(&mut stream).await? {
        Some(h) => h,
        None => return Ok(()),
    };
    let (method, target, headers) = head;

    let started_at = OffsetDateTime::now_utc();
    let cap_id = crate::new_capture_id();
    let session_id = storage.current_session_id()?.ok_or_else(|| {
        anyhow::anyhow!("no active session — proxy.start should have created one")
    })?;

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = match target.split_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(443)),
            None => (target.clone(), 443),
        };

        emit_started(&events, cap_id, &host, &method, "/");
        insert_capture_opening(
            &storage, cap_id, session_id, peer, &host, port, "https", &method,
        )?;

        // RFC 7231 says a client SHOULD NOT pipeline anything after CONNECT
        // until the 200 reply lands. If a client did anyway, those bytes
        // belong to the *inside* of the would-be tunnel — we can't feed them
        // to TlsAcceptor and we can't proxy them either. Treat as fatal.
        if !extra.is_empty() {
            mark_error(&storage, cap_id, "connect_pipelined", "")?;
            emit_error(
                &events,
                cap_id,
                &host,
                "connect_pipelined",
                "bytes after CONNECT",
            );
            return Ok(());
        }
        stream
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;

        let tls_stream = match tls_acceptor.accept(stream).await {
            Ok(s) => s,
            Err(e) => {
                mark_error(&storage, cap_id, "tls_handshake", &e.to_string())?;
                emit_error(&events, cap_id, &host, "tls_handshake", &e.to_string());
                return Ok(());
            }
        };

        return handle_tls_inner(tls_stream, host, port, cap_id, started_at, storage, events).await;
    }

    // Plain HTTP: target is an absolute URL like http://host/path.
    let (host, port, path) = parse_http_target(&target);
    emit_started(&events, cap_id, &host, &method, &path);
    insert_capture_opening(
        &storage, cap_id, session_id, peer, &host, port, "http", &method,
    )?;
    update_url_path(&storage, cap_id, &path)?;
    persist_headers(&storage, cap_id, "request", &headers)?;

    // Read Content-Length-bounded request body. `extra` holds bytes the head
    // parser already pulled off the socket past \r\n\r\n — typically the
    // first chunk of a POST body. Without forwarding it, POST/PUT/PATCH look
    // bodyless to the upstream and stall (Envoy upstream waits 60s then 503s).
    let req_body = assemble_request_body(extra, &mut stream, &headers).await?;
    if !req_body.is_empty() {
        if let Some(req_body_id) = persist_body(&storage, &req_body, &headers, "request")? {
            set_req_body(&storage, cap_id, req_body_id)?;
        }
    }

    let mut write_half = stream;

    // Stub/patch hook: short-circuit on stub, remember rule on patch so we
    // can mutate the upstream response below.
    let mut patch_rule: Option<pane_storage::ActiveRule> = None;
    if let Ok(rules) = storage.list_active_rules() {
        tracing::debug!(
            count = rules.len(),
            ids = ?rules.iter().map(|r| (r.id, r.name.as_str())).collect::<Vec<_>>(),
            "active rules loaded for HTTP request"
        );
        let content_type_lower = content_type_lower(&headers);
        let req = crate::rules::RequestSummary {
            host: &host,
            method: &method,
            path: &path,
            body: &req_body,
            content_type: content_type_lower.as_deref(),
        };
        if let Some(rule) = crate::rules::first_match(&rules, req) {
            tracing::info!(
                rule_id = %rule.id,
                rule_name = %rule.name,
                mode = ?rule.mode,
                host = %host,
                method = %method,
                path = %path,
                "rule matched HTTP request"
            );
            match rule.mode {
                pane_storage::RuleMode::Stub => {
                    serve_stub(&mut write_half, &storage, &events, cap_id, started_at, rule)
                        .await?;
                    return Ok(());
                }
                pane_storage::RuleMode::Patch => {
                    patch_rule = Some(rule.clone());
                }
            }
        }
    }

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let url = format!("http://{host}:{port}{path}");
    let mut builder = client.request(method.parse()?, &url);
    for (k, v) in &headers {
        let kl = k.to_ascii_lowercase();
        if kl == "host" || kl == "content-length" || kl == "transfer-encoding" {
            continue;
        }
        builder = builder.header(k, v);
    }
    if !req_body.is_empty() {
        builder = builder.body(req_body);
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

    let mut status = resp.status().as_u16();
    let mut res_headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let raw_body = resp.bytes().await?.to_vec();
    let body =
        apply_patches_if_any(patch_rule.as_ref(), &mut status, &mut res_headers, raw_body).await;

    persist_headers(&storage, cap_id, "response", &res_headers)?;

    let mime = res_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.as_str());

    let body_id = storage
        .bodies
        .put(&body, "identity", mime, storage.conn())?;
    set_res_body(&storage, cap_id, body_id)?;

    write_response(&mut write_half, status, &res_headers, &body).await?;

    let ended_at = OffsetDateTime::now_utc();
    let duration_ms = (ended_at - started_at).whole_milliseconds().max(0) as i64;
    if patch_rule.is_some() {
        mark_patched(
            &storage,
            cap_id,
            status,
            body.len() as i64,
            duration_ms,
            ended_at,
        )?;
    } else {
        mark_completed(
            &storage,
            cap_id,
            status,
            body.len() as i64,
            duration_ms,
            ended_at,
        )?;
    }
    emit_completed(
        &events,
        cap_id,
        status,
        duration_ms as u64,
        body.len() as u64,
    );
    Ok(())
}

async fn handle_tls_inner(
    tls_stream: tokio_rustls::server::TlsStream<TcpStream>,
    host: String,
    port: u16,
    cap_id: Uuid,
    started_at: OffsetDateTime,
    storage: Arc<Storage>,
    events: broadcast::Sender<EngineEvent>,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(tls_stream);

    let (method, target, headers) = match read_tls_request_head(&mut reader).await? {
        Some(h) => h,
        None => {
            mark_error(&storage, cap_id, "empty_tls_request", "")?;
            return Ok(());
        }
    };
    let path = if target.starts_with('/') {
        target.clone()
    } else {
        format!("/{target}")
    };
    // Replace the placeholder CONNECT method (set when the outer
    // tunnel row was inserted) with the actual HTTP verb from inside
    // the TLS stream — otherwise the entire HTTPS traffic shows up
    // as CONNECT in the captures list.
    update_method(&storage, cap_id, &method)?;
    update_url_path(&storage, cap_id, &path)?;
    persist_headers(&storage, cap_id, "request", &headers)?;

    // Content-Length-bounded request body. Chunked encoding is rare on outbound
    // mobile traffic; deferring it limits this PR's surface area.
    let req_body = read_request_body(&mut reader, &headers).await?;
    if !req_body.is_empty() {
        if let Some(req_body_id) = persist_body(&storage, &req_body, &headers, "request")? {
            set_req_body(&storage, cap_id, req_body_id)?;
        }
    }

    // Stub/patch hook (TLS): on stub, short-circuit; on patch, remember and
    // fall through to upstream so we can mutate the response below.
    let mut patch_rule: Option<pane_storage::ActiveRule> = None;
    if let Ok(rules) = storage.list_active_rules() {
        tracing::debug!(
            count = rules.len(),
            ids = ?rules.iter().map(|r| (r.id, r.name.as_str())).collect::<Vec<_>>(),
            "active rules loaded for HTTPS request"
        );
        let content_type_lower = content_type_lower(&headers);
        let req = crate::rules::RequestSummary {
            host: &host,
            method: &method,
            path: &path,
            body: &req_body,
            content_type: content_type_lower.as_deref(),
        };
        if let Some(rule) = crate::rules::first_match(&rules, req) {
            tracing::info!(
                rule_id = %rule.id,
                rule_name = %rule.name,
                mode = ?rule.mode,
                host = %host,
                method = %method,
                path = %path,
                "rule matched HTTPS request"
            );
            match rule.mode {
                pane_storage::RuleMode::Stub => {
                    let mut tls_stream = reader.into_inner();
                    serve_stub(&mut tls_stream, &storage, &events, cap_id, started_at, rule)
                        .await?;
                    let _ = tls_stream.shutdown().await;
                    return Ok(());
                }
                pane_storage::RuleMode::Patch => {
                    patch_rule = Some(rule.clone());
                }
            }
        }
    }

    let url = format!("https://{host}:{port}{path}");
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut builder = client.request(method.parse()?, &url);
    for (k, v) in &headers {
        let kl = k.to_ascii_lowercase();
        if kl == "host" || kl == "content-length" || kl == "transfer-encoding" {
            continue;
        }
        builder = builder.header(k, v);
    }
    if !req_body.is_empty() {
        builder = builder.body(req_body);
    }

    let resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            mark_error(&storage, cap_id, "upstream", &e.to_string())?;
            emit_error(&events, cap_id, &host, "upstream", &e.to_string());
            let mut w = reader.into_inner();
            let _ = w.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
            return Ok(());
        }
    };

    let mut status = resp.status().as_u16();
    let mut res_headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let raw_body = resp.bytes().await?.to_vec();
    let body =
        apply_patches_if_any(patch_rule.as_ref(), &mut status, &mut res_headers, raw_body).await;

    persist_headers(&storage, cap_id, "response", &res_headers)?;

    let mime = res_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.as_str());
    let body_id = storage
        .bodies
        .put(&body, "identity", mime, storage.conn())?;
    set_res_body(&storage, cap_id, body_id)?;

    let mut tls_stream = reader.into_inner();
    write_response(&mut tls_stream, status, &res_headers, &body).await?;
    let _ = tls_stream.shutdown().await;

    let ended_at = OffsetDateTime::now_utc();
    let duration_ms = (ended_at - started_at).whole_milliseconds().max(0) as i64;
    if patch_rule.is_some() {
        mark_patched(
            &storage,
            cap_id,
            status,
            body.len() as i64,
            duration_ms,
            ended_at,
        )?;
    } else {
        mark_completed(
            &storage,
            cap_id,
            status,
            body.len() as i64,
            duration_ms,
            ended_at,
        )?;
    }
    emit_completed(
        &events,
        cap_id,
        status,
        duration_ms as u64,
        body.len() as u64,
    );
    Ok(())
}

type ParsedHead = (String, String, Vec<(String, String)>);

/// Read the request head (request-line + headers + blank line) off `stream`
/// directly, without a BufReader. Returns the parsed head plus any bytes that
/// were read past the `\r\n\r\n` boundary — callers decide whether to forward
/// them (HTTPS body) or drop them.
async fn read_request_head<R>(stream: &mut R) -> anyhow::Result<Option<(ParsedHead, Vec<u8>)>>
where
    R: AsyncReadExt + Unpin,
{
    const MAX_HEAD: usize = 64 * 1024;
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 1024];
    let end = loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            return Err(anyhow::anyhow!("eof before end of headers"));
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = find_double_crlf(&buf) {
            break p + 4;
        }
        if buf.len() > MAX_HEAD {
            return Err(anyhow::anyhow!("request head exceeds {MAX_HEAD} bytes"));
        }
    };
    let extra = buf[end..].to_vec();
    let head_str = std::str::from_utf8(&buf[..end])?;
    let mut lines = head_str.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Ok(Some(((method, target, headers), extra)))
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Same as `read_request_head` but for an already-buffered TLS stream where
/// over-reading is harmless — we keep using BufReader there for read-line
/// ergonomics. Kept separate so the CONNECT path can't accidentally use this.
async fn read_tls_request_head<R>(reader: &mut BufReader<R>) -> anyhow::Result<Option<ParsedHead>>
where
    R: AsyncReadExt + Unpin,
{
    let mut request_line = String::new();
    let n = reader.read_line(&mut request_line).await?;
    if n == 0 {
        return Ok(None);
    }
    let request_line = request_line.trim_end().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
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
    Ok(Some((method, target, headers)))
}

async fn read_request_body<R>(
    reader: &mut BufReader<R>,
    headers: &[(String, String)],
) -> anyhow::Result<Vec<u8>>
where
    R: AsyncReadExt + Unpin,
{
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    if len == 0 {
        return Ok(Vec::new());
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_response<W>(
    w: &mut W,
    status: u16,
    res_headers: &[(String, String)],
    body: &[u8],
) -> anyhow::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut head = format!("HTTP/1.1 {status} OK\r\n");
    for (k, v) in res_headers {
        let kl = k.to_ascii_lowercase();
        if kl == "transfer-encoding" || kl == "content-length" || kl == "connection" {
            continue;
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    head.push_str("Connection: close\r\n\r\n");
    w.write_all(head.as_bytes()).await?;
    w.write_all(body).await?;
    Ok(())
}

/// Lowercased Content-Type header value (just the leading part is fine; the
/// matcher only does substring checks like `.contains("json")`).
fn content_type_lower(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_ascii_lowercase())
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

#[allow(clippy::too_many_arguments)]
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

fn update_method(storage: &Storage, id: Uuid, method: &str) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    conn.execute(
        "UPDATE capture SET method=?1 WHERE id=?2",
        params![method, id.to_string()],
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

/// Pull a Content-Length-bounded request body off a raw stream, prepending
/// any bytes that the head-parser already consumed past `\r\n\r\n`.
///
/// Chunked transfer-encoding is not yet handled — that's rare on mobile
/// outbound traffic but should be addressed eventually. For now bodies
/// without Content-Length are treated as empty.
async fn assemble_request_body(
    extra: Vec<u8>,
    stream: &mut TcpStream,
    headers: &[(String, String)],
) -> anyhow::Result<Vec<u8>> {
    let cl = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    if cl == 0 {
        return Ok(Vec::new());
    }
    if extra.len() >= cl {
        return Ok(extra[..cl].to_vec());
    }
    let need = cl - extra.len();
    let mut body = extra;
    body.reserve(need);
    let mut rest = vec![0u8; need];
    stream.read_exact(&mut rest).await?;
    body.extend_from_slice(&rest);
    Ok(body)
}

/// Persist a body blob into the content-addressed body store and return its id.
/// Returns `None` for empty bodies — those aren't worth a row.
fn persist_body(
    storage: &Storage,
    bytes: &[u8],
    headers: &[(String, String)],
    _direction: &str,
) -> anyhow::Result<Option<Uuid>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let mime = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.as_str());
    let id = storage
        .bodies
        .put(bytes, "identity", mime, storage.conn())?;
    Ok(Some(id))
}

fn set_req_body(storage: &Storage, id: Uuid, body_id: Uuid) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    conn.execute(
        "UPDATE capture SET req_body_id=?1 WHERE id=?2",
        params![body_id.to_string(), id.to_string()],
    )?;
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
    status: u16,
    total_bytes: i64,
    duration_ms: i64,
    ended_at: OffsetDateTime,
) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    conn.execute(
        "UPDATE capture SET state='completed', status=?1, ended_at=?2, duration_ms=?3,
                            total_bytes=?4
         WHERE id=?5",
        params![
            status as i64,
            ended_at.unix_timestamp(),
            duration_ms,
            total_bytes,
            id.to_string(),
        ],
    )?;
    Ok(())
}

/// If a patch-mode rule fired, parse the body as JSON, apply each patch op
/// to a virtual `{status, headers, body}` tree, and return the re-serialized
/// body. Returns `raw_body` unchanged when there's no rule or the body is
/// not JSON.
async fn apply_patches_if_any(
    rule: Option<&pane_storage::ActiveRule>,
    status: &mut u16,
    headers: &mut Vec<(String, String)>,
    raw_body: Vec<u8>,
) -> Vec<u8> {
    let Some(rule) = rule else { return raw_body };
    if rule.delay_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(rule.delay_ms)).await;
    }
    let ct_json = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_ascii_lowercase().contains("json"))
        .unwrap_or(false);
    if !ct_json || raw_body.is_empty() {
        // Status/header-only patches still apply even when the body isn't
        // JSON; we just skip body ops.
        let mut placeholder = serde_json::Value::Null;
        let mut tree = crate::patch::ResponseTree {
            status,
            headers,
            body: &mut placeholder,
        };
        for op in &rule.patches {
            crate::patch::apply(&mut tree, op);
        }
        return raw_body;
    }
    let mut body_json: serde_json::Value = match serde_json::from_slice(&raw_body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "patch: response body is not valid JSON; serving as-is");
            return raw_body;
        }
    };
    let mut tree = crate::patch::ResponseTree {
        status,
        headers,
        body: &mut body_json,
    };
    for op in &rule.patches {
        crate::patch::apply(&mut tree, op);
    }
    match serde_json::to_vec(&body_json) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "patch: failed to re-serialize patched body; serving raw");
            raw_body
        }
    }
}

fn mark_patched(
    storage: &Storage,
    id: Uuid,
    status: u16,
    total_bytes: i64,
    duration_ms: i64,
    ended_at: OffsetDateTime,
) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    conn.execute(
        "UPDATE capture SET state='patched', status=?1, ended_at=?2, duration_ms=?3,
                            total_bytes=?4
         WHERE id=?5",
        params![
            status as i64,
            ended_at.unix_timestamp(),
            duration_ms,
            total_bytes,
            id.to_string(),
        ],
    )?;
    Ok(())
}

fn mark_stubbed(
    storage: &Storage,
    id: Uuid,
    status: u16,
    total_bytes: i64,
    duration_ms: i64,
    ended_at: OffsetDateTime,
) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    conn.execute(
        "UPDATE capture SET state='stubbed', status=?1, ended_at=?2, duration_ms=?3,
                            total_bytes=?4
         WHERE id=?5",
        params![
            status as i64,
            ended_at.unix_timestamp(),
            duration_ms,
            total_bytes,
            id.to_string(),
        ],
    )?;
    Ok(())
}

/// Persist the stub response (headers + body) onto an in-flight capture,
/// write it to the client stream, mark the capture as stubbed, and emit a
/// completed event.
async fn serve_stub<W>(
    stream: &mut W,
    storage: &Storage,
    events: &broadcast::Sender<EngineEvent>,
    cap_id: Uuid,
    started_at: OffsetDateTime,
    rule: &pane_storage::ActiveRule,
) -> anyhow::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    if rule.delay_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(rule.delay_ms)).await;
    }
    persist_headers(storage, cap_id, "response", &rule.headers)?;
    let body_id = storage.bodies.put(
        &rule.body,
        "identity",
        rule.body_mime.as_deref(),
        storage.conn(),
    )?;
    set_res_body(storage, cap_id, body_id)?;
    write_response(stream, rule.status, &rule.headers, &rule.body).await?;
    let ended_at = OffsetDateTime::now_utc();
    let duration_ms = (ended_at - started_at).whole_milliseconds().max(0) as i64;
    mark_stubbed(
        storage,
        cap_id,
        rule.status,
        rule.body.len() as i64,
        duration_ms,
        ended_at,
    )?;
    emit_completed(
        events,
        cap_id,
        rule.status,
        duration_ms as u64,
        rule.body.len() as u64,
    );
    Ok(())
}

fn mark_error(storage: &Storage, id: Uuid, kind: &str, _msg: &str) -> anyhow::Result<()> {
    let conn = storage.conn().lock();
    conn.execute(
        "UPDATE capture SET state='error', error_kind=?1, ended_at=?2 WHERE id=?3",
        params![
            kind,
            OffsetDateTime::now_utc().unix_timestamp(),
            id.to_string()
        ],
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
