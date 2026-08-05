//! "Capture this Mac" — trust Pane's CA in the login keychain and point the
//! Mac's own system proxy at the local MITM port so the user's browser/system
//! traffic flows through Pane, selectable on demand from the Captures device
//! dropdown.
//!
//! Moved verbatim from `src-tauri/src/host_proxy.rs` — it never had a Tauri
//! dependency, only a reference to the old `AppState`. Two changes: it takes
//! `&Core`, and `trust_ca` materializes the cert under the core's data
//! directory instead of a hardcoded `ProjectDirs` lookup.
//!
//! macOS-only. On other platforms every entry point is a no-op stub so the
//! Tauri commands (registered unconditionally in `lib.rs`) still compile and
//! return a clean "disabled" status.
//!
//! Safety model (mirrors Charles):
//!   - Trusting the CA is a durable, one-time action; we never untrust it.
//!   - The PROXY is the only thing reverted — on Stop, on app exit, and (for
//!     crash cases where neither runs) by `self_heal_on_start`.
//!   - The user's PRIOR proxy config is snapshotted and restored, not blanket
//!     turned off — they may run their own proxy.
//!
//! All `networksetup` / `security` / `route` calls are synchronous
//! `std::process::Command`: the exit handler runs without a tokio reactor, and
//! these are short-lived CLI invocations, so blocking is correct and avoids the
//! "no reactor running" hazard.

use crate::Core;

/// Snapshot of one proxy channel (web or secure-web) prior to Pane taking it
/// over, so `disable` can put it back exactly.
#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Default)]
pub struct ProxyChannelState {
    pub enabled: bool,
    pub server: String,
    pub port: u16,
}

/// The full prior state captured at `enable` time. Holds the exact service name
/// we modified so `disable` targets it even if the active network service has
/// since changed (Wi-Fi → Ethernet), which would otherwise strand the original
/// service pointing at a dead 127.0.0.1:8888.
#[cfg(target_os = "macos")]
#[derive(Clone, Debug)]
pub struct HostProxySnapshot {
    pub service: String,
    pub web: ProxyChannelState,
    pub secure: ProxyChannelState,
}

/// The local MITM port reserved for Mac-local (host) traffic. Devices use
/// 8891+; 8888 is never in the device pool, so 8888 → "__host__".
pub const HOST_PROXY_PORT: u16 = 8888;

/// Sentinel device_id stamped on the registry for port 8888 so host traffic is
/// attributed and filterable, without a real `device` table row.
pub const HOST_SENTINEL: &str = "__host__";

