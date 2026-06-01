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

mod leaf;
mod patch;
mod proxy_loop;
mod rules;

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

        Ok(EngineHandle { listen: cfg.listen, shutdown_tx })
    }

    fn events(&self) -> broadcast::Receiver<EngineEvent> {
        self.events_tx.subscribe()
    }
}

pub(crate) fn new_capture_id() -> Uuid {
    Uuid::new_v4()
}
