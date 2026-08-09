//! The control server.
//!
//! Per connection there is exactly **one writer task** owning the write half,
//! fed by an mpsc channel. That single-owner rule is the critical invariant:
//! request handlers and event subscriptions both emit lines concurrently, and
//! anything else would interleave partial JSON. The reader spawns a handler
//! per request, so a slow `captures.export` never blocks a live tail.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use pane_core::Core;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::discovery::{self, Discovery, InstanceKind};
use crate::protocol::{Request, Response, SubscribeAck, SubscribeArgs};

/// Outbound line queue depth per connection.
const WRITE_QUEUE: usize = 1024;

pub struct ControlServer {
    data_dir: PathBuf,
}

impl ControlServer {
    /// Bind the endpoint for `core`'s data directory and serve until the
    /// returned future is dropped.
    ///
    /// The caller must already hold the instance lock — that is what makes it
    /// safe to delete a leftover socket inode here.
    /// `http` is `Some` when this process also serves the browser UI
    /// (`pane serve`). It is passed in rather than registered afterwards so
    /// `control.json` is written exactly once, already complete — a second
    /// write would be a window in which a reader sees an instance with no
    /// endpoint.
    pub async fn bind(
        core: Arc<Core>,
        kind: InstanceKind,
        http: Option<crate::HttpEndpoint>,
    ) -> Result<(Self, ServeHandle)> {
        let data_dir = core.data_dir().to_path_buf();
        discovery::clear_stale(&data_dir);

        let sock_path = Discovery::socket_path_in(&data_dir);
        let listener = bind_endpoint(&sock_path)?;

        Discovery {
            protocol: crate::protocol::PROTOCOL_VERSION,
            pid: std::process::id(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            kind,
            endpoint: sock_path.clone(),
            data_dir: data_dir.clone(),
            started_at: time::OffsetDateTime::now_utc().to_string(),
            http,
        }
        .write(&data_dir)
        .context("writing control metadata")?;

        tracing::info!(endpoint = %sock_path.display(), ?kind, "control endpoint listening");

        let task_core = core.clone();
        let handle = tokio::spawn(async move {
            loop {
                let stream = match listener.accept().await {
                    Ok((s, _)) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "control: accept failed");
                        continue;
                    }
                };
                let conn_core = task_core.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_connection(conn_core, stream).await {
                        tracing::debug!(error = %e, "control: connection ended");
                    }
                });
            }
        });

        Ok((Self { data_dir }, ServeHandle(handle)))
    }

    /// Remove the socket and metadata on clean shutdown.
    pub fn cleanup(&self) {
        discovery::cleanup(&self.data_dir);
    }
}

/// Dropping this aborts the accept loop.
pub struct ServeHandle(tokio::task::JoinHandle<()>);

impl Drop for ServeHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(unix)]
fn bind_endpoint(path: &Path) -> Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt;

    // A unix socket is a file, and `bind` refuses to overwrite one. Nothing
    // removes it when a process dies without running its cleanup — a crash, a
    // SIGKILL, Force Quit — so the *next* launch could never publish an
    // endpoint. The app itself came up fine, which is what made this so
    // confusing: the window was there, and only `pane` and the MCP server were
    // broken, reporting "no running instance" (exit 3) until someone knew to
    // delete the file by hand.
    //
    // Removing it is safe here specifically because `InstanceLock` has already
    // been acquired by the time we bind: that lock is released by the kernel on
    // process death, so holding it proves no other instance owns this data
    // directory, and therefore any socket still sitting here is a leftover. We
    // probe it anyway before unlinking — if something does answer, the lock's
    // guarantee has been violated and deleting the live socket would be the
    // worse outcome.
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => anyhow::bail!(
                "another process is already serving the control socket at {} — \
                 refusing to replace it",
                path.display()
            ),
            Err(_) => {
                tracing::info!(
                    path = %path.display(),
                    "removing a stale control socket left by a previous run"
                );
                std::fs::remove_file(path).with_context(|| {
                    format!("removing stale control socket at {}", path.display())
                })?;
            }
        }
    }

    let listener = tokio::net::UnixListener::bind(path)
        .with_context(|| format!("binding control socket at {}", path.display()))?;
    // Same-user only. On a Unix socket this is kernel-enforced, which is why
    // there is no token: a token would add no boundary a same-uid process
    // could not already cross by reading the token file.
    let mut perm = std::fs::metadata(path)?.permissions();
    perm.set_mode(0o600);
    std::fs::set_permissions(path, perm)?;
    Ok(listener)
}

