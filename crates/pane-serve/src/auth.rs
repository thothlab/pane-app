//! Who is allowed to talk to this server.
//!
//! Three checks, in order, and each is guarding against something different:
//!
//! 1. **`Host`** — must name loopback. This is the DNS-rebinding defence:
//!    `evil.com` can resolve itself to `127.0.0.1` and get a browser to send
//!    requests here from a page it controls, but it cannot make that browser
//!    send `Host: 127.0.0.1`.
//! 2. **`Origin`** — when present, must be our own. Same-origin means the app
//!    never sets it cross-origin, so anything that does is not the app.
//! 3. **Token** — proves the request comes from someone who could read
//!    `control.json` (mode 0600). This is what replaces the Unix socket's
//!    kernel-enforced uid check, because any local process can reach a loopback
//!    port.
//!
//! The token lives in a cookie rather than `localStorage` + a header, because
//! `EventSource` cannot set headers and neither can `<a download>` or
//! `window.open`. One cookie covers `fetch`, SSE, downloads and the logcat tab
//! identically. `HttpOnly` keeps an XSS in the SPA from reading it back out.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::AppState;

/// One year. The server mints a fresh token per run, so an old cookie stops
/// working on restart regardless — the max-age only decides how long a browser
/// bothers to keep sending one that may already be dead.
const COOKIE_MAX_AGE: u32 = 31_536_000;
const COOKIE_NAME: &str = "pane_token";

pub async fn guard(State(st): State<AppState>, req: Request<Body>, next: Next) -> Response {
    if !host_is_loopback(req.headers().get(header::HOST)) {
        return deny(
            StatusCode::FORBIDDEN,
            "forbidden_host",
            "this server only answers to a loopback Host header",
        );
    }

    if let Some(origin) = req.headers().get(header::ORIGIN) {
        if origin.as_bytes() != st.origin.as_bytes() {
            return deny(
                StatusCode::FORBIDDEN,
                "forbidden_origin",
                "cross-origin requests are not accepted",
            );
        }
    }

    // Liveness has to be answerable without a token in hand.
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }

    // The token arrived in the URL: swap it for a cookie and bounce to the
    // clean path. Stripping it keeps the token out of the address bar, out of
    // screenshots, and out of any `Referer` this page later sends.
    if let Some(t) = query_token(req.uri().query()) {
        if constant_time_eq(t.as_bytes(), st.token.as_bytes()) {
            return redirect_with_cookie(req.uri().path(), req.uri().query(), &st.token);
        }
        return unauthorized();
    }

    let presented = cookie_token(req.headers().get(header::COOKIE))
        .or_else(|| bearer_token(req.headers().get(header::AUTHORIZATION)));

    match presented {
        Some(t) if constant_time_eq(t.as_bytes(), st.token.as_bytes()) => next.run(req).await,
        _ => unauthorized(),
    }
}

fn unauthorized() -> Response {
    deny(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "open the URL printed by `pane serve`, which carries the token",
    )
}

/// Errors are shaped like every other error the frontend sees, so the transport
/// layer needs no special case to parse them.
fn deny(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({
            "kind": kind,
            "message": message,
            "details": null,
        })),
    )
        .into_response()
}

fn redirect_with_cookie(path: &str, query: Option<&str>, token: &str) -> Response {
    let rest = strip_token(query);
    let location = match rest {
        Some(q) => format!("{path}?{q}"),
        None => path.to_string(),
    };
    let cookie = format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={COOKIE_MAX_AGE}"
    );

    // No `Secure`: the origin is http:// on loopback, and a Secure cookie would
    // simply never be stored.
    let mut resp = (StatusCode::FOUND, "").into_response();
    let headers = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&location) {
        headers.insert(header::LOCATION, v);
    }
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        headers.insert(header::SET_COOKIE, v);
    }
    resp
}

/// `Host` names loopback, ignoring the port.
///
/// A missing header fails closed: HTTP/1.1 requires it, so its absence is not
/// something a browser does.
fn host_is_loopback(host: Option<&HeaderValue>) -> bool {
    let Some(raw) = host.and_then(|h| h.to_str().ok()) else {
        return false;
    };
    // `[::1]:8890` — the brackets exist precisely so the colons inside the
    // address are not confused with the port separator.
    let name = if let Some(rest) = raw.strip_prefix('[') {
        match rest.split_once(']') {
            Some((addr, _)) => addr,
            None => return false,
        }
    } else {
        raw.split(':').next().unwrap_or("")
    };
    matches!(name, "127.0.0.1" | "localhost" | "::1")
}

