//! Landing page + healthcheck.

use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

/// Inline HTML with CSS in a `<style>` block — the page loads cleanly
/// even when offline mirrors archive it. The app-icon PNG is the only
/// external asset, served from `/logo.png` below. The four download
/// buttons point at `/download/{target}` which then 302s to the actual
/// GitHub Releases asset (configurable via `ARGOS_DOWNLOADS_BASE`).
const LANDING_HTML: &str = include_str!("../../static/landing.html");

/// 1024×1024 master app icon, served at `/logo.png` for the landing
/// header (and anywhere else a brand mark is useful).
const LOGO_PNG: &[u8] = include_bytes!("../../static/logo.png");

/// macOS installer script — fetches the latest dmg, copies the bundle
/// into /Applications, and strips `com.apple.quarantine`. Workaround
/// for unsigned builds until Developer ID + notarization is set up.
const INSTALL_MACOS_SH: &str = include_str!("../../static/install-macos.sh");

/// ASCII-armored GPG public key for `Pane Releases
/// <releases@thothlab.tech>`. Signs the detached `.asc` files next to
/// every Linux artifact. Served at `/pane-gpg.pub` so users can
/// `curl … | gpg --import` before verifying their download.
const PANE_GPG_PUB: &str = include_str!("../../static/pane-gpg.pub");

pub async fn index() -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        LANDING_HTML,
    )
        .into_response()
}

pub async fn logo() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        LOGO_PNG,
    )
        .into_response()
}

pub async fn install_macos() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        INSTALL_MACOS_SH,
    )
        .into_response()
}

pub async fn pane_gpg_pub() -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/pgp-keys; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        PANE_GPG_PUB,
    )
        .into_response()
}

pub async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

#[cfg(test)]
mod tests {
    use crate::test_harness;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    #[tokio::test]
    async fn index_returns_html() {
        let h = test_harness::make().await;
        let res = h
            .router
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let ct = res
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.starts_with("text/html"));
        let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(body.contains("Pane"));
        assert!(body.contains("btn disabled"));
        assert!(body.contains("Apple Silicon"));
    }

    #[tokio::test]
    async fn install_macos_returns_shell_script() {
        let h = test_harness::make().await;
        let res = h
            .router
            .oneshot(
                Request::builder()
                    .uri("/install-macos.sh")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let ct = res
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("shellscript"));
        let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(body.starts_with("#!/usr/bin/env bash"));
        assert!(body.contains("com.apple.quarantine"));
    }

    #[tokio::test]
    async fn healthz_returns_ok() {
        let h = test_harness::make().await;
        let res = h
            .router
            .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
