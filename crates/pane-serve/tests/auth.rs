//! The three checks that stand between a local port and control of the proxy.

mod common;
use common::{Harness, TOKEN};

#[tokio::test]
async fn without_a_token_nothing_is_reachable() {
    let h = Harness::start().await;
    let res = Harness::raw_client()
        .get(h.at("/"))
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 401);
    let body: serde_json::Value = res.json().await.expect("json");
    assert_eq!(body["kind"], "unauthorized");
}

#[tokio::test]
async fn a_wrong_token_is_rejected() {
    let h = Harness::start().await;
    let res = Harness::raw_client()
        .get(h.at("/?t=nope"))
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 401);
}

/// The entry hop: token in the URL becomes a cookie, and the URL it redirects
/// to must no longer contain the token — that is what keeps it out of the
/// address bar, screenshots and any later `Referer`.
#[tokio::test]
async fn the_url_token_becomes_a_cookie_and_is_stripped() {
    let h = Harness::start().await;
    let res = Harness::raw_client()
        .get(format!("{}/?t={TOKEN}", h.url))
        .send()
        .await
        .expect("get");

    assert_eq!(res.status(), 302);

    let location = res
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("location");
    assert!(!location.contains(TOKEN), "token leaked into {location}");
    assert!(!location.contains("t="), "token key left in {location}");

    let cookie = res
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .expect("set-cookie");
    assert!(cookie.contains(&format!("pane_token={TOKEN}")));
    assert!(cookie.contains("HttpOnly"), "XSS could read it: {cookie}");
    assert!(cookie.contains("SameSite=Strict"), "{cookie}");
}

/// The logcat window opens at `/?logcat=1&serial=…&t=…`; losing those params on
/// the redirect would land it on the main view instead.
#[tokio::test]
async fn the_redirect_keeps_the_rest_of_the_query() {
    let h = Harness::start().await;
    let res = Harness::raw_client()
        .get(format!("{}/?logcat=1&serial=R5CT&t={TOKEN}", h.url))
        .send()
        .await
        .expect("get");

    let location = res
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("location");
    assert!(location.contains("logcat=1"), "{location}");
    assert!(location.contains("serial=R5CT"), "{location}");
    assert!(!location.contains(TOKEN), "{location}");
}

#[tokio::test]
async fn a_cookie_authenticates_subsequent_requests() {
    let h = Harness::start().await;
    let res = Harness::raw_client()
        .get(h.at("/healthz"))
        .header("cookie", format!("theme=dark; pane_token={TOKEN}"))
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn bearer_authenticates_for_curl_and_tests() {
    let h = Harness::start().await;
    let (status, _) = h.rpc("ping", serde_json::Value::Null).await;
    assert_eq!(status, 200);
}

/// DNS rebinding: an attacker's domain can resolve to 127.0.0.1, but it cannot
/// make the browser send our Host header.
#[tokio::test]
async fn a_rebound_host_header_is_refused() {
    let h = Harness::start().await;
    let res = Harness::authed()
        .get(h.at("/healthz"))
        .header("host", "evil.com")
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 403);
    let body: serde_json::Value = res.json().await.expect("json");
    assert_eq!(body["kind"], "forbidden_host");
}

#[tokio::test]
async fn a_foreign_origin_is_refused() {
    let h = Harness::start().await;
    let res = Harness::authed()
        .get(h.at("/healthz"))
        .header("origin", "http://evil.com")
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 403);
    let body: serde_json::Value = res.json().await.expect("json");
    assert_eq!(body["kind"], "forbidden_origin");
}

#[tokio::test]
async fn our_own_origin_is_accepted() {
    let h = Harness::start().await;
    let res = Harness::authed()
        .get(h.at("/healthz"))
        .header("origin", &h.url)
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 200);
}

/// "Is it up, and did it ship a UI?" has to be answerable without a token.
#[tokio::test]
async fn healthz_needs_no_token() {
    let h = Harness::start().await;
    let res = Harness::raw_client()
        .get(h.at("/healthz"))
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 200);

    let body: serde_json::Value = res.json().await.expect("json");
    assert_eq!(body["pane"], true);
    assert!(body["app_version"].is_string());
    // "embedded" in a real build, "absent" when dist/ was never built. Both are
    // valid; the point is that it says which.
    assert!(matches!(
        body["ui"].as_str(),
        Some("embedded") | Some("absent")
    ));
}
