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
    pub shutdown_tx: tokio::sync::mpsc::Sender<()>,
}

impl EngineHandle {
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let _ = self.shutdown_tx.send(()).await;
        Ok(())
    }
}

#[async_trait]
pub trait ProxyEngine: Send + Sync {
    async fn start(&self, cfg: EngineConfig) -> anyhow::Result<EngineHandle>;
    fn events(&self) -> broadcast::Receiver<EngineEvent>;
}
