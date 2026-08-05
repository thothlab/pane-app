//! Client half of the control protocol.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixStream;

use crate::discovery::Discovery;
use crate::protocol::{EventFrame, Request, Response, SubscribeArgs};

/// A connection to a running instance.
pub struct Client {
    writer: BufWriter<tokio::net::unix::OwnedWriteHalf>,
    reader: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    next_id: u64,
}

/// Why connecting failed. The CLI distinguishes these because "nothing is
/// running" is a normal, recoverable state (fall back to reading the database
/// directly) while a version mismatch is not.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("no running Pane instance")]
    NotRunning,
    #[error(
        "this Pane instance speaks control protocol v{their}, and this build understands \
         up to v{ours} — update the CLI"
    )]
    ProtocolTooNew { theirs: u32, ours: u32, their: u32 },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl Client {
    /// Connect to the instance owning `data_dir`, if there is one.
    ///
    /// A discovery file with a dead socket behind it is treated as "not
    /// running" rather than an error: that is exactly the residue a SIGKILLed
    /// instance leaves, and the caller's fallback path handles it. The client
    /// never deletes those files — only the next server start does, under the
    /// instance lock, because deleting them here would race a starting server.
    pub async fn connect(data_dir: &Path) -> Result<Self, ConnectError> {
        let Some(meta) = Discovery::read(data_dir).map_err(ConnectError::Other)? else {
            return Err(ConnectError::NotRunning);
        };
        if !meta.is_compatible() {
            return Err(ConnectError::ProtocolTooNew {
                theirs: meta.protocol,
                ours: crate::protocol::PROTOCOL_VERSION,
                their: meta.protocol,
            });
        }
        let stream = match UnixStream::connect(&meta.endpoint).await {
            Ok(s) => s,
            Err(_) => return Err(ConnectError::NotRunning),
        };
        Ok(Self::from_stream(stream))
    }

    pub fn from_stream(stream: UnixStream) -> Self {
        let (read_half, write_half) = stream.into_split();
        Self {
            writer: BufWriter::new(write_half),
            reader: BufReader::new(read_half).lines(),
            next_id: 1,
        }
    }

    fn take_id(&mut self) -> String {
        let id = self.next_id;
        self.next_id += 1;
        id.to_string()
    }

    /// Issue one request and wait for its terminal frame.
    ///
    /// Event frames for other subscriptions are skipped rather than treated as
    /// a protocol error, since the connection is multiplexed.
    pub async fn call(&mut self, op: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let id = self.take_id();
        self.send(&Request {
            id: id.clone(),
            op: op.to_string(),
            params,
        })
        .await?;

        loop {
            let Some(line) = self.reader.next_line().await? else {
                return Err(anyhow!("connection closed while waiting for `{op}`"));
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Response>(&line)
                .with_context(|| format!("malformed response frame: {line}"))?
            {
                Response::Ok { id: rid, result } if rid == id => return Ok(result),
                Response::Err { id: rid, error } if rid == id => {
                    return Err(anyhow::Error::new(error))
                }
                _ => continue,
            }
        }
    }

    /// Subscribe, then hand each event to `on_event` until it returns
    /// `false`, the stream ends, or the connection drops.
    pub async fn subscribe<F>(&mut self, args: SubscribeArgs, mut on_event: F) -> Result<()>
    where
        F: FnMut(EventFrame) -> bool,
    {
        let id = self.take_id();
        self.send(&Request {
            id: id.clone(),
            op: "events.subscribe".into(),
            params: serde_json::to_value(args)?,
        })
        .await?;

        while let Some(line) = self.reader.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Response>(&line)? {
                Response::Event { id: rid, event } if rid == id => {
                    if !on_event(event) {
                        return Ok(());
                    }
                }
                Response::Err { id: rid, error } if rid == id => {
                    return Err(anyhow::Error::new(error))
                }
                _ => continue,
            }
        }
        Ok(())
    }

    async fn send(&mut self, req: &Request) -> Result<()> {
        let mut line = serde_json::to_string(req)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }
}

/// Is an instance listening for `data_dir`?
pub async fn is_running(data_dir: &Path) -> bool {
    Client::connect(data_dir).await.is_ok()
}
