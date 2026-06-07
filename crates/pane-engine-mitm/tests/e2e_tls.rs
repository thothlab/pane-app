//! End-to-end smoke test for the HTTPS MITM path.
//!
//! Wires up: an in-memory CA → MitmEngine → a mock TLS upstream that returns a
//! fixed body. The client then drives the proxy exactly the way a real HTTP
//! client would: TCP connect, send `CONNECT`, read `200 Connection Established`,
//! perform a TLS handshake trusting the in-memory CA, send `GET /`, read the
//! response body back over the encrypted tunnel. Asserts the body round-trips
//! and that the capture row in storage is tagged scheme=https.

use std::sync::Arc;

use pane_ca::CaMaterial;
use pane_engine::{EngineConfig, ProxyEngine};
use pane_engine_mitm::MitmEngine;
use pane_storage::Storage;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, SanType,
    PKCS_ECDSA_P256_SHA256, PKCS_ED25519,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

const UPSTREAM_BODY: &[u8] = b"hello over tls";

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

fn make_localhost_cert() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut params = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    params
        .subject_alt_names
        .push(SanType::DnsName("localhost".try_into().unwrap()));
    let cert = params.self_signed(&kp).unwrap();
    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(kp.serialize_der()));
    (cert_der, key_der)
}

async fn run_mock_https_upstream(listener: TcpListener) {
    let (cert, key) = make_localhost_cert();
    let cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    loop {
        let (sock, _) = match listener.accept().await {
            Ok(v) => v,
            Err(_) => return,
        };
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let mut tls = match acceptor.accept(sock).await {
                Ok(t) => t,
                Err(_) => return,
            };
            let mut buf = vec![0u8; 4096];
            // Read until end-of-headers — request body is empty for GET.
            let mut total = 0usize;
            loop {
                let n = match tls.read(&mut buf[total..]).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                total += n;
                if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
                if total == buf.len() {
                    return;
                }
            }
            let body = UPSTREAM_BODY;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = tls.write_all(head.as_bytes()).await;
            let _ = tls.write_all(body).await;
            let _ = tls.shutdown().await;
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
async fn end_to_end_https_mitm() {
    // Storage with migrations applied.
    let tmp = tempdir().unwrap();
    let storage = Arc::new(Storage::open(tmp.path()).unwrap());

    // Persist the CA so session_record can FK to it.
    let ca = make_ca();
    let sha = format!("{:x}", Sha256::digest(ca.cert_pem.as_bytes()));
    let nb = OffsetDateTime::now_utc();
    let na = nb + time::Duration::days(365);
    storage
        .insert_ca(ca.id, &ca.cert_pem, &sha, "pane-test-ca", nb, na)
        .unwrap();

    // Mock HTTPS upstream on a fresh local port.
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();
    tokio::spawn(run_mock_https_upstream(upstream_listener));

    // Pick a port for the proxy, register the session row, then start.
    let proxy_addr = pick_port();
    storage.session_record(proxy_addr).unwrap();
    let engine = MitmEngine::new(storage.clone());
    let _handle = engine
        .start(EngineConfig { listen: proxy_addr, ca: ca.clone(), pac_listen: None, heartbeat_listen: None })
        .await
        .unwrap();

    // Give the listener a brief moment to come up. The pick_port + start race
    // means a fast test machine occasionally beats the bind — a single retry
    // would be cleaner, but a short sleep keeps the test linear and readable.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // --- Client side: drive the proxy like curl would ---

    let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
    let connect = format!(
        "CONNECT localhost:{p} HTTP/1.1\r\nHost: localhost:{p}\r\n\r\n",
        p = upstream_addr.port()
    );
    sock.write_all(connect.as_bytes()).await.unwrap();
    let mut buf = [0u8; 1024];
    let n = sock.read(&mut buf).await.unwrap();
    let head = std::str::from_utf8(&buf[..n]).unwrap();
    assert!(
        head.starts_with("HTTP/1.1 200"),
        "expected 200 from CONNECT, got: {head:?}"
    );

    // TLS upgrade, trusting our in-memory CA.
    let mut root_store = rustls::RootCertStore::empty();
    let ca_ders = rustls_pemfile::certs(&mut ca.cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for der in ca_ders {
        root_store.add(der).unwrap();
    }
    let client_cfg = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(client_cfg));
    let sni = ServerName::try_from("localhost").unwrap();
    let mut tls = connector.connect(sni, sock).await.expect("tls handshake");

    // Send GET, read until EOF.
    tls.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut resp = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match tls.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => resp.extend_from_slice(&chunk[..n]),
        }
    }
    let resp_str = String::from_utf8_lossy(&resp);
    assert!(
        resp_str.contains("200"),
        "expected 200 in response, got: {resp_str}"
    );
    assert!(
        resp.windows(UPSTREAM_BODY.len()).any(|w| w == UPSTREAM_BODY),
        "expected upstream body in response, got: {resp_str}"
    );

    // Capture row should be present, tagged https.
    // Give the proxy a tick to finish the storage write after sending the
    // response (mark_completed runs after write_response).
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let count = storage.captures_count().unwrap();
    assert!(count >= 1, "expected at least one capture row");
    let scheme: String = storage
        .conn()
        .lock()
        .query_row(
            "SELECT scheme FROM capture ORDER BY started_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(scheme, "https");
}
