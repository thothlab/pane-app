//! Engine trait + event types.
//!
//! Concrete implementations (e.g. mitmproxy-rs adapter, or a future native
//! engine) live in sibling crates and provide `impl ProxyEngine`. Tauri wires
//! the chosen impl into `AppState` so the rest of the app code stays engine-
//! agnostic.

use std::net::SocketAddr;

use async_trait::async_trait;
use pane_ca::CaMaterial;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

pub struct EngineConfig {
    pub listen: SocketAddr,
    pub ca: CaMaterial,
    /// Optional address for the PAC server. The MITM engine binds an
    /// extra TCP listener here that serves a Proxy Auto-Config script
    /// pointing at `listen`. Pane sets `http_proxy_pac` on devices to
    /// this URL — Android falls back to DIRECT when it's unreachable
    /// (USB unplugged, Pane stopped), so the phone keeps its internet.
    pub pac_listen: Option<SocketAddr>,
    /// Optional address for the heartbeat server. The companion APK on
    /// the device connects here (adb-reverse-forwarded) and pings every
    /// 2 seconds. When the APK loses the connection — Pane stopped, USB
    /// unplugged — it clears `http_proxy` on the device so the user
    /// doesn't get stranded with no internet.
    pub heartbeat_listen: Option<SocketAddr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineEvent {
    RequestStarted {
        id: Uuid,
        host: String,
        method: String,
        path: String,
        started_at: String,
    },
    ResponseHeaders {
        id: Uuid,
        status: u16,
    },
    Completed {
        id: Uuid,
        status: u16,
        duration_ms: u64,
        total_bytes: u64,
    },
    Error {
        id: Uuid,
        host: String,
        error_kind: String,
        message: String,
    },
    PinningSuspected {
        id: Uuid,
        host: String,
        alpn: Option<String>,
    },
}

impl EngineEvent {
    pub fn topic(&self) -> &'static str {
        match self {
            EngineEvent::RequestStarted { .. } => "capture.started",
            EngineEvent::ResponseHeaders { .. } => "capture.headers",
            EngineEvent::Completed { .. } => "capture.completed",
            EngineEvent::Error { .. } => "capture.error",
            EngineEvent::PinningSuspected { .. } => "pinning.detected",
        }
    }

    pub fn payload(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[derive(Clone)]
pub struct EngineHandle {
    pub listen: SocketAddr,
    pub pac_listen: Option<SocketAddr>,
    pub heartbeat_listen: Option<SocketAddr>,
    pub shutdown_tx: tokio::sync::mpsc::Sender<()>,
    pub pac_shutdown_tx: Option<tokio::sync::mpsc::Sender<()>>,
    pub heartbeat_shutdown_tx: Option<tokio::sync::mpsc::Sender<()>>,
}

impl EngineHandle {
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let _ = self.shutdown_tx.send(()).await;
        if let Some(pac_tx) = &self.pac_shutdown_tx {
            let _ = pac_tx.send(()).await;
        }
        if let Some(hb_tx) = &self.heartbeat_shutdown_tx {
            let _ = hb_tx.send(()).await;
        }
        Ok(())
    }
}

#[async_trait]
pub trait ProxyEngine: Send + Sync {
    async fn start(&self, cfg: EngineConfig) -> anyhow::Result<EngineHandle>;
    fn events(&self) -> broadcast::Receiver<EngineEvent>;
}
