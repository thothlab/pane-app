//! `GET /download/{target}` → 302 to the configured download base.
//!
//! `target` ∈ `macos-aarch64 | macos-x64 | linux-x64 | windows-x64`.
//! Anything else returns 400.

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::AppState;

pub async fn redirect(State(state): State<AppState>, Path(target): Path<String>) -> Response {
    let Some(asset) = asset_for_target(&target) else {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "unknown target `{target}`. Expected: macos-aarch64, macos-x64, linux-x64, windows-x64."
            ),
        )
            .into_response();
    };
    let url = format!("{}/{}", state.downloads_base.trim_end_matches('/'), asset);
    (StatusCode::FOUND, [(header::LOCATION, url)]).into_response()
}

/// Map a target to the canonical filename in the releases bucket.
///
/// **Version is baked in** — Tauri's default bundle naming includes
/// the version (`Pane_<v>_<arch>.dmg`), and GitHub's
/// `/releases/latest/download/<name>` only accepts the exact filename.
/// When the next release lands, bump these strings (or, follow-up,
/// read the latest manifest.json and use its `url` field — same
/// source of truth as the updater).
pub fn asset_for_target(target: &str) -> Option<&'static str> {
    match target {
        "macos-aarch64" | "darwin-aarch64" => Some("Pane_0.1.0_aarch64.dmg"),
        "linux-x64" | "linux-x86_64" => Some("Pane_0.1.0_amd64.AppImage"),
        "windows-x64" | "windows-x86_64" => Some("Pane_0.1.0_x64-setup.exe"),
        // Intel macOS native build is published on demand via the
        // release-darwin-x86_64 workflow. Until that runs for a given
        // tag, /download/macos-x64 has no asset to point at.
        "macos-x64" | "darwin-x86_64" => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::test_harness;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    #[tokio::test]
    async fn redirects_to_configured_base() {
        let h = test_harness::make().await;
        let res = h
            .router
            .oneshot(
                Request::builder()
                    .uri("/download/macos-aarch64")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        let loc = res.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(
            loc,
            "https://example.test/releases/latest/download/Pane_0.1.0_aarch64.dmg"
        );
    }

    #[tokio::test]
    async fn unknown_target_returns_400() {
        let h = test_harness::make().await;
        let res = h
            .router
            .oneshot(
                Request::builder()
                    .uri("/download/bsd")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }
}
