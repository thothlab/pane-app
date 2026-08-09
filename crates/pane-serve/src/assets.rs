//! The SPA bundle, embedded at build time.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

use crate::AppState;

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

const INDEX: &str = "/index.html";

fn lookup(path: &str) -> Option<(&'static str, &'static [u8])> {
    ASSETS
        .iter()
        .find(|(p, _, _)| *p == path)
        .map(|(_, ct, body)| (*ct, *body))
}

/// Serve an embedded file, falling back to `index.html` for client-side routes.
///
/// `/assets/*` is deliberately excluded from that fallback. Those names are
/// content-hashed, so a miss means the browser is asking for a chunk this build
/// does not contain — usually a stale tab against a restarted server. Answering
/// with `index.html` would turn a clear 404 into a MIME-type error inside the
/// module loader, which is much harder to read.
pub async fn serve(State(st): State<AppState>, uri: Uri) -> Response {
    if !DIST_PRESENT {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "The Pane UI bundle was not built into this binary.\n\
             Run `pnpm build`, then rebuild the CLI.\n",
        )
            .into_response();
    }
    let _ = &st;

    let path = uri.path();
    let path = if path == "/" { INDEX } else { path };

    if let Some((ct, body)) = lookup(path) {
        return ok(ct, body);
    }
    if path.starts_with("/assets/") {
        return (StatusCode::NOT_FOUND, "no such asset\n").into_response();
    }
    match lookup(INDEX) {
        Some((ct, body)) => ok(ct, body),
        None => (StatusCode::NOT_FOUND, "no index.html in the bundle\n").into_response(),
    }
}

/// `no-store`, not a cache header pair.
///
/// The origin is loopback so there is nothing to save, and the bundle changes
/// whenever the binary is rebuilt while the URL — `http://127.0.0.1:8890` —
/// does not. A cached index.html pointing at chunks that no longer exist is
/// exactly the confusing failure the `/assets/` 404 above is guarding against.
fn ok(content_type: &'static str, body: &'static [u8]) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Body::from(body),
    )
        .into_response()
}

/// Whether a UI bundle was embedded. Reported by `/healthz`.
pub fn present() -> bool {
    DIST_PRESENT
}
