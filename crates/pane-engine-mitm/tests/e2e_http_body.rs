//! End-to-end test for the plain-HTTP POST path with a request body.
//!
//! Regression coverage for the Envoy/Istio 503 timeout caused by Pane
//! silently dropping POST bodies on the plain-HTTP forward — see the fix in
//! `proxy_loop.rs::assemble_request_body` + `set_req_body`. The mock upstream
//! here echoes the body back and asserts the body was received.

use std::sync::Arc;

use pane_ca::CaMaterial;
use pane_engine::{EngineConfig, ProxyEngine};
use pane_engine_mitm::MitmEngine;
use pane_storage::Storage;
use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, PKCS_ED25519};
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const REQ_BODY: &[u8] = b"{\"login\":\"public\",\"password\":\"123123\"}";

fn make_ca() -> CaMaterial {
    let kp = KeyPair::generate_for(&PKCS_ED25519).unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "pane-test-ca");
    params.distinguished_name = dn;
    let cert = params.self_signed(&kp).unwrap();
    CaMaterial {
        id: uuid::Uuid::new_v4(),
        cert_pem: cert.pem(),
        key_pem: kp.serialize_pem(),
    }
}

/// Mock HTTP upstream: reads the full request (head + Content-Length body),
/// asserts the received body matches REQ_BODY, then echoes 200 with a JSON
/// payload. If body is missing or wrong, returns 503 with a marker so the
/// test sees a clear failure mode.
async fn run_mock_http_upstream(listener: TcpListener) {
    loop {
        let (mut sock, _) = match listener.accept().await {
            Ok(v) => v,
            Err(_) => return,
        };
        tokio::spawn(async move {
            let mut buf = Vec::with_capacity(4096);
            let mut tmp = [0u8; 1024];
            let head_end = loop {
                let n = match sock.read(&mut tmp).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break p + 4;
                }
                if buf.len() > 64 * 1024 {
                    return;
                }
            };
            let head_str = std::str::from_utf8(&buf[..head_end]).unwrap_or("");
            let cl: usize = head_str
                .lines()
                .find_map(|l| {
                    let mut parts = l.splitn(2, ':');
                    let k = parts.next()?.trim();
                    let v = parts.next()?.trim();
                    if k.eq_ignore_ascii_case("content-length") {
                        v.parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            let mut body = buf[head_end..].to_vec();
            if body.len() < cl {
                let need = cl - body.len();
                let mut rest = vec![0u8; need];
                if sock.read_exact(&mut rest).await.is_err() {
                    return;
                }
                body.extend_from_slice(&rest);
            }
            let (status_line, payload) = if body == REQ_BODY {
                ("HTTP/1.1 200 OK", &b"{\"ok\":true}"[..])
            } else {
                (
                    "HTTP/1.1 503 Service Unavailable",
                    &b"upstream connect error: missing/garbled body"[..],
                )
            };
            let resp = format!(
                "{status_line}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.write_all(payload).await;
            let _ = sock.shutdown().await;
        });
    }
}

fn pick_port() -> std::net::SocketAddr {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    a
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_http_post_body_round_trip() {
    let tmp = tempdir().unwrap();
    let storage = Arc::new(Storage::open(tmp.path()).unwrap());

    let ca = make_ca();
    let sha = format!("{:x}", Sha256::digest(ca.cert_pem.as_bytes()));
    let nb = OffsetDateTime::now_utc();
    storage
        .insert_ca(ca.id, &ca.cert_pem, &sha, "pane-test-ca", nb, nb + time::Duration::days(365))
        .unwrap();

    // Mock HTTP upstream.
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();
    tokio::spawn(run_mock_http_upstream(upstream_listener));

    // Proxy.
    let proxy_addr = pick_port();
    storage.session_record(proxy_addr).unwrap();
    let engine = MitmEngine::new(storage.clone());
    let _handle = engine
        .start(EngineConfig { listen: proxy_addr, ca: ca.clone(), pac_listen: None })
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Send the plain-HTTP POST through the proxy. Charles/curl-style: the
    // request line carries an absolute URL — that's what HTTP clients emit
    // when they're proxy-aware (or when Android's system proxy is set).
    let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "POST http://localhost:{p}/api/auth HTTP/1.1\r\n\
         Host: localhost:{p}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {cl}\r\n\
         Connection: close\r\n\
         \r\n",
        p = upstream_addr.port(),
        cl = REQ_BODY.len(),
    );
    sock.write_all(req.as_bytes()).await.unwrap();
    sock.write_all(REQ_BODY).await.unwrap();

    // Read the response fully until EOF.
    let mut resp = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match sock.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => resp.extend_from_slice(&chunk[..n]),
        }
    }
    let resp_str = String::from_utf8_lossy(&resp);
    assert!(
        resp_str.contains("200 OK"),
        "expected 200 from upstream (body was forwarded), got: {resp_str}"
    );
    assert!(
        resp_str.contains("\"ok\":true"),
        "expected ok-payload in response, got: {resp_str}"
    );

    // DB-side assertions: status is 503-less (it should be 200), and there
    // should be a req_body_id pointing to the persisted JSON.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let (status, req_body_id): (Option<i64>, Option<String>) = storage
        .conn()
        .lock()
        .query_row(
            "SELECT status, req_body_id FROM capture ORDER BY started_at DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, Some(200), "capture.status should be persisted as 200");
    assert!(req_body_id.is_some(), "capture.req_body_id should be set after body forward");
}
