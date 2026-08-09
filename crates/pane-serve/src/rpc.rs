//! `POST /rpc` — the operation table, over HTTP.
//!
//! One endpoint rather than 48 REST routes. `pane_control::dispatch` already
//! exists to be "one place that names every remotely-callable operation"; this
//! makes the browser its third consumer after the CLI and the MCP server, and a
//! parallel table of routes would only be a second thing to keep in sync.
//!
//! The response is the **bare** result on success and the **bare** `ApiError`
//! on failure, with no envelope. That is deliberate: it makes the HTTP
//! transport byte-compatible with what Tauri's `invoke` resolves and rejects
//! with, so `src/ipc/client.ts` and every call site above it are identical on
//! both platforms.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use serde::Deserialize;

use crate::AppState;

#[derive(Deserialize)]
pub struct RpcRequest {
    op: String,
    #[serde(default)]
    params: serde_json::Value,
}

pub async fn rpc(State(st): State<AppState>, Json(req): Json<RpcRequest>) -> Response {
    match pane_control::dispatch::dispatch(&st.core, &req.op, req.params).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (status_for(&e.kind), Json(e)).into_response(),
    }
}

/// Map an `ApiError` kind onto a status.
///
/// Mirrors `pane_cli::output::exit_code_for_kind` so the two classifications of
/// the same error cannot drift: what the CLI calls a "not found" exit is what
/// this calls a 404. `unknown_op` is the one kind the CLI never sees, since its
/// clap parser rejects a bad verb before it reaches the wire.
fn status_for(kind: &str) -> StatusCode {
    use pane_ipc::kinds as k;
    match kind {
        k::NOT_FOUND | "unknown_op" => StatusCode::NOT_FOUND,
        "bad_params" | "bad_request" | k::FILTER_PARSE | k::INVALID_ADDR => StatusCode::BAD_REQUEST,
        k::PROXY_NOT_RUNNING
        | k::ENGINE_START
        | k::ENGINE_STOP
        | k::TOOLING_MISSING
        | k::ADB
        | k::LOGCAT_SPAWN => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pane_ipc::kinds as k;

    /// The frontend branches on status before it looks at `kind`, so a
    /// misclassification here shows up as an unhandled error in the UI.
    #[test]
    fn kinds_map_onto_the_status_their_exit_code_implies() {
        assert_eq!(status_for(k::NOT_FOUND), StatusCode::NOT_FOUND);
        assert_eq!(status_for("unknown_op"), StatusCode::NOT_FOUND);
        assert_eq!(status_for(k::FILTER_PARSE), StatusCode::BAD_REQUEST);
        assert_eq!(status_for("bad_params"), StatusCode::BAD_REQUEST);
        assert_eq!(status_for(k::PROXY_NOT_RUNNING), StatusCode::CONFLICT);
        assert_eq!(status_for(k::TOOLING_MISSING), StatusCode::CONFLICT);
        assert_eq!(status_for(k::DB), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// A 2xx must never carry an error kind, and a 4xx/5xx must never be
    /// mistaken for success — the transport tells them apart by status alone.
    #[test]
    fn no_kind_maps_to_a_success_status() {
        for kind in [
            k::DB,
            k::IO,
            k::NOT_FOUND,
            k::FILTER_PARSE,
            k::INVALID_ADDR,
            k::ENGINE_START,
            k::ENGINE_STOP,
            k::PROXY_NOT_RUNNING,
            k::NO_CA,
            k::ROTATE_FAILED,
            k::EXPORT_FAILED,
            k::WRITE,
            k::DECODE,
            k::REPLAY_FAILED,
            k::TOOLING_MISSING,
            k::ADB,
            k::IOS_ADD_FAILED,
            k::ANDROID_ADD_FAILED,
            k::REMOVE_FAILED,
            k::LOGCAT_SPAWN,
            "unknown_op",
            "bad_params",
        ] {
            assert!(
                status_for(kind).is_client_error() || status_for(kind).is_server_error(),
                "{kind} mapped to a non-error status"
            );
        }
    }
}
