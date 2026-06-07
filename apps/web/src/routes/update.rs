//! `GET /api/update/{target}/{current}` — Tauri 2 updater manifest.
//!
//! Channel selection: an optional `X-Pane-Channel` request header
//! picks which manifest file to read. `stable` (or no header) reads
//! `manifest.json`; `beta` reads `manifest-beta.json`; `nightly`
//! reads `manifest-nightly.json`. Unknown channel → 400. Missing
//! manifest file for a known channel → 503.
//!
//! Returns the platform slice for `{target}` if the latest version
//! is newer than `{current}`. Same-or-newer current → 204 No Content.

use std::path::Path as StdPath;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::AppState;

/// Header clients send to opt into beta / nightly. Missing or `stable`
/// → standard manifest.json file.
const CHANNEL_HEADER: &str = "X-Pane-Channel";

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestFile {
    /// Latest version published, e.g. `"0.1.1"`.
    pub version: String,
    /// Human-readable release notes (Markdown).
    pub notes: String,
    /// Publication timestamp (RFC 3339).
    pub pub_date: String,
    /// Per-platform signature + URL.
    pub platforms: std::collections::BTreeMap<String, PlatformEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlatformEntry {
    pub signature: String,
    pub url: String,
}

/// Subset of [`ManifestFile`] returned to the Tauri updater — the
/// `platforms` map is narrowed to just the requested target.
#[derive(Debug, Serialize)]
pub struct UpdateResponse<'a> {
    pub version: &'a str,
    pub notes: &'a str,
    pub pub_date: &'a str,
    pub platforms: std::collections::BTreeMap<String, PlatformEntry>,
}

pub async fn manifest_for_target(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((target, current)): Path<(String, String)>,
) -> Response {
    let channel = match resolve_channel(&headers) {
        Ok(c) => c,
        Err(unknown) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("unknown channel `{unknown}`"),
            )
                .into_response();
        }
    };

    let manifest_path = state.data_dir.join(manifest_filename(channel));
    let manifest = match load_manifest(&manifest_path).await {
        Ok(m) => m,
        Err(LoadError::NotFound) => {
            return (StatusCode::SERVICE_UNAVAILABLE, "no manifest published yet").into_response();
        }
        Err(LoadError::Invalid(msg)) => {
            tracing::error!(error = %msg, "manifest.json failed to parse");
            return (StatusCode::INTERNAL_SERVER_ERROR, "manifest unavailable").into_response();
        }
    };

    if version_is_uptodate(&current, &manifest.version) {
        return StatusCode::NO_CONTENT.into_response();
    }

    let Some(platform) = manifest.platforms.get(&target).cloned() else {
        return (
            StatusCode::BAD_REQUEST,
            format!("unknown target `{target}`"),
        )
            .into_response();
    };

    let mut platforms = std::collections::BTreeMap::new();
    platforms.insert(target, platform);

    Json(UpdateResponse {
        version: &manifest.version,
        notes: &manifest.notes,
        pub_date: &manifest.pub_date,
        platforms,
    })
    .into_response()
}

#[derive(Debug)]
enum LoadError {
    NotFound,
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    Stable,
    Beta,
    Nightly,
}

fn resolve_channel(headers: &HeaderMap) -> Result<Channel, String> {
    let Some(value) = headers.get(CHANNEL_HEADER) else {
        return Ok(Channel::Stable);
    };
    let Ok(s) = value.to_str() else {
        return Err("<non-utf8>".into());
    };
    // Case-insensitive — Tauri's header parser may normalise, and the
    // setting comes from JSON anyway.
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "stable" => Ok(Channel::Stable),
        "beta" => Ok(Channel::Beta),
        "nightly" => Ok(Channel::Nightly),
        other => Err(other.to_string()),
    }
}

fn manifest_filename(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "manifest.json",
        Channel::Beta => "manifest-beta.json",
        Channel::Nightly => "manifest-nightly.json",
    }
}

async fn load_manifest(path: &StdPath) -> Result<ManifestFile, LoadError> {
    let bytes = match tokio::fs::read(path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::NotFound),
        Err(e) => return Err(LoadError::Invalid(e.to_string())),
    };
    serde_json::from_slice(&bytes).map_err(|e| LoadError::Invalid(e.to_string()))
}

