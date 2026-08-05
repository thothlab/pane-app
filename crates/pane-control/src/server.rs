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
use crate::protocol::{EventFrame, Request, Response, SubscribeAck, SubscribeArgs};

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
    pub async fn bind(core: Arc<Core>, kind: InstanceKind) -> Result<(Self, ServeHandle)> {
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
/// Enrichment and filtering happen here rather than in the bus pump, so a slow
/// disk read only delays this subscriber instead of every consumer.
fn spawn_subscription(
    core: Arc<Core>,
    tx: mpsc::Sender<String>,
    id: String,
    args: SubscribeArgs,
) -> tokio::task::JoinHandle<()> {
    let mut rx = core.events.subscribe();
    tokio::spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                // Report the gap instead of tearing the stream down — a
                // consumer would rather know it missed N events than have the
                // connection die mid-run.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let _ = send(
                        &tx,
                        Response::Event {
                            id: id.clone(),
                            event: EventFrame {
                                topic: "stream.lagged".into(),
                                payload: serde_json::json!({ "skipped": n }),
                            },
                        },
                    )
                    .await;
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            if !args.topics.is_empty() && !args.topics.contains(&ev.topic) {
                continue;
            }

            let mut payload = ev.payload.clone();

            // capture.completed carries only {id, status, duration_ms,
            // total_bytes}; host/method/path lived on capture.started, a
            // different event. Re-read the row so one line is one whole
            // capture.
            if ev.topic == "capture.completed" {
                let cap_id = ev
                    .payload
                    .get("id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| uuid::Uuid::parse_str(s).ok());
                if let Some(cap_id) = cap_id {
                    if let Some(filter) = args.filter.as_deref() {
                        match core.storage.capture_matches(cap_id, filter) {
                            Ok(true) => {}
                            Ok(false) => continue,
                            Err(e) => {
                                tracing::debug!(error = %e, "tail filter evaluation failed");
                                continue;
                            }
                        }
                    }
                    if args.enrich == "summary" {
                        if let Ok(cap) = core.capture_get(cap_id).await {
                            if let Ok(v) = serde_json::to_value(&cap) {
                                payload = v;
                            }
                        }
                    }
                } else if args.filter.is_some() {
                    continue;
                }
            }

            if send(
                &tx,
                Response::Event {
                    id: id.clone(),
                    event: EventFrame {
                        topic: ev.topic,
                        payload,
                    },
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