// ───────────────────────────── macOS implementation ─────────────────────────

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use anyhow::{anyhow, Context, Result};
    use std::process::Command;

    /// Run a command, returning stdout on success or a descriptive Err on a
    /// non-zero exit / spawn failure. Never panics.
    fn run(bin: &str, args: &[&str]) -> Result<String> {
        let out = Command::new(bin)
            .args(args)
            .output()
            .with_context(|| format!("failed to spawn `{bin}`"))?;
        if !out.status.success() {
            return Err(anyhow!(
                "`{bin} {}` exited {}: {}",
                args.join(" "),
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Detect the active network service name (e.g. "Wi-Fi", "Ethernet").
    ///
    /// Strategy: `route get default` gives the default-route interface
    /// (`interface: en0`). `networksetup -listnetworkserviceorder` lists each
    /// service with its device in a `(Hardware Port: …, Device: enX)` line; the
    /// service display name is the line directly above. We map the route
    /// interface to that service. Falls back to the first enabled service.
    pub fn active_service() -> Result<String> {
        let iface = default_interface().ok();
        if let Some(iface) = iface.as_deref() {
            if let Some(svc) = service_for_interface(iface)? {
                return Ok(svc);
            }
        }
        first_enabled_service()
    }

    fn default_interface() -> Result<String> {
        let out = run("route", &["get", "default"])?;
        for line in out.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("interface:") {
                let iface = rest.trim().to_string();
                if !iface.is_empty() {
                    return Ok(iface);
                }
            }
        }
        Err(anyhow!("no default-route interface"))
    }

    /// Parse `-listnetworkserviceorder`, mapping a device (enX) to its service
    /// display name. Each entry looks like:
    ///   (1) Wi-Fi
    ///   (Hardware Port: Wi-Fi, Device: en1)
    /// The service name carries a "(N) " ordinal prefix we strip.
    fn service_for_interface(iface: &str) -> Result<Option<String>> {
        let out = run("networksetup", &["-listnetworkserviceorder"])?;
        let needle = format!("Device: {iface})");
        let lines: Vec<&str> = out.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.contains(&needle) {
                // The display-name line is the one immediately above.
                if i > 0 {
                    if let Some(name) = strip_ordinal(lines[i - 1]) {
                        return Ok(Some(name));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Strip a leading "(N) " ordinal from a service-order line, returning the
    /// trimmed display name. Returns None for lines without the ordinal (e.g.
    /// the hardware-port detail line).
    fn strip_ordinal(line: &str) -> Option<String> {
        let line = line.trim_start();
        let rest = line.strip_prefix('(')?;
        let close = rest.find(')')?;
        // Ensure the inside is a number (ordinal), not "Hardware Port: …".
        let inside = &rest[..close];
        if inside.trim().parse::<u32>().is_err() {
            return None;
        }
        let name = rest[close + 1..].trim();
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    }

    /// First enabled service from `-listallnetworkservices`. The first output
    /// line is an informational header (skipped); disabled services are
    /// `*`-prefixed (skipped).
    fn first_enabled_service() -> Result<String> {
        let out = run("networksetup", &["-listallnetworkservices"])?;
        for line in out.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() || line.starts_with('*') {
                continue;
            }
            return Ok(line.to_string());
        }
        Err(anyhow!("no enabled network service found"))
    }

    /// Parse a `networksetup -getwebproxy`/`-getsecurewebproxy` block:
    ///   Enabled: Yes
    ///   Server: 127.0.0.1
    ///   Port: 8080
    fn parse_proxy_block(out: &str) -> ProxyChannelState {
        let mut st = ProxyChannelState::default();
        for line in out.lines() {
            let line = line.trim();
            if let Some(v) = line.strip_prefix("Enabled:") {
                st.enabled = v.trim().eq_ignore_ascii_case("Yes");
            } else if let Some(v) = line.strip_prefix("Server:") {
                st.server = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Port:") {
                st.port = v.trim().parse().unwrap_or(0);
            }
        }
        st
    }

    fn get_web_proxy(service: &str) -> Result<ProxyChannelState> {
        Ok(parse_proxy_block(&run(
            "networksetup",
            &["-getwebproxy", service],
        )?))
    }

    fn get_secure_web_proxy(service: &str) -> Result<ProxyChannelState> {
        Ok(parse_proxy_block(&run(
            "networksetup",
            &["-getsecurewebproxy", service],
        )?))
    }

    /// SHA-256 fingerprint of the CA cert, normalized to uppercase hex without
    /// separators, for comparison against `security find-certificate -Z`
    /// output (which prints `SHA-256 hash: AABB…`).
    fn normalize_fp(fp: &str) -> String {
        fp.chars()
            .filter(|c| c.is_ascii_hexdigit())
            .flat_map(|c| c.to_uppercase())
            .collect()
    }

    fn login_keychain() -> Result<String> {
        let home = std::env::var("HOME").context("HOME not set")?;
        Ok(format!("{home}/Library/Keychains/login.keychain-db"))
    }

    /// True if a cert with this SHA-256 fingerprint is already in the login
    /// keychain. Keying on the fingerprint (not the CN) is essential: a rotated
    /// CA shares CN "Pane Root CA" but has a different fingerprint, and must be
    /// (re-)added rather than skipped.
    fn ca_already_trusted(sha256_fp: &str) -> bool {
        let keychain = match login_keychain() {
            Ok(k) => k,
            Err(_) => return false,
        };
        let want = normalize_fp(sha256_fp);
        if want.is_empty() {
            return false;
        }
        // -a all matches, -Z prints SHA-1 + SHA-256 hashes.
        let out = match run(
            "security",
            &[
                "find-certificate",
                "-a",
                "-Z",
                "-c",
                "Pane Root CA",
                &keychain,
            ],
        ) {
            Ok(o) => o,
            Err(_) => return false,
        };
        out.lines().any(|line| {
            line.trim()
                .strip_prefix("SHA-256 hash:")
                .map(|h| normalize_fp(h) == want)
                .unwrap_or(false)
        })
    }

    /// Idempotently trust the CA in the login keychain. Materializes the cert
    /// PEM (held in memory by CaStore) to a stable file under the data dir —
    /// `security add-trusted-cert` needs a path. Shows a GUI auth prompt the
    /// first time; that's expected and acceptable.
    fn trust_ca(cert_pem: &str, sha256_fp: &str, dir: &std::path::Path) -> Result<()> {
        if ca_already_trusted(sha256_fp) {
            tracing::info!("host capture: CA already trusted in login keychain");
            return Ok(());
        }
        std::fs::create_dir_all(dir)?;
        // Public cert — no 0600 needed. Stable path so re-enable reuses it.
        let path = dir.join("ca-cert.pem");
        std::fs::write(&path, cert_pem).context("writing CA cert to disk")?;
        let keychain = login_keychain()?;
        let path_str = path.to_string_lossy().into_owned();
        run(
            "security",
            &[
                "add-trusted-cert",
                "-r",
                "trustRoot",
                "-k",
                &keychain,
                &path_str,
            ],
        )?;
        tracing::info!(path = %path.display(), "host capture: CA trusted in login keychain");
        Ok(())
    }

    fn set_web_proxy(service: &str, host: &str, port: u16) -> Result<()> {
        run(
            "networksetup",
            &["-setwebproxy", service, host, &port.to_string()],
        )?;
        run(
            "networksetup",
            &["-setsecurewebproxy", service, host, &port.to_string()],
        )?;
        Ok(())
    }

    /// Restore a single channel from a snapshot, choosing set-proxy vs
    /// turn-off based on the prior enabled flag.
    fn restore_channel(service: &str, kind: ProxyKind, prior: &ProxyChannelState) -> Result<()> {
        if prior.enabled && !prior.server.is_empty() {
            run(
                "networksetup",
                &[
                    kind.set_flag(),
                    service,
                    &prior.server,
                    &prior.port.to_string(),
                ],
            )?;
        } else {
            run("networksetup", &[kind.state_flag(), service, "off"])?;
        }
        Ok(())
    }

    enum ProxyKind {
        Web,
        Secure,
    }
    impl ProxyKind {
        fn set_flag(&self) -> &'static str {
            match self {
                ProxyKind::Web => "-setwebproxy",
                ProxyKind::Secure => "-setsecurewebproxy",
            }
        }
        fn state_flag(&self) -> &'static str {
            match self {
                ProxyKind::Web => "-setwebproxystate",
                ProxyKind::Secure => "-setsecurewebproxystate",
            }
        }
    }

    /// Enable host capture: trust the CA, snapshot & set the system proxy, and
    /// stamp the registry so 8888 → "__host__".
    pub fn enable(state: &Core) -> Result<String> {
        // The proxy must be running first: it's what listens on 8888. Enabling
        // host capture while stopped would point the Mac's system proxy at a
        // dead 127.0.0.1:8888 and strand it offline — and nothing would revert
        // it until the next Start/Stop/quit. Bail before any side effect; the
        // UI surfaces this as "start the proxy first".
        if state.proxy_handle.lock().is_none() {
            return Err(anyhow!("start the proxy before capturing this Mac"));
        }
        let material = state.ca.material();
        let dto = state.ca.current_dto().ok();
        let fp = dto.map(|d| d.sha256_fp).unwrap_or_default();
        trust_ca(&material.cert_pem, &fp, state.data_dir())?;

        let service = active_service()?;

        // Re-enable guard: if a snapshot already exists, the *current* proxy is
        // already Pane's 127.0.0.1:8888. Re-snapshotting would save the dead
        // proxy as "prior", so skip the snapshot and just (re-)apply.
        {
            let mut guard = state.host_proxy.lock();
            if guard.is_none() {
                let web = get_web_proxy(&service)?;
                let secure = get_secure_web_proxy(&service)?;
                *guard = Some(HostProxySnapshot {
                    service: service.clone(),
                    web,
                    secure,
                });
            }
        }

        set_web_proxy(&service, "127.0.0.1", HOST_PROXY_PORT)?;
        state.registry.set_port(HOST_PROXY_PORT, HOST_SENTINEL);
        tracing::info!(service = %service, "host capture enabled");
        Ok(service)
    }

    /// Disable host capture: restore the snapshot's prior proxy on the exact
    /// service we modified, then unstamp the registry. Safe to call when not
    /// enabled (no-op).
    pub fn disable(state: &Core) -> Result<()> {
        let snapshot = state.host_proxy.lock().take();
        let Some(snap) = snapshot else {
            return Ok(());
        };
        // Always clear the sentinel even if restore partially fails.
        state.registry.clear_port(HOST_PROXY_PORT);
        restore_channel(&snap.service, ProxyKind::Web, &snap.web)?;
        restore_channel(&snap.service, ProxyKind::Secure, &snap.secure)?;
        tracing::info!(service = %snap.service, "host capture disabled, prior proxy restored");
        Ok(())
    }

    pub fn status(state: &Core) -> (bool, Option<String>) {
        let guard = state.host_proxy.lock();
        match guard.as_ref() {
            Some(snap) => (true, Some(snap.service.clone())),
            None => (false, None),
        }
    }

    /// At startup, clear a stale self-pointer left by a crash (Tauri can't run
    /// cleanup on SIGKILL). If the active service's web/secure proxy points at
    /// 127.0.0.1:8888, turn both off so the user isn't stranded offline.
    pub fn self_heal_on_start() {
        let service = match active_service() {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(error = %e, "host capture self-heal: no active service");
                return;
            }
        };
        let stale = |st: &ProxyChannelState| {
            st.enabled && st.server == "127.0.0.1" && st.port == HOST_PROXY_PORT
        };
        let web = get_web_proxy(&service).unwrap_or_default();
        let secure = get_secure_web_proxy(&service).unwrap_or_default();
        if stale(&web) || stale(&secure) {
            let _ = run("networksetup", &["-setwebproxystate", &service, "off"]);
            let _ = run(
                "networksetup",
                &["-setsecurewebproxystate", &service, "off"],
            );
            tracing::warn!(
                service = %service,
                "host capture self-heal: cleared stale 127.0.0.1:8888 proxy pointer"
            );
        }
    }
}

// ───────────────────────────── public API (macOS) ───────────────────────────

#[cfg(target_os = "macos")]
pub fn enable(state: &Core) -> anyhow::Result<String> {
    imp::enable(state)
}

#[cfg(target_os = "macos")]
pub fn disable(state: &Core) -> anyhow::Result<()> {
    imp::disable(state)
}

#[cfg(target_os = "macos")]
pub fn status(state: &Core) -> (bool, Option<String>) {
    imp::status(state)
}

#[cfg(target_os = "macos")]
pub fn self_heal_on_start() {
    imp::self_heal_on_start()
}

// ───────────────────────────── stubs (non-macOS) ────────────────────────────

#[cfg(not(target_os = "macos"))]
pub fn enable(_state: &Core) -> anyhow::Result<String> {
    Err(anyhow::anyhow!(
        "capture this Mac is only supported on macOS"
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn disable(_state: &Core) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn status(_state: &Core) -> (bool, Option<String>) {
    (false, None)
}

#[cfg(not(target_os = "macos"))]
pub fn self_heal_on_start() {}
