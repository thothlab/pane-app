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
use pane_engine::{
    DevicePortRegistry, EngineConfig, EngineEvent, EngineHandle, ProxyEngine, PROXY_PORT_POOL,
};
use pane_storage::Storage;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tokio_rustls::TlsAcceptor;
use uuid::Uuid;

mod heartbeat;
mod leaf;
mod pac;
mod passthrough;
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
        Self {
            storage,
            events_tx: tx,
        }
    }
}

#[async_trait]
impl ProxyEngine for MitmEngine {
    async fn start(&self, cfg: EngineConfig) -> anyhow::Result<EngineHandle> {
        let ca = Arc::new(cfg.ca);
        let leaf_cache = Arc::new(leaf::LeafCache::new(ca.clone()));

        // Single ServerConfig reused across connections and across every pool
        // port. The cert resolver is SNI-keyed and caches per host inside
        // LeafCache, so no per-connection setup beyond the TLS handshake
        // itself. ALPN is restricted to HTTP/1.1: we don't parse h2 yet, and
        // offering only http/1.1 forces clients to downgrade rather than open
        // an unintelligible h2 connection.
        let mut server_cfg = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(leaf_cache.clone());
        server_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        let tls_acceptor = TlsAcceptor::from(Arc::new(server_cfg));

        // One TCP listener per pool port so each USB device (routed via its own
        // `adb reverse tcp:8888 tcp:<pool_port>`) lands on a distinct local
        // port we can map back to a device_id. `cfg.listen` is the canonical
        // 8888 entry; the rest of the pool are spares. PAC/heartbeat stay on
        // their own dedicated ports and are untouched here.
        //
        // Shutdown: the external handle exposes an mpsc (one `()` = stop). We
        // fan that single signal out to every accept loop via a broadcast — an
        // mpsc receiver can't be cloned across N loops.
        // Hosts we won't decrypt. Shared by every accept loop so a lesson
        // learned on one device's port applies to all of them — the client
        // rejecting our cert is a property of the host, not of the port it
        // happened to arrive on.
        let no_mitm = passthrough::NoMitmSet::new();

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let (stop_tx, _stop_rx0) = broadcast::channel::<()>(1);
        {
            let stop_tx = stop_tx.clone();
            tokio::spawn(async move {
                let _ = shutdown_rx.recv().await;
                let _ = stop_tx.send(());
            });
        }

        // Canonical listen port (production: 8888 == PROXY_PORT_POOL[0]; tests:
        // an ephemeral port) is bound first and is fatal — it's what the device
        // (or test client) connects to. The rest of the attribution pool is
        // best-effort: a spare already in use just means that slot's device
        // won't get individual attribution.
        let host = cfg.listen.ip();
        let base_port = cfg.listen.port();
        let mut ports = vec![base_port];
        ports.extend(PROXY_PORT_POOL.iter().copied().filter(|p| *p != base_port));

        for (idx, port) in ports.into_iter().enumerate() {
            let addr = SocketAddr::new(host, port);
            let listener = match TcpListener::bind(addr).await {
                Ok(l) => l,
                Err(e) => {
                    if idx == 0 {
                        return Err(e.into());
                    }
                    tracing::warn!(error = %e, %addr, "pool proxy port failed to bind; skipping");
                    continue;
                }
            };
            tracing::info!(%addr, "proxy engine listening");
            spawn_accept_loop(
                listener,
                port,
                cfg.registry.clone(),
                self.storage.clone(),
                self.events_tx.clone(),
                tls_acceptor.clone(),
                stop_tx.subscribe(),
                no_mitm.clone(),
            );
        }

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

/// Spawn one accept loop for a single pool port. Each accepted connection is
/// handed `local_port` + the shared registry so `proxy_loop::handle` can
/// resolve which device it belongs to. Stops when `stop_rx` fires.
#[allow(clippy::too_many_arguments)]
fn spawn_accept_loop(
    listener: TcpListener,
    local_port: u16,
    registry: DevicePortRegistry,
    storage: Arc<Storage>,
    events: broadcast::Sender<EngineEvent>,
    tls_acceptor: TlsAcceptor,
    mut stop_rx: broadcast::Receiver<()>,
    no_mitm: passthrough::NoMitmSet,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop_rx.recv() => {
                    tracing::info!(port = local_port, "proxy accept loop shutting down");
                    break;
                }
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, peer)) => {
                            let registry = registry.clone();
                            let storage = storage.clone();
                            let events = events.clone();
                            let tls_acceptor = tls_acceptor.clone();
                            let no_mitm = no_mitm.clone();
                            tokio::spawn(async move {
                                if let Err(e) = proxy_loop::handle(
                                    stream, peer, local_port, registry, storage, events, tls_acceptor,
                                    no_mitm,
                                ).await {
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
}

pub(crate) fn new_capture_id() -> Uuid {
    Uuid::new_v4()
}
