//! The Pane UI over local HTTP, for a browser.
//!
//! # Why a separate front end
//!
//! `pane-control` already exposes every operation the UI needs, but over a Unix
//! socket — deliberately, because a socket's 0600 mode is a kernel-enforced
//! same-uid check and needs no token. A browser cannot open one. So this crate
//! is the second transport in front of the same
//! [`pane_control::dispatch::dispatch`] table and the same event bus, speaking
//! TCP instead.
//!
//! Loopback TCP gives up the property the socket had: any local process can
//! connect to `127.0.0.1:8890` regardless of uid. That is what the token in
//! [`auth`] is for, and why it is stored in `control.json` at 0600 — reading it
//! already requires being the same user.
//!
//! # Why not inside `pane-control`
//!
//! `src-tauri` links `pane-control`. Putting axum and an embedded copy of the
//! SPA there would ship both inside the desktop binary, which already carries
//! the bundle as `frontendDist`. A cargo feature would not help: CI builds the
//! workspace with `--all-features`.
//!
//! # Panics
//!
//! The release profile sets `panic = "abort"`, so there is no catch-panic layer
//! that could save a handler — a panic here takes down the proxy and every
//! paired device with it. Handlers in this crate do not `unwrap`, `expect`, or
//! index slices.

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::routing::get;
use axum::Router;
use pane_core::Core;
use tokio::net::TcpListener;

mod assets;
mod auth;
mod rpc;

/// Shared by every handler.
#[derive(Clone)]
pub struct AppState {
    core: Arc<Core>,
    token: Arc<String>,
    /// This server's own origin, e.g. `http://127.0.0.1:8890`. Compared against
    /// the `Origin` header.
    origin: Arc<String>,
}

pub struct ServeConfig {
    /// 0 asks the OS for a free port; the chosen one comes back in [`Bound`].
    pub port: u16,
    /// Fixed token instead of a fresh one. For `pnpm dev` and tests, where a
    /// stable value beats a printed one.
    pub token: Option<String>,
}

/// A listener holding a port, before anything is served on it.
///
/// Binding is split from serving because the caller needs the URL and token
/// *before* `control.json` is written — that file is written once, complete,
/// rather than amended after the fact.
pub struct Bound {
    pub url: String,
    pub token: String,
    pub port: u16,
    listener: TcpListener,
}

/// Aborts the accept loop when dropped, matching `pane_control::ServeHandle`.
pub struct ServeHandle(tokio::task::JoinHandle<()>);

impl Drop for ServeHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Take a port on loopback and mint a token.
///
/// Loopback only, with no option to widen it. Serving the UI on a LAN address
/// would expose an unauthenticated-by-default proxy controller to the network,
/// and the token alone is not enough of a story for that.
pub async fn bind(cfg: ServeConfig) -> Result<Bound> {
    let listener = TcpListener::bind(("127.0.0.1", cfg.port))
        .await
        .with_context(|| format!("binding 127.0.0.1:{}", cfg.port))?;
    let port = listener
        .local_addr()
        .context("reading the bound port")?
        .port();
    let token = cfg
        .token
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Ok(Bound {
        url: format!("http://127.0.0.1:{port}"),
        token,
        port,
        listener,
    })
}

impl Bound {
    /// The URL to open in a browser: the origin plus the one-shot token, which
    /// the auth layer swaps for a cookie and strips from the address bar.
    pub fn entry_url(&self) -> String {
        format!("{}/?t={}", self.url, self.token)
    }

    pub fn serve(self, core: Arc<Core>) -> ServeHandle {
        let state = AppState {
            core,
            token: Arc::new(self.token),
            origin: Arc::new(self.url.clone()),
        };
        let app = router(state);
        ServeHandle(tokio::spawn(async move {
            if let Err(e) = axum::serve(self.listener, app).await {
                tracing::error!(error = %e, "pane serve: HTTP server stopped");
            }
        }))
    }
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/rpc", axum::routing::post(rpc::rpc))
        .fallback(assets::serve)
        // Below the auth layer, so nothing — not even a 404 — is reachable
        // without a token. `guard` exempts /healthz itself.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::guard,
        ))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

/// Liveness plus enough identity for a client to know what it reached.
///
/// Unauthenticated on purpose: it carries no data, and something has to be
/// reachable for "is the server up, and did it ship a UI?" to be answerable
/// without a token in hand.
async fn healthz(
    axum::extract::State(st): axum::extract::State<AppState>,
) -> axum::Json<serde_json::Value> {
    let _ = &st;
    axum::Json(serde_json::json!({
        "pane": true,
        "protocol": pane_control::PROTOCOL_VERSION,
        "app_version": env!("CARGO_PKG_VERSION"),
        "ui": if assets::present() { "embedded" } else { "absent" },
    }))
}
