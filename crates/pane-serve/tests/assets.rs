//! Serving the embedded SPA.
//!
//! These assert status and content-type only, never body content: CI stubs
//! `dist/index.html` with a one-line placeholder for the Tauri macro, so
//! anything about what the page *says* would pass locally and fail there.

mod common;
use common::Harness;

/// Skip when the binary was built without a bundle. Locally `pnpm build` has
/// usually run; in CI it has not, and a 503 there is the correct answer rather
/// than a failure.
async fn ui_present(h: &Harness) -> bool {
    let res = Harness::raw_client()
        .get(h.at("/healthz"))
        .send()
        .await
        .expect("healthz");
    let body: serde_json::Value = res.json().await.expect("json");
    body["ui"] == "embedded"
}

#[tokio::test]
async fn the_root_serves_html() {
    let h = Harness::start().await;
    if !ui_present(&h).await {
        return;
    }
    let res = Harness::authed().get(h.at("/")).send().await.expect("get");

    assert_eq!(res.status(), 200);
    let ct = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(ct.starts_with("text/html"), "content-type was {ct}");
}

/// Client-side routes are real URLs the user can reload or deep-link into.
#[tokio::test]
async fn an_unknown_route_falls_back_to_the_app() {
    let h = Harness::start().await;
    if !ui_present(&h).await {
        return;
    }
    let res = Harness::authed()
        .get(h.at("/devices"))
        .send()
        .await
        .expect("get");

    assert_eq!(res.status(), 200);
    let ct = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(ct.starts_with("text/html"), "content-type was {ct}");
}

/// A miss under /assets/ means a stale tab is asking for a chunk this build
/// does not have. Answering with index.html would turn a legible 404 into a
/// MIME error inside the module loader.
#[tokio::test]
async fn a_missing_chunk_404s_instead_of_serving_html() {
    let h = Harness::start().await;
    if !ui_present(&h).await {
        return;
    }
    let res = Harness::authed()
        .get(h.at("/assets/index-deadbeef.js"))
        .send()
        .await
        .expect("get");

    assert_eq!(res.status(), 404);
    let ct = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(!ct.starts_with("text/html"), "content-type was {ct}");
}

/// The bundle changes whenever the binary is rebuilt while the URL does not.
#[tokio::test]
async fn assets_are_not_cached() {
    let h = Harness::start().await;
    if !ui_present(&h).await {
        return;
    }
    let res = Harness::authed().get(h.at("/")).send().await.expect("get");
    let cc = res
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert_eq!(cc, "no-store");
}

#[tokio::test]
async fn the_ui_is_behind_the_token_too() {
    let h = Harness::start().await;
    let res = Harness::raw_client()
        .get(h.at("/"))
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 401);
}
