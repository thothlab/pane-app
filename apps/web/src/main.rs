//! Pane landing page + update manifest + crash collection service.
//!
//! One binary, one container, one job: be the public face of Pane
//! at `pane.thothlab.tech` during closed alpha.
//!
//! Routes:
//!   - `GET  /`                     → landing page (inline HTML)
//!   - `GET  /healthz`              → 200 OK (for Caddy / monit)
//!   - `GET  /docs/*`               → static Astro export
//!   - `GET  /download/{target}`    → 302 → GitHub Releases
//!   - `GET  /install-macos.sh`     → macOS installer script (curl|bash)
//!   - `GET  /api/update/{target}/{current}` → Tauri update manifest
//!   - `POST /api/crash`            → save + dedup crash report
//!
//! Persistent state lives in `$PANE_DATA_DIR` (default
//! `/var/lib/pane-web`) so a container restart doesn't lose crashes
//! or manifest.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod ratelimit;
mod routes;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("pane_web=info,tower_http=info")),
        )
        .with_target(false)
        .init();

    let config = Config::from_env()?;
    info!(
        bind = %config.bind,
        data_dir = %config.data_dir.display(),
        docs_dir = %config.docs_dir.display(),
        "starting pane-web"
    );

    tokio::fs::create_dir_all(config.data_dir.join("crashes")).await?;

    let state = AppState {
        data_dir: config.data_dir.clone(),
        downloads_base: config.downloads_base.clone(),
        rate_limiter: Arc::new(ratelimit::RateLimiter::per_minute(10)),
    };

    let app = build_router(state, &config);

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

#[derive(Clone)]
pub struct AppState {
    pub data_dir: PathBuf,
    pub downloads_base: String,
    pub rate_limiter: Arc<ratelimit::RateLimiter>,
}

#[derive(Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub docs_dir: PathBuf,
    pub downloads_base: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind = env::var("PANE_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8744".to_string())
            .parse()?;
        let data_dir = env::var("PANE_DATA_DIR")
            .unwrap_or_else(|_| "/var/lib/pane-web".to_string())
            .into();
        let docs_dir = env::var("PANE_DOCS_DIR")
            .unwrap_or_else(|_| "/srv/pane-docs".to_string())
            .into();
        let downloads_base = env::var("PANE_DOWNLOADS_BASE")
            .unwrap_or_else(|_| "https://github.com/thothlab/pane-app/releases/latest/download".to_string());
        Ok(Self {
            bind,
            data_dir,
            docs_dir,
            downloads_base,
        })
    }
}

fn build_router(state: AppState, config: &Config) -> Router {
    use axum::routing::{get, post};
    use tower_http::services::ServeDir;
    use tower_http::trace::TraceLayer;

    let api = Router::new()
        .route(
            "/update/:target/:current",
            get(routes::update::manifest_for_target),
        )
        .route("/crash", post(routes::crash::submit));

    Router::new()
        .route("/", get(routes::landing::index))
        .route("/en", get(routes::landing::index_en))
        .route("/en/", get(routes::landing::index_en))
        .route("/logo.png", get(routes::landing::logo))
        .route("/install-macos.sh", get(routes::landing::install_macos))
        .route("/pane-gpg.pub", get(routes::landing::pane_gpg_pub))
        .route("/healthz", get(routes::landing::healthz))
        .route("/download/:target", get(routes::download::redirect))
        .nest("/api", api)
        .nest_service("/docs", ServeDir::new(&config.docs_dir))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("shutdown signal received, stopping");
}

#[cfg(test)]
pub(crate) mod test_harness {
    use super::*;
    use tempfile::TempDir;

    pub struct Harness {
        pub state: AppState,
        pub router: Router,
        pub _tmp: TempDir,
    }

    pub async fn make() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_path_buf();
        let docs_dir = tmp.path().join("docs");
        tokio::fs::create_dir_all(&docs_dir).await.unwrap();
        tokio::fs::create_dir_all(data_dir.join("crashes"))
            .await
            .unwrap();
        let state = AppState {
            data_dir: data_dir.clone(),
            downloads_base: "https://example.test/releases/latest/download".into(),
            rate_limiter: Arc::new(ratelimit::RateLimiter::per_minute(10)),
        };
        let config = Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            data_dir,
            docs_dir,
            downloads_base: state.downloads_base.clone(),
        };
        let router = build_router(state.clone(), &config);
        Harness {
            state,
            router,
            _tmp: tmp,
        }
    }
}
