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

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use mycharles_ca::CaMaterial;
use mycharles_engine::{EngineConfig, EngineEvent, EngineHandle, ProxyEngine};
use mycharles_storage::Storage;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

mod leaf;
mod proxy_loop;

pub struct MitmEngine {
    storage: Arc<Storage>,
    events_tx: broadcast::Sender<EngineEvent>,
}

impl MitmEngine {
    pub fn new(storage: Arc<Storage>) -> Self {
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
                                let leaf_cache = leaf_cache.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = proxy_loop::handle(stream, peer, storage, events, leaf_cache).await {
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

// Re-export for handlers.
pub(crate) fn new_capture_id() -> Uuid {
    Uuid::new_v4()
}

#[allow(dead_code)]
pub(crate) fn _force_use(_: SocketAddr, _: &CaMaterial) {} // suppress unused warns in stubs

pub(crate) async fn _drain<R: AsyncReadExt + Unpin>(r: &mut R) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    r.read_to_end(&mut buf).await?;
    Ok(buf)
}

pub(crate) async fn _write_all<W: AsyncWriteExt + Unpin>(
    w: &mut W,
    bytes: &[u8],
) -> anyhow::Result<()> {
    w.write_all(bytes).await?;
    Ok(())
}