fn query_token(query: Option<&str>) -> Option<String> {
    param(query, "t")
}

fn param(query: Option<&str>, key: &str) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

/// The query with `t` removed. `None` when nothing else was in it.
fn strip_token(query: Option<&str>) -> Option<String> {
    let q = query?;
    let kept: Vec<&str> = q
        .split('&')
        .filter(|pair| !(*pair == "t" || pair.starts_with("t=")))
        .filter(|pair| !pair.is_empty())
        .collect();
    (!kept.is_empty()).then(|| kept.join("&"))
}

fn cookie_token(cookie: Option<&HeaderValue>) -> Option<String> {
    let raw = cookie?.to_str().ok()?;
    raw.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == COOKIE_NAME).then(|| v.to_string())
    })
}

fn bearer_token(auth: Option<&HeaderValue>) -> Option<String> {
    let raw = auth?.to_str().ok()?;
    raw.strip_prefix("Bearer ").map(|t| t.trim().to_string())
}

/// Compare without leaking where the first difference is.
///
/// The length check short-circuits, which is fine: the token is a v4 UUID, so
/// its length is public. Hand-rolled rather than pulling in `subtle` — this is
/// ten lines and adding a dependency to `Cargo.lock` is not free here.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hv(s: &str) -> HeaderValue {
        HeaderValue::from_str(s).expect("header")
    }

    #[test]
    fn loopback_hosts_are_accepted_with_or_without_a_port() {
        for h in [
            "127.0.0.1",
            "127.0.0.1:8890",
            "localhost",
            "localhost:8890",
            "[::1]",
            "[::1]:8890",
        ] {
            assert!(host_is_loopback(Some(&hv(h))), "{h} should be loopback");
        }
    }

    /// The DNS-rebinding case: the address resolves to us, the name does not.
    #[test]
    fn a_rebound_name_is_rejected() {
        for h in [
            "evil.com",
            "evil.com:8890",
            "127.0.0.1.nip.io",
            "notlocalhost",
            "192.168.1.5:8890",
        ] {
            assert!(!host_is_loopback(Some(&hv(h))), "{h} should be rejected");
        }
    }

    #[test]
    fn a_missing_host_header_fails_closed() {
        assert!(!host_is_loopback(None));
    }

    #[test]
    fn the_token_comes_out_of_the_query() {
        assert_eq!(query_token(Some("t=abc")).as_deref(), Some("abc"));
        assert_eq!(query_token(Some("logcat=1&t=abc")).as_deref(), Some("abc"));
        assert_eq!(query_token(Some("logcat=1")), None);
        assert_eq!(query_token(None), None);
    }

    /// The logcat window opens at `/?logcat=1&serial=X&t=…`; the redirect must
    /// keep everything except the token.
    #[test]
    fn stripping_the_token_preserves_the_rest_of_the_query() {
        assert_eq!(strip_token(Some("t=abc")), None);
        assert_eq!(
            strip_token(Some("logcat=1&t=abc&serial=R5CT")).as_deref(),
            Some("logcat=1&serial=R5CT")
        );
        assert_eq!(
            strip_token(Some("t=abc&logcat=1")).as_deref(),
            Some("logcat=1")
        );
        assert_eq!(strip_token(None), None);
    }

    /// A key merely starting with `t` is not the token.
    #[test]
    fn stripping_leaves_similarly_named_keys_alone() {
        assert_eq!(
            strip_token(Some("topic=x&t=abc&tail=1")).as_deref(),
            Some("topic=x&tail=1")
        );
    }

    #[test]
    fn the_cookie_is_found_among_others() {
        assert_eq!(
            cookie_token(Some(&hv("theme=dark; pane_token=abc; x=1"))).as_deref(),
            Some("abc")
        );
        assert_eq!(cookie_token(Some(&hv("theme=dark"))), None);
        assert_eq!(cookie_token(None), None);
    }

    #[test]
    fn bearer_is_accepted_for_curl_and_tests() {
        assert_eq!(
            bearer_token(Some(&hv("Bearer abc"))).as_deref(),
            Some("abc")
        );
        assert_eq!(bearer_token(Some(&hv("Basic abc"))), None);
    }

    #[test]
    fn constant_time_eq_still_compares_correctly() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }
}
