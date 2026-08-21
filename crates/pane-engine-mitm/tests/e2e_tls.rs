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
    // rustls 0.23 panics in ServerConfig/ClientConfig::builder() without a
    // process-wide CryptoProvider. `MitmEngine::new` installs one, but the
    // mock upstream below is spawned *before* the engine is constructed and
    // builds its own ServerConfig — so whether the test passed came down to
    // which task the scheduler ran first. Under a loaded machine (several test
    // binaries at once, which is cargo's default) the upstream usually won and
    // the run failed with "expected 200 in response". Install it up front so
    // the ordering stops mattering. Idempotent: `install_default` errors if a
    // provider is already set, and ignoring that is correct.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

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
        .start(EngineConfig {
            listen: proxy_addr,
            ca: ca.clone(),
            pac_listen: None,
            heartbeat_listen: None,
            registry: pane_engine::DevicePortRegistry::new(),
            no_mitm: pane_engine::NoMitmSet::new(),
        })
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
        resp.windows(UPSTREAM_BODY.len())
            .any(|w| w == UPSTREAM_BODY),
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

/// The HTTPS path must scope rules to the device the connection came from.
///
/// This is the test that would have caught the original gap: `handle` resolves
/// the device from the local port, but the TLS handler used to be called
/// without it, so rule matching *inside* the tunnel ran against the
/// unattributed set. Every storage unit test still passed, and since almost all
/// real mobile traffic is HTTPS, the feature would have looked like it simply
/// did nothing.
///
/// Two proxy ports, one registry, one storage: port A belongs to device A, port
/// B to device B, and a single rule is live only on A. Same request, same host,
/// two different answers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn https_rules_are_scoped_to_the_device_the_connection_came_from() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let tmp = tempdir().unwrap();
    let storage = Arc::new(Storage::open(tmp.path()).unwrap());

    let ca = make_ca();
    let sha = format!("{:x}", Sha256::digest(ca.cert_pem.as_bytes()));
    let nb = OffsetDateTime::now_utc();
    let na = nb + time::Duration::days(365);
    storage
        .insert_ca(ca.id, &ca.cert_pem, &sha, "pane-test-ca", nb, na)
        .unwrap();

    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();
    tokio::spawn(run_mock_https_upstream(upstream_listener));

    // A rule that stubs the same request the upstream would answer, live only
    // on device A. `device_id` needs no `device` row — the scope column holds
    // whatever id the port registry hands the proxy.
    storage
        .upsert_rule(pane_ipc::RuleUpsertArgs {
            id: None,
            name: "scoped-to-a".into(),
            enabled: true,
            enabled_scope: Some("set".into()),
            devices: Some(vec!["device-a".into()]),
            priority: 0,
            collection_id: None,
            mode: "stub".into(),
            patches: vec![],
            match_host_glob: Some("localhost".into()),
            match_method: None,
            match_path_glob: None,
            match_params: vec![],
            match_req_body: None,
            match_conditions: vec![],
            tags: vec![],
            res_status: 503,
            res_headers: vec![],
            res_body_id: None,
            res_body_base64: Some(base64_encode(STUB_BODY)),
            res_body_mime: Some("text/plain".into()),
            res_delay_ms: 0,
        })
        .unwrap();

    // One registry shared by both proxies, each port standing in for a device.
    let registry = pane_engine::DevicePortRegistry::new();
    let addr_a = pick_port();
    let addr_b = pick_port();
    registry.set_port(addr_a.port(), "device-a");
    registry.set_port(addr_b.port(), "device-b");
    storage.session_record(addr_a).unwrap();

    let mut handles = Vec::new();
    for addr in [addr_a, addr_b] {
        let engine = MitmEngine::new(storage.clone());
        handles.push(
            engine
                .start(EngineConfig {
                    listen: addr,
                    ca: ca.clone(),
                    pac_listen: None,
                    heartbeat_listen: None,
                    registry: registry.clone(),
                    no_mitm: pane_engine::NoMitmSet::new(),
                })
                .await
                .unwrap(),
        );
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let from_a = https_get_through(addr_a, upstream_addr.port(), &ca).await;
    let from_b = https_get_through(addr_b, upstream_addr.port(), &ca).await;

    assert!(
        from_a.contains(std::str::from_utf8(STUB_BODY).unwrap()),
        "device A should have been served the mock, got: {from_a}"
    );
    assert!(
        from_b.contains(std::str::from_utf8(UPSTREAM_BODY).unwrap()),
        "device B is outside the rule's scope and should have reached the real \
         upstream, got: {from_b}"
    );
    assert!(
        !from_b.contains(std::str::from_utf8(STUB_BODY).unwrap()),
        "device B must not see device A's scenario, got: {from_b}"
    );

    // And the capture rows agree — this is the same pair of assertions the CLI
    // workflow makes with `state:stubbed rule:` and `device:`.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let stubbed_for: Vec<(String, Option<String>)> = {
        let conn = storage.conn().lock();
        let mut stmt = conn
            .prepare(
                "SELECT state, device_id FROM capture \
                 WHERE matched_rule_name = 'scoped-to-a'",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    };
    assert_eq!(
        stubbed_for,
        vec![("stubbed".to_string(), Some("device-a".to_string()))],
        "exactly one capture, on device A, should have been served by the rule"
    );
}

const STUB_BODY: &[u8] = b"served by the mock";

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// CONNECT + TLS + GET through one proxy address, returning the raw response.
async fn https_get_through(
    proxy: std::net::SocketAddr,
    upstream_port: u16,
    ca: &CaMaterial,
) -> String {
    let mut sock = TcpStream::connect(proxy).await.unwrap();
    let connect = format!(
        "CONNECT localhost:{upstream_port} HTTP/1.1\r\nHost: localhost:{upstream_port}\r\n\r\n"
    );
    sock.write_all(connect.as_bytes()).await.unwrap();
    let mut buf = [0u8; 1024];
    let n = sock.read(&mut buf).await.unwrap();
    assert!(
        std::str::from_utf8(&buf[..n])
            .unwrap()
            .starts_with("HTTP/1.1 200"),
        "CONNECT failed through {proxy}"
    );

    let mut root_store = rustls::RootCertStore::empty();
    for der in rustls_pemfile::certs(&mut ca.cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    {
        root_store.add(der).unwrap();
    }
    let client_cfg = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(client_cfg));
    let sni = ServerName::try_from("localhost").unwrap();
    let mut tls = connector.connect(sni, sock).await.expect("tls handshake");

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
    String::from_utf8_lossy(&resp).to_string()
}
