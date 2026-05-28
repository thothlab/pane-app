//! Replay: take a RequestSpec, fire it via reqwest, persist both request and
//! response as a fresh capture row, and link to a ReplayRecord. For MVP we
//! bypass the proxy and mark the row as `is_replay=1` so the UI can tag it.

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use mycharles_ipc::{HeaderDto, ReplayRecordDto, ReplaySendArgs, RequestSpec};
use rusqlite::params;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Storage;

pub async fn send(storage: &Storage, args: ReplaySendArgs) -> Result<ReplayRecordDto> {
    let session_id = storage
        .current_session_id()?
        .ok_or_else(|| anyhow!("no active session — start the proxy first"))?;

    let req = &args.request;
    let started_at = OffsetDateTime::now_utc();

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()?;

    let method = req
        .method
        .parse::<reqwest::Method>()
        .map_err(|e| anyhow!("invalid method: {e}"))?;

    let mut builder = client.request(method, &req.url);
    for h in &req.headers {
        builder = builder.header(h.name.clone(), h.value.clone());
    }
    let body_bytes = body_to_bytes(req);
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes.clone());
    }

    let resp = builder.send().await.context("replay send failed")?;
    let status = resp.status().as_u16();
    let http_version = format!("{:?}", resp.version());
    let res_headers: Vec<HeaderDto> = resp
        .headers()
        .iter()
        .map(|(k, v)| HeaderDto {
            name: k.to_string(),
            value: v.to_str().unwrap_or("").to_string(),
        })
        .collect();
    let resp_bytes = resp.bytes().await?;
    let ended_at = OffsetDateTime::now_utc();
    let duration_ms = (ended_at - started_at).whole_milliseconds().max(0) as i64;

    // URL → host/port/path. Tolerant parse; bail with anyhow on bad URL.
    let url = url_parse(&req.url)?;
    let scheme = url.scheme;
    let server_host = url.host;
    let server_port = url.port;
    let url_path = url.path_and_query;

    // Persist as a capture row.
    let cap_id = Uuid::new_v4();
    let req_body_id = if body_bytes.is_empty() {
        None
    } else {
        Some(storage.bodies.put(&body_bytes, "identity", None, &storage.conn)?)
    };
    let mime = res_headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.as_str());
    let res_body_id = if resp_bytes.is_empty() {
        None
    } else {
        Some(storage.bodies.put(&resp_bytes, "identity", mime, &storage.conn)?)
    };

    {
        let conn = storage.conn.lock();
        conn.execute(
            "INSERT INTO capture (id, session_id, started_at, ended_at, client_addr,
                server_host, server_port, scheme, http_version, method, url_path, status,
                req_body_id, res_body_id, total_bytes, duration_ms, state, error_kind, is_replay)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,'completed',NULL,1)",
            params![
                cap_id.to_string(),
                session_id.to_string(),
                started_at.unix_timestamp(),
                ended_at.unix_timestamp(),
                "127.0.0.1:0",
                server_host,
                server_port as i64,
                scheme,
                http_version,
                req.method,
                url_path,
                status as i64,
                req_body_id.map(|u| u.to_string()),
                res_body_id.map(|u| u.to_string()),
                (body_bytes.len() + resp_bytes.len()) as i64,
                duration_ms
            ],
        )?;

        for (idx, h) in req.headers.iter().enumerate() {
            conn.execute(
                "INSERT INTO header (capture_id, direction, name, value, order_idx)
                 VALUES (?1,'request',?2,?3,?4)",
                params![cap_id.to_string(), &h.name, &h.value, idx as i64],
            )?;
        }
        for (idx, h) in res_headers.iter().enumerate() {
            conn.execute(
                "INSERT INTO header (capture_id, direction, name, value, order_idx)
                 VALUES (?1,'response',?2,?3,?4)",
                params![cap_id.to_string(), &h.name, &h.value, idx as i64],
            )?;
        }
    }

    storage.insert_replay_record(args.source_id, Some(cap_id))
}

fn body_to_bytes(req: &RequestSpec) -> Vec<u8> {
    if let Some(b64) = &req.body_base64 {
        return base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap_or_default();
    }
    req.body_text.clone().unwrap_or_default().into_bytes()
}

struct ParsedUrl {
    scheme: String,
    host: String,
    port: u16,
    path_and_query: String,
}

fn url_parse(s: &str) -> Result<ParsedUrl> {
    // Minimal hand-rolled URL parser to avoid a dependency just for this hot
    // path. Format: scheme://host[:port]/path[?query].
    let (scheme, rest) = s
        .split_once("://")
        .ok_or_else(|| anyhow!("missing scheme: {s}"))?;
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, "/".into()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or_else(|_| default_port(scheme))),
        None => (authority.to_string(), default_port(scheme)),
    };
    Ok(ParsedUrl {
        scheme: scheme.to_string(),
        host,
        port,
        path_and_query: path,
    })
}

fn default_port(scheme: &str) -> u16 {
    match scheme {
        "https" | "wss" => 443,
        _ => 80,
    }
}