#[cfg(not(unix))]
fn bind_endpoint(_path: &Path) -> Result<tokio::net::UnixListener> {
    // Windows would use tokio::net::windows::named_pipe here. Deliberately not
    // written blind: CI lints and tests Rust on ubuntu only, so a Windows-only
    // path would never be compiled, let alone run, before release.
    anyhow::bail!("the control endpoint is not implemented on this platform yet")
}

async fn serve_connection(core: Arc<Core>, stream: tokio::net::UnixStream) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let (tx, mut rx) = mpsc::channel::<String>(WRITE_QUEUE);

    // The single writer. Everything that emits a line goes through `tx`.
    let writer = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if write_half.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if write_half.write_all(b"\n").await.is_err() {
                break;
            }
        }
    });

    let mut lines = BufReader::new(read_half).lines();
    // Subscription id → task, so `events.unsubscribe` and connection teardown
    // can both stop them.
    let mut subs: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let _ = send(
                    &tx,
                    Response::Err {
                        id: "-".into(),
                        error: pane_core::api_err("bad_request", e.to_string()),
                    },
                )
                .await;
                continue;
            }
        };

        match req.op.as_str() {
            "events.subscribe" => {
                let args: SubscribeArgs =
                    serde_json::from_value(req.params.clone()).unwrap_or_default();
                let handle = spawn_subscription(core.clone(), tx.clone(), req.id.clone(), args);
                subs.insert(req.id.clone(), handle);
                let _ = send(
                    &tx,
                    Response::Ok {
                        id: req.id.clone(),
                        result: serde_json::to_value(SubscribeAck {
                            subscription: req.id,
                        })
                        .unwrap_or(serde_json::Value::Null),
                    },
                )
                .await;
            }
            "events.unsubscribe" => {
                let target = req
                    .params
                    .get("subscription")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&req.id)
                    .to_string();
                if let Some(h) = subs.remove(&target) {
                    h.abort();
                }
                let _ = send(
                    &tx,
                    Response::Ok {
                        id: req.id,
                        result: serde_json::Value::Null,
                    },
                )
                .await;
            }
            _ => {
                // One task per request so a slow op doesn't head-of-line block
                // the reader (or a concurrent event stream).
                let core = core.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let resp = match crate::dispatch::dispatch(&core, &req.op, req.params).await {
                        Ok(result) => Response::Ok { id: req.id, result },
                        Err(error) => Response::Err { id: req.id, error },
                    };
                    let _ = send(&tx, resp).await;
                });
            }
        }
    }

    for (_, h) in subs {
        h.abort();
    }
    drop(tx);
    let _ = writer.await;
    Ok(())
}

async fn send(tx: &mpsc::Sender<String>, resp: Response) -> Result<()> {
    let line = serde_json::to_string(&resp)?;
    tx.send(line).await.ok();
    Ok(())
}

/// Stream bus events for one subscription.
///
/// What belongs on the stream is decided by [`crate::subscription::shape_event`],
/// which the HTTP/SSE front end in `pane-serve` also calls — this function owns
/// only the socket plumbing. Shaping happens per subscriber rather than in the
/// bus pump, so a slow disk read delays this subscriber alone.
fn spawn_subscription(
    core: Arc<Core>,
    tx: mpsc::Sender<String>,
    id: String,
    args: SubscribeArgs,
) -> tokio::task::JoinHandle<()> {
    let mut rx = core.events.subscribe();
    tokio::spawn(async move {
        loop {
            let frame = match rx.recv().await {
                Ok(ev) => match crate::subscription::shape_event(&core, &args, &ev).await {
                    Some(frame) => frame,
                    None => continue,
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    crate::subscription::lagged_frame(n)
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            if send(
                &tx,
                Response::Event {
                    id: id.clone(),
                    event: frame,
                },
            )
            .await
            .is_err()
            {
                break;
            }
        }
    })
}
