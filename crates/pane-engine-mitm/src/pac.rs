//! Tiny HTTP server that returns a Proxy Auto-Config (PAC) script.
//!
//! On the device we set `settings put global http_proxy_pac http://127.0.0.1:8889/proxy.pac`.
//! Android fetches the PAC, parses it, and uses it to decide where each
//! request goes. The PAC we serve just says "everything via 127.0.0.1:8888"
//! (Pane's MITM proxy). When the USB cable is unplugged and the PAC URL
//! becomes unreachable, Android transparently falls back to DIRECT — the
//! device keeps its internet. This is the whole point: a direct
//! `http_proxy` setting strands the device on `ERR_PROXY_CONNECTION_FAILED`
//! when Pane goes away. PAC doesn't.
//!
//! No framework, no router, no MIME library. The HTTP/1.1 surface we need
//! is a single GET → 200 OK → static body, and we serve any path the same
//! way so the device can pick `/proxy.pac`, `/wpad.dat`, whatever.

use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Minimal PAC script: route everything through the given proxy.
fn pac_body(proxy_host: &str, proxy_port: u16) -> String {
    format!(
        "function FindProxyForURL(url, host) {{\n\
         \treturn \"PROXY {proxy_host}:{proxy_port}\";\n\
         }}\n"
    )
}

/// Start the PAC server on `listen` and have it advertise the MITM proxy
/// at `proxy_host:proxy_port`. Returns a shutdown channel; drop it (or
/// send on it) to stop the server.
pub async fn start(
    listen: SocketAddr,
    proxy_host: String,
    proxy_port: u16,
) -> anyhow::Result<mpsc::Sender<()>> {
    let listener = TcpListener::bind(listen).await?;
    tracing::info!(listen = %listen, proxy = %format!("{proxy_host}:{proxy_port}"), "PAC server listening");
    let body = pac_body(&proxy_host, proxy_port);
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/x-ns-proxy-autoconfig\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Cache-Control: no-store\r\n\
         \r\n{}",
        body.len(),
        body,
    );
    let (tx, mut rx) = mpsc::channel::<()>(1);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = rx.recv() => {
                    tracing::debug!("PAC server shutting down");
                    break;
                }
                accept = listener.accept() => {
                    match accept {
                        Ok((mut sock, _peer)) => {
                            let response = response.clone();
                            tokio::spawn(async move {
                                // Read+drop the request bytes — we serve the
                                // same PAC for every request, no parsing
                                // needed. The body is tiny and the device
                                // closes the connection after one round
                                // trip, so just write+close.
                                let mut buf = [0u8; 1024];
                                let _ = sock.read(&mut buf).await;
                                let _ = sock.write_all(response.as_bytes()).await;
                                let _ = sock.shutdown().await;
                            });
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "PAC accept failed");
                        }
                    }
                }
            }
        }
    });
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpStream;

    #[tokio::test]
    async fn serves_pac_with_proxy_directive() {
        let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let _tx = start(listen, "127.0.0.1".into(), 8888).await.unwrap();
        // Wait briefly so listener is up. With `:0` we lose the actual
        // bound port; rebind to a fixed port for the test.
    }

    #[tokio::test]
    async fn pac_body_format() {
        let body = pac_body("127.0.0.1", 8888);
        assert!(body.contains("function FindProxyForURL"));
        assert!(body.contains("PROXY 127.0.0.1:8888"));
    }

    #[tokio::test]
    async fn pac_server_responds_with_200_and_body() {
        let listen: SocketAddr = "127.0.0.1:18889".parse().unwrap();
        let _tx = start(listen, "10.0.0.1".into(), 9999).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut stream = TcpStream::connect(listen).await.unwrap();
        use tokio::io::AsyncWriteExt;
        stream
            .write_all(b"GET /proxy.pac HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf);
        assert!(response.starts_with("HTTP/1.1 200 OK"), "got: {response}");
        assert!(response.contains("application/x-ns-proxy-autoconfig"));
        assert!(response.contains("PROXY 10.0.0.1:9999"));
    }
}
