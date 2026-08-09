//! `POST /rpc` against the real dispatch table.

mod common;
use common::Harness;

#[tokio::test]
async fn ping_reports_the_protocol_and_version() {
    let h = Harness::start().await;
    let (status, body) = h.rpc("ping", serde_json::Value::Null).await;

    assert_eq!(status, 200);
    assert_eq!(body["protocol"], pane_control::PROTOCOL_VERSION);
    assert!(body["app_version"].is_string());
}

/// The result is the bare value, with no `{result: …}` envelope. That is what
/// makes the HTTP transport byte-compatible with what Tauri's `invoke`
/// resolves with, so the frontend needs no per-platform unwrapping.
#[tokio::test]
async fn a_result_is_returned_bare() {
    let h = Harness::start().await;
    let (status, body) = h
        .rpc("captures.list", serde_json::json!({"limit": 10}))
        .await;

    assert_eq!(status, 200);
    assert!(body.is_array(), "expected a bare array, got {body}");
    assert_eq!(body.as_array().map(|a| a.len()), Some(0));
}

/// Likewise an error is the bare `ApiError`, matching what `invoke` rejects
/// with.
#[tokio::test]
async fn an_error_is_returned_bare_with_a_matching_status() {
    let h = Harness::start().await;
    let (status, body) = h.rpc("no.such.op", serde_json::Value::Null).await;

    assert_eq!(status, 404);
    assert_eq!(body["kind"], "unknown_op");
    assert!(body["message"].is_string());
}

#[tokio::test]
async fn a_malformed_filter_is_a_client_error() {
    let h = Harness::start().await;
    // `limit` is required — `ListCapturesArgs` gives it no default — so it has
    // to be present or the request fails at deserialization before the filter
    // is ever looked at.
    let (status, body) = h
        .rpc(
            "captures.list",
            serde_json::json!({"filter": "status:oops", "limit": 100}),
        )
        .await;

    assert_eq!(status, 400);
    assert_eq!(body["kind"], "filter_parse");
}

/// Missing required args are a client error too, distinct from a bad filter.
#[tokio::test]
async fn incomplete_args_are_a_client_error() {
    let h = Harness::start().await;
    let (status, body) = h
        .rpc("captures.list", serde_json::json!({"filter": null}))
        .await;

    assert_eq!(status, 400);
    assert_eq!(body["kind"], "bad_params");
}

/// The drift net.
///
/// `dispatch::OPS` is the contract the CLI, the MCP server and now the browser
/// all share. Walking it here means the wire surface can only shrink by someone
/// deleting an op — never by an HTTP-layer mistake quietly hiding one. Ops are
/// called with null params, so most fail; the assertion is only that the router
/// *recognised* them.
#[tokio::test]
async fn every_dispatch_op_is_reachable_over_http() {
    let h = Harness::start().await;

    for op in pane_control::dispatch::OPS {
        // events.* are owned by the streaming layer, not dispatch: on the
        // socket the connection loop handles them, and over HTTP they are
        // GET /events. dispatch rejects them on purpose.
        if op.starts_with("events.") {
            continue;
        }
        let (status, body) = h.rpc(op, serde_json::Value::Null).await;
        assert_ne!(
            body["kind"], "unknown_op",
            "`{op}` is in OPS but /rpc does not know it (status {status})"
        );
    }
}

#[tokio::test]
async fn get_is_not_allowed_on_rpc() {
    let h = Harness::start().await;
    let res = Harness::authed()
        .get(h.at("/rpc"))
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 405);
}

#[tokio::test]
async fn rpc_needs_a_token_like_everything_else() {
    let h = Harness::start().await;
    let res = Harness::raw_client()
        .post(h.at("/rpc"))
        .json(&serde_json::json!({"op": "ping"}))
        .send()
        .await
        .expect("post");
    assert_eq!(res.status(), 401);
}