/// Naïve semver-prefix comparator: `"0.1.0" < "0.1.1"`.
/// Tauri uses real semver under the hood; for our purposes we just
/// need to know "is `current` >= `latest`" with a stable ordering.
fn version_is_uptodate(current: &str, latest: &str) -> bool {
    let parse = |s: &str| -> Option<(u32, u32, u32)> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() < 3 {
            return None;
        }
        Some((
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].split('-').next()?.parse().ok()?,
        ))
    };
    match (parse(current), parse(latest)) {
        (Some(c), Some(l)) => c >= l,
        _ => false, // unparseable → safer to offer the update
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_harness;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    async fn write_manifest_to(h: &test_harness::Harness, filename: &str, version: &str) {
        let mut platforms = std::collections::BTreeMap::new();
        platforms.insert(
            "darwin-aarch64".to_string(),
            PlatformEntry {
                signature: "<sig>".into(),
                url: format!("https://example.test/releases/pane-{version}-arm64.dmg"),
            },
        );
        let m = ManifestFile {
            version: version.into(),
            notes: "Some notes.".into(),
            pub_date: "2026-05-11T10:00:00Z".into(),
            platforms,
        };
        let json = serde_json::to_vec_pretty(&m).unwrap();
        tokio::fs::write(h.state.data_dir.join(filename), json)
            .await
            .unwrap();
    }

    async fn write_manifest(h: &test_harness::Harness, version: &str) {
        write_manifest_to(h, "manifest.json", version).await;
    }

    #[tokio::test]
    async fn returns_204_when_current_matches() {
        let h = test_harness::make().await;
        write_manifest(&h, "0.1.0").await;
        let res = h
            .router
            .oneshot(
                Request::builder()
                    .uri("/api/update/darwin-aarch64/0.1.0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn returns_manifest_when_update_available() {
        let h = test_harness::make().await;
        write_manifest(&h, "0.1.1").await;
        let res = h
            .router
            .oneshot(
                Request::builder()
                    .uri("/api/update/darwin-aarch64/0.1.0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["version"], "0.1.1");
        assert!(v["platforms"]["darwin-aarch64"]["url"]
            .as_str()
            .unwrap()
            .contains("0.1.1"));
        // Other platforms are stripped — only the requested one survives.
        assert_eq!(v["platforms"].as_object().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn returns_400_for_unknown_target() {
        let h = test_harness::make().await;
        write_manifest(&h, "0.1.1").await;
        let res = h
            .router
            .oneshot(
                Request::builder()
                    .uri("/api/update/bsd/0.1.0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn returns_503_when_manifest_missing() {
        let h = test_harness::make().await;
        let res = h
            .router
            .oneshot(
                Request::builder()
                    .uri("/api/update/darwin-aarch64/0.1.0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn channel_header_picks_beta_manifest() {
        let h = test_harness::make().await;
        write_manifest_to(&h, "manifest.json", "0.1.0").await;
        write_manifest_to(&h, "manifest-beta.json", "0.2.0").await;
        let res = h
            .router
            .oneshot(
                Request::builder()
                    .uri("/api/update/darwin-aarch64/0.1.0")
                    .header("X-Pane-Channel", "beta")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["version"], "0.2.0");
    }

    #[tokio::test]
    async fn missing_header_means_stable() {
        let h = test_harness::make().await;
        write_manifest_to(&h, "manifest.json", "0.1.0").await;
        write_manifest_to(&h, "manifest-beta.json", "0.2.0").await;
        let res = h
            .router
            .oneshot(
                Request::builder()
                    .uri("/api/update/darwin-aarch64/0.0.9")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["version"], "0.1.0");
    }

    #[tokio::test]
    async fn unknown_channel_returns_400() {
        let h = test_harness::make().await;
        let res = h
            .router
            .oneshot(
                Request::builder()
                    .uri("/api/update/darwin-aarch64/0.1.0")
                    .header("X-Pane-Channel", "experimental")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn beta_channel_without_manifest_returns_503() {
        let h = test_harness::make().await;
        write_manifest_to(&h, "manifest.json", "0.1.0").await;
        // No manifest-beta.json on disk.
        let res = h
            .router
            .oneshot(
                Request::builder()
                    .uri("/api/update/darwin-aarch64/0.0.9")
                    .header("X-Pane-Channel", "beta")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn version_compare_handles_prerelease_suffix() {
        assert!(version_is_uptodate("0.1.0", "0.1.0"));
        assert!(!version_is_uptodate("0.1.0", "0.1.1"));
        assert!(version_is_uptodate("0.2.0", "0.1.5"));
        // Pre-release suffix on current → strip and compare.
        assert!(version_is_uptodate("0.1.0-rc1", "0.1.0"));
    }
}
