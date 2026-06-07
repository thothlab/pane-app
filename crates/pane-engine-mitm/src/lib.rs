//! HTTP/1.1 MITM proxy.
//!
//! Listens on a local TCP socket. For each accepted connection:
//! 1. Read the first request — if it's a `CONNECT host:port`, switch into
//!    TLS-MITM mode: reply `200 OK`, perform a TLS handshake using a leaf
//!    cert issued by our root CA, parse the inner HTTP request, forward it
//!    upstream via reqwest, persist a capture row, send the response back.
//! 2. For plain HTTP requests, forward and persist.
//!
//! This implementation is intentionally compact: it covers the MVP code paths
//! (HTTP/1.1, JSON/text/binary bodies, error reporting) and stops short of
//! HTTP/2 multiplexing and CONNECT-tunneled WebSockets. Both are tracked as
//! follow-up tasks; the trait surface is engine-agnostic so a richer engine
//! (mitmproxy-rs once stable) can drop in without rewiring callers.

use std::sync::Arc;

use async_trait::async_trait;
use pane_engine::{EngineConfig, EngineEvent, EngineHandle, ProxyEngine};
use pane_storage::Storage;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

mod heartbeat;
mod leaf;
mod pac;
mod patch;
mod proxy_loop;
mod rules;

/// Public helper for the `pac_smoke` example. Not intended for app code —
/// it just runs the PAC server in the foreground and waits.
#[doc(hidden)]
pub async fn __pac_smoke_helper() -> anyhow::Result<()> {
    let listen: std::net::SocketAddr = "127.0.0.1:8889".parse().unwrap();
    let _tx = pac::start(listen, "127.0.0.1".into(), 8888).await?;
    tracing::info!("PAC server up on {listen}. Ctrl-C to stop.");
    tokio::signal::ctrl_c().await?;
    Ok(())
}

pub struct MitmEngine {
    storage: Arc<Storage>,
    events_tx: broadcast::Sender<EngineEvent>,
}

impl MitmEngine {
    pub fn new(storage: Arc<Storage>) -> Self {
        // rustls 0.23 panics on ServerConfig::builder() without a process-wide
        // CryptoProvider. install_default() errors if already installed (e.g.
        // reqwest's rustls-tls beat us to it) — ignoring that is correct.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (tx, _rx) = broadcast::channel(4096);
        Self { storage, events_tx: tx }
    }
}

#[async_trait]
impl ProxyEngine for MitmEngine {
    async fn start(&self, cfg: EngineConfig) -> anyhow::Result<EngineHandle> {
        let listener = TcpListener::bind(cfg.listen).await?;
        tracing::info!(listen = %cfg.listen, "proxy engine listening");

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let storage = self.storage.clone();
        let events = self.events_tx.clone();
        let ca = Arc::new(cfg.ca);
        let leaf_cache = Arc::new(leaf::LeafCache::new(ca.clone()));

        // Single ServerConfig reused across connections. The cert resolver is
        // SNI-keyed and caches per host inside LeafCache, so no per-connection
        // setup beyond the TLS handshake itself. ALPN is restricted to HTTP/1.1:
        // we don't parse h2 yet, and offering only http/1.1 forces clients to
        // downgrade rather than open an unintelligible h2 connection.
        let mut server_cfg = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(leaf_cache.clone());
        server_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        let tls_acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_cfg));

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        tracing::info!("proxy engine shutting down");
                        break;
                    }
                    accept = listener.accept() => {
                        match accept {
                            Ok((stream, peer)) => {
                                let storage = storage.clone();
                                let events = events.clone();
                                let tls_acceptor = tls_acceptor.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = proxy_loop::handle(stream, peer, storage, events, tls_acceptor).await {
                                        tracing::warn!(error = %e, "connection handler error");
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "accept failed");
                            }
                        }
                    }
                }
            }
        });

        // Spin up the PAC server if requested. Failure here is logged
        // but doesn't abort proxy startup — without PAC the device gets
        // the legacy direct-proxy experience (which still works while
        // USB is connected), so it's strictly an enhancement.
        let (pac_listen, pac_shutdown_tx) = match cfg.pac_listen {
            Some(addr) => {
                let proxy_host = cfg.listen.ip().to_string();
                let proxy_port = cfg.listen.port();
                match pac::start(addr, proxy_host, proxy_port).await {
                    Ok(tx) => (Some(addr), Some(tx)),
                    Err(e) => {
                        tracing::warn!(error = %e, addr = %addr, "PAC server failed to bind");
                        (None, None)
                    }
                }
            }
            None => (None, None),
        };

        // Heartbeat listener for the device-side companion APK. Same
        // best-effort pattern as PAC: bind failure is logged but not
        // fatal. Without it, the APK's watchdog can never connect and
        // stays in "disconnected" mode forever — harmless, just means
        // the unplug-no-internet protection doesn't kick in for this
        // session.
        let (heartbeat_listen, heartbeat_shutdown_tx) = match cfg.heartbeat_listen {
            Some(addr) => match heartbeat::start(addr).await {
                Ok(tx) => (Some(addr), Some(tx)),
                Err(e) => {
                    tracing::warn!(error = %e, addr = %addr, "heartbeat server failed to bind");
                    (None, None)
                }
            },
            None => (None, None),
        };

        Ok(EngineHandle {
            listen: cfg.listen,
            pac_listen,
            heartbeat_listen,
            shutdown_tx,
            pac_shutdown_tx,
            heartbeat_shutdown_tx,
        })
    }

    fn events(&self) -> broadcast::Receiver<EngineEvent> {
        self.events_tx.subscribe()
    }
}

pub(crate) fn new_capture_id() -> Uuid {
    Uuid::new_v4()
}
