//! LAN-reachable setup helper. Boots a tiny HTTP server on a separate port,
//! serves the iOS mobileconfig + Android PEM and instruction pages, and prints
//! a QR code pointing at the landing URL. Tokens are one-shot; the server
//! self-terminates after the first successful download or 15-min timeout.

use anyhow::Result;
use base64::Engine as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use uuid::Uuid;

pub struct SetupSession {
    pub token: String,
    pub url: String,
    pub qr_svg: String,
}

pub struct SetupConfig {
    pub ca_pem: String,
    pub proxy_host: String,
    pub proxy_port: u16,
    pub bind: SocketAddr,
    pub lan_ip: IpAddr,
}

pub async fn start(cfg: SetupConfig) -> Result<SetupSession> {
    let token = Uuid::new_v4().to_string();
    let listener = TcpListener::bind(cfg.bind).await?;
    let port = listener.local_addr()?.port();
    let url = format!("http://{}:{}/setup?t={}", cfg.lan_ip, port, token);

    let qr = qrcode::QrCode::new(url.as_bytes())?;
    let svg = qr.render::<qrcode::render::svg::Color>().build();

    let state = Arc::new(ServerState {
        token: token.clone(),
        ca_pem: cfg.ca_pem,
        proxy_host: cfg.proxy_host,
        proxy_port: cfg.proxy_port,
        done: Mutex::new(false),
    });

    tokio::spawn(async move {
        // Self-terminate after 15 minutes regardless.
        let timeout = tokio::time::sleep(std::time::Duration::from_secs(15 * 60));
        tokio::pin!(timeout);
        loop {
            tokio::select! {
                _ = &mut timeout => {
                    tracing::info!("setup server timing out after 15 minutes");
                    break;
                }
                accept = listener.accept() => {
                    if let Ok((stream, _)) = accept {
                        let state = state.clone();
                        tokio::spawn(async move {
                            let _ = handle_conn(stream, state).await;
                        });
                    }
                }
            }
        }
    });

    Ok(SetupSession {
        token,
        url,
        qr_svg: svg,
    })
}

struct ServerState {
    token: String,
    ca_pem: String,
    proxy_host: String,
    proxy_port: u16,
    done: Mutex<bool>,
}

async fn handle_conn(mut stream: tokio::net::TcpStream, state: Arc<ServerState>) -> Result<()> {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let req = String::from_utf8_lossy(&buf[..n]).to_string();
    let first = req.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let _method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");

    let (path_only, query) = match path.split_once('?') {
        Some((p, q)) => (p, q),
        None => (path, ""),
    };
    let token_ok = query
        .split('&')
        .find_map(|p| p.strip_prefix("t="))
        .map(|t| t == state.token)
        .unwrap_or(false);

    let user_agent = req
        .lines()
        .find_map(|l| l.strip_prefix("User-Agent:").map(str::trim))
        .unwrap_or("");

    let (status, mime, body) = match (path_only, token_ok) {
        ("/setup", true) => {
            let html = landing_html(user_agent, &state);
            ("200 OK", "text/html; charset=utf-8", html.into_bytes())
        }
        ("/setup/ios/profile.mobileconfig", true) => {
            let xml = pane_mobileconfig::build_full_profile(
                &state.ca_pem,
                &state.proxy_host,
                state.proxy_port,
            )?;
            (
                "200 OK",
                "application/x-apple-aspen-config",
                xml.into_bytes(),
            )
        }
        ("/setup/android/ca.pem", true) => (
            "200 OK",
            "application/x-pem-file",
            state.ca_pem.clone().into_bytes(),
        ),
        (_, false) => ("403 Forbidden", "text/plain", b"forbidden".to_vec()),
        _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
    };

    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.write_all(&body).await?;
    *state.done.lock().await = true;
    Ok(())
}

fn landing_html(user_agent: &str, state: &ServerState) -> String {
    let is_ios = user_agent.contains("iPhone") || user_agent.contains("iPad");
    let is_android = user_agent.contains("Android");
    let body = if is_ios {
        format!(
            r#"
<h1>Install Pane profile</h1>
<p>Tap the button, then Settings → General → VPN & Device Management to install.</p>
<p>After install: Settings → General → About → Certificate Trust Settings → toggle on.</p>
<p><a href="/setup/ios/profile.mobileconfig?t={}" class="btn">Install profile</a></p>"#,
            state.token
        )
    } else if is_android {
        format!(
            r#"
<h1>Install Pane CA</h1>
<ol>
  <li><a href="/setup/android/ca.pem?t={}">Download CA (PEM)</a></li>
  <li>Settings → Security → Encryption & Credentials → Install a certificate → CA certificate.</li>
  <li>Wi-Fi → modify network → Proxy: Manual → Host {host}, Port {port}.</li>
</ol>"#,
            state.token,
            host = state.proxy_host,
            port = state.proxy_port,
        )
    } else {
        r#"<h1>Open this URL on your phone</h1>"#.to_string()
    };

    format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width">
<title>Pane setup</title>
<style>
  body{{font-family:-apple-system,system-ui,sans-serif;max-width:600px;margin:2em auto;padding:0 1em;line-height:1.5}}
  .btn{{display:inline-block;background:#111;color:#fff;padding:1em 2em;text-decoration:none;border-radius:8px;margin:1em 0}}
</style></head><body>{body}</body></html>"#
    )
}

pub fn pick_lan_ip() -> IpAddr {
    // Best-effort: open a UDP socket to a public IP and read the local addr.
    // Doesn't actually send a packet; just lets the kernel pick the route.
    use std::net::UdpSocket;
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("1.1.1.1:80")?;
            s.local_addr()
        })
        .map(|a| a.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)))
}

#[allow(dead_code)]
pub fn ca_pem_to_data_url(pem: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(pem);
    format!("data:application/x-pem-file;base64,{b64}")
}
