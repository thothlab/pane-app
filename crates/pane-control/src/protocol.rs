//! Wire protocol: one JSON value per line, full duplex.
//!
//! Line-delimited rather than length-prefixed so the endpoint stays
//! debuggable with `nc -U`, matching how the PAC and heartbeat servers are
//! written. Requests and responses are correlated by `id`, which lets a
//! long-lived event subscription share a connection with ordinary
//! request/response traffic.

use serde::{Deserialize, Serialize};

/// Bumped only on a breaking change. A client that sees a higher number
/// refuses with a clear message instead of failing somewhere inside serde.
pub const PROTOCOL_VERSION: u32 = 1;

/// Client → server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Client-chosen correlation id, echoed on every frame for this request.
    pub id: String,
    /// Dotted operation name — `captures.list`, `rules.set_enabled`, …
    ///
    /// Deliberately not the flat Tauri command names: those are globally
    /// ambiguous (`start`, `stop`, `status`, `current`, `export`, `send`,
    /// `clear`, `remove`) which is tolerable behind `invoke()` but poor as a
    /// public API.
    pub op: String,
    /// Operation arguments. Existing `pane_ipc` arg structs, verbatim.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Server → client. Discriminated by `type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Terminal success for `id`.
    Ok {
        id: String,
        result: serde_json::Value,
    },
    /// Terminal failure for `id`. Carries `ApiError` verbatim, so the CLI can
    /// map `kind` onto an exit code.
    Err {
        id: String,
        error: pane_ipc::ApiError,
    },
    /// One event on a live subscription. Non-terminal: more may follow.
    Event { id: String, event: EventFrame },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFrame {
    pub topic: String,
    pub payload: serde_json::Value,
}

/// Arguments for `events.subscribe`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubscribeArgs {
    /// Topics to receive. Empty means everything.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Captures filter DSL applied to `capture.completed`, evaluated against
    /// the persisted row. Ignored for other topics.
    #[serde(default)]
    pub filter: Option<String>,
    /// `"none"` (raw event) or `"summary"` (attach the full `CaptureDto`).
    ///
    /// `capture.completed` carries only `{id, status, duration_ms,
    /// total_bytes}` — host, method and path were on `capture.started`, a
    /// different event — so an unenriched stream would print bare UUIDs.
    #[serde(default = "default_enrich")]
    pub enrich: String,
}

fn default_enrich() -> String {
    "summary".to_string()
}

/// Result of `events.subscribe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeAck {
    pub subscription: String,
}

/// Result of `ping` — also the readiness probe used to tell a live endpoint
/// from a stale discovery file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pong {
    pub protocol: u32,
    pub app_version: String,
    pub pid: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_frames_are_tagged_by_type() {
        let ok = Response::Ok {
            id: "1".into(),
            result: serde_json::json!({"a": 1}),
        };
        let v = serde_json::to_value(&ok).unwrap();
        assert_eq!(v["type"], "ok");
        assert_eq!(v["id"], "1");

        let err = Response::Err {
            id: "2".into(),
            error: pane_ipc::ApiError {
                kind: pane_ipc::kinds::NOT_FOUND.into(),
                message: "nope".into(),
                details: None,
            },
        };
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v["type"], "err");
        assert_eq!(v["error"]["kind"], "not_found");
    }

    #[test]
    fn request_params_default_to_null() {
        let r: Request = serde_json::from_str(r#"{"id":"1","op":"ping"}"#).unwrap();
        assert_eq!(r.op, "ping");
        assert!(r.params.is_null());
    }

    #[test]
    fn subscribe_defaults_to_enriched() {
        let a: SubscribeArgs = serde_json::from_str("{}").unwrap();
        assert_eq!(a.enrich, "summary");
        assert!(a.topics.is_empty());
    }
}
