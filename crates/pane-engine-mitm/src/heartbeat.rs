//! Tiny TCP server that lets the Pane companion APK on the device
//! detect whether Pane is alive.
//!
//! The APK opens a TCP socket to `127.0.0.1:8890` (adb-reverse-forwarded
//! from the device to the laptop), sends `PING\n`, expects `PONG\n`, and
//! repeats every 2 seconds. When this server goes away — because Pane
//! stopped or the user unplugged USB and adb-reverse died — the APK
//! sees the socket break and clears the device's `http_proxy` setting,
//! restoring its internet.
//!
//! Why a dedicated listener and not, say, multiplexing the MITM port:
//! the MITM port speaks HTTP/CONNECT. Probing it with raw `PING\n` would
//! either error or be treated as a malformed request — fine, but the
//! probe would then race against actual proxied traffic, and the APK
//! couldn't tell "Pane is alive" from "Pane sent me a 400 Bad Request".
//! A separate port with a trivial 5-byte protocol keeps the contract
//! unambiguous.
//!
//! Connection lifecycle: the APK keeps one TCP connection open for the
//! lifetime of a session and pings on it repeatedly. We service every
//! PING with a PONG and stay in the read loop. When the client (the
//! APK, or `adb reverse`) drops, the read errors and the per-connection
//! task exits — no global state to clean.

use std::net::SocketAddr;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Start the heartbeat server on `listen`. Returns a shutdown channel;
/// drop it (or send on it) to stop the server.
pub async fn start(listen: SocketAddr) -> anyhow::Result<mpsc::Sender<()>> {
    let listener = TcpListener::bind(listen).await?;
    tracing::info!(listen = %listen, "heartbeat server listening");
    let (tx, mut rx) = mpsc::channel::<()>(1);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = rx.recv() => {
                    tracing::debug!("heartbeat server shutting down");
                    break;
                }
                accept = listener.accept() => {
                    match accept {
                        Ok((sock, peer)) => {
                            tracing::debug!(peer = %peer, "heartbeat client connected");
                            tokio::spawn(async move {
                                if let Err(e) = serve(sock).await {
                                    tracing::debug!(error = %e, "heartbeat connection ended");
                                }
                            });
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "heartbeat accept failed");
                        }
                    }
                }
            }
        }
    });
    Ok(tx)
}

/// Per-connection loop: read PING lines, write PONG lines, until the
/// peer disconnects or sends garbage.
async fn serve(sock: tokio::net::TcpStream) -> anyhow::Result<()> {
    let (rd, mut wr) = sock.into_split();
    let mut reader = BufReader::new(rd);
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(()); // EOF — peer closed cleanly
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed != "PING" {
            // Unknown frame — close. Don't try to recover; the APK only
            // ever sends PING, so anything else is either a bug or a
            // probe from something unrelated.
            anyhow::bail!("unexpected frame: {trimmed:?}");
        }
        wr.write_all(b"PONG\n").await?;
        wr.flush().await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;

    #[tokio::test]
    async fn ping_gets_pong() {
        let listen: SocketAddr = "127.0.0.1:18890".parse().unwrap();
        let _tx = start(listen).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let stream = TcpStream::connect(listen).await.unwrap();
        let (rd, mut wr) = stream.into_split();
        let mut reader = BufReader::new(rd);

        wr.write_all(b"PING\n").await.unwrap();
        wr.flush().await.unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(line.trim(), "PONG");

        // Second round-trip — verifies the connection stays open.
        wr.write_all(b"PING\n").await.unwrap();
        wr.flush().await.unwrap();
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(line.trim(), "PONG");
    }

    #[tokio::test]
    async fn garbage_input_closes_connection() {
        let listen: SocketAddr = "127.0.0.1:18891".parse().unwrap();
        let _tx = start(listen).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let stream = TcpStream::connect(listen).await.unwrap();
        let (rd, mut wr) = stream.into_split();
        let mut reader = BufReader::new(rd);

        wr.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
        wr.flush().await.unwrap();
        let mut line = String::new();
        let n = reader.read_line(&mut line).await.unwrap();
        // Server closes after rejecting the frame, so the next read
        // returns 0 bytes. line stays empty.
        assert_eq!(n, 0);
    }
}
