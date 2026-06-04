//! Android USB integration via bundled `adb`.
//!
//! Two paths:
//!  - Rooted device: push CA into `/system/etc/security/cacerts/<hash>.0`
//!    (Android subject_hash_old format) so the OS trusts our root globally.
//!  - Non-rooted: generate a `network_security_config.xml` snippet that the
//!    user pastes into their debug build, plus copy the PEM to the clipboard.
//!
//! Either way we set the device-side HTTP proxy and `adb reverse` so `localhost:8888`
//! on the device reaches the desktop proxy without touching Wi-Fi config.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use pane_ca::CaMaterial;
use pane_ipc::{AndroidToolingStatusDto, DeviceDto, DiscoveredDeviceDto};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use uuid::Uuid;

/// Single source of truth for the "where to look for adb" failure message.
/// Surfaced in the UI verbatim, so phrase it as an instruction, not a log line.
const ADB_NOT_FOUND_MSG: &str = "adb not found. Install Android platform-tools \
    (https://developer.android.com/tools/releases/platform-tools) and either add it to PATH, \
    set ANDROID_HOME, or install at the default Android SDK location.";

pub struct AndroidPlatform;

impl AndroidPlatform {
    pub fn new() -> Self {
        Self
    }

    pub async fn discover(&self) -> Result<Vec<DiscoveredDeviceDto>> {
        let out = run("adb", &["devices", "-l"]).await?;
        let mut devices = Vec::new();
        for line in out.lines().skip(1).filter(|l| !l.trim().is_empty()) {
            // Format: <serial> device usb:... product:... model:... device:...
            let mut parts = line.split_whitespace();
            let serial = match parts.next() {
                Some(s) => s,
                None => continue,
            };
            let status = parts.next().unwrap_or("");
            if status != "device" {
                continue;
            }
            let model = parts
                .find_map(|p| p.strip_prefix("model:"))
                .unwrap_or("Android device");
            devices.push(DiscoveredDeviceDto {
                platform: "android".into(),
                serial: serial.to_string(),
                name: model.to_string(),
            });
        }
        Ok(devices)
    }

    pub async fn add_usb(&self, serial: &str, ca: &CaMaterial) -> Result<DeviceDto> {
        // Probe root + version.
        let rooted = run("adb", &["-s", serial, "shell", "which", "su"])
            .await
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let android_release = run(
            "adb",
            &["-s", serial, "shell", "getprop", "ro.build.version.release"],
        )
        .await
        .unwrap_or_else(|_| "unknown".into())
        .trim()
        .to_string();
        let manufacturer = run(
            "adb",
            &["-s", serial, "shell", "getprop", "ro.product.manufacturer"],
        )
        .await
        .unwrap_or_else(|_| "unknown".into())
        .trim()
        .to_string();

        let mut last_error: Option<String> = None;

        if rooted {
            if let Err(e) = install_system_ca(serial, &ca.cert_pem).await {
                tracing::warn!(error = %e, "system CA install failed — falling back to debug-build snippet");
                last_error = Some(format!("system install failed: {e}"));
            }
        } else {
            // No root → push the CA to /sdcard/Download and fire the system
            // "Install certificate" VIEW intent. CertInstaller prompts the
            // user to name it + enter the lockscreen PIN. Once accepted, any
            // app whose network_security_config trusts user CAs — which
            // includes most debug builds with `<debug-overrides>` — will
            // start trusting Pane. This is the same flow Charles uses; Pane
            // previously skipped it entirely and showed only a warning.
            match install_user_ca(serial, &ca.cert_pem).await {
                Ok(()) => {
                    last_error = Some(
                        "On the device: tap 'Install anyway' on the warning \
                         screen, then in the file picker open Downloads and \
                         select 'pane-ca.cer'. Enter your screen-lock PIN to \
                         confirm. Apps that trust user CAs (debug builds, or \
                         release builds opted in via network_security_config) \
                         will then accept Pane. Release builds with SSL \
                         pinning need extra bypass."
                            .into(),
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "user CA install dialog failed");
                    last_error = Some(format!(
                        "couldn't open the cert install dialog ({e}). \
                         Install the CA manually: Settings → Security → \
                         Install from storage → pick /sdcard/Download/pane-ca.cer."
                    ));
                }
            }
        }

        // Always set up proxy redirect (works regardless of trust path).
        // `adb reverse` is the only thing that makes 127.0.0.1:8888 on the
        // device actually reach Pane on the host. If it fails (e.g. adb not
        // on PATH for the GUI process), the device sees a connection refused
        // and nothing works — surface that loudly instead of silently OK-ing.
        if let Err(e) = run("adb", &["-s", serial, "reverse", "tcp:8888", "tcp:8888"]).await {
            tracing::error!(error = %e, serial, "adb reverse failed — device cannot reach proxy");
            last_error = Some(format!("adb reverse failed: {e}"));
        }
        if let Err(e) = run(
            "adb",
            &["-s", serial, "shell", "settings", "put", "global", "http_proxy", "127.0.0.1:8888"],
        )
        .await
        {
            tracing::warn!(error = %e, serial, "setting global http_proxy failed");
            // Many apps ignore the global proxy setting anyway — not fatal.
        }

        Ok(DeviceDto {
            id: Uuid::new_v4(),
            platform: "android".into(),
            connection: "usb".into(),
            serial: serial.to_string(),
            display_name: format!("{manufacturer} (Android {android_release})"),
            state: "ready".into(),
            ca_installed_at: Some(time::OffsetDateTime::now_utc().to_string()),
            capabilities: serde_json::json!({
                "rooted": rooted,
                "android_release": android_release,
                "manufacturer": manufacturer,
            }),
            last_error,
        })
    }

    /// Best-effort probe for whether we can talk to `adb` at all. Used by the UI
    /// to show a clear "install platform-tools" banner instead of just an empty
    /// "no devices detected" list when the real problem is missing tooling.
    pub fn tooling_status(&self) -> AndroidToolingStatusDto {
        match resolve_adb() {
            Some(path) => AndroidToolingStatusDto {
                ok: true,
                adb_path: Some(path.to_string_lossy().into_owned()),
                error: None,
            },
            None => AndroidToolingStatusDto {
                ok: false,
                adb_path: None,
                error: Some(ADB_NOT_FOUND_MSG.into()),
            },
        }
    }

    pub async fn remove(&self, serial: &str) -> Result<()> {
        let _ = run("adb", &["-s", serial, "reverse", "--remove", "tcp:8888"]).await;
        let _ = run(
            "adb",
            &["-s", serial, "shell", "settings", "put", "global", "http_proxy", ":0"],
        )
        .await;
        Ok(())
    }
}

/// Push the CA cert (DER, .cer) to `/sdcard/Download/` and open the
/// system "Install CA certificate" warning screen on the device.
///
/// On Android 11+ CertInstaller refuses `file://` URIs — it bounces
/// everything through the SAF DocumentsUI picker as a side-effect of
/// scoped storage. So we can't hand the file directly; the best we can
/// do without a companion app + FileProvider is jump straight to the
/// `InstallCaCertificateWarning` activity. That cuts the user flow to
/// two taps: "Install anyway" → pick `pane-ca.cer` from Downloads.
///
/// If the direct-activity launch fails (some heavily customized OEM
/// builds rename or guard the class), we fall back to opening the
/// Security settings root and let the user navigate the standard
/// Encryption & credentials → Install certificate path.
async fn install_user_ca(serial: &str, pem: &str) -> Result<()> {
    let der = pem_to_der(pem)?;
    let tmp = std::env::temp_dir().join("pane-ca.cer");
    std::fs::write(&tmp, der)?;
    let device_path = "/sdcard/Download/pane-ca.cer";
    run(
        "adb",
        &["-s", serial, "push", tmp.to_str().unwrap(), device_path],
    )
    .await?;
    // Try the direct path first.
    let direct = run(
        "adb",
        &[
            "-s", serial, "shell", "am", "start",
            "-n", "com.android.settings/.security.InstallCaCertificateWarning",
        ],
    )
    .await;
    if direct.is_err() {
        run(
            "adb",
            &[
                "-s", serial, "shell", "am", "start",
                "-a", "android.settings.SECURITY_SETTINGS",
            ],
        )
        .await?;
    }
    Ok(())
}

async fn install_system_ca(serial: &str, pem: &str) -> Result<()> {
    let hash = subject_hash_old(pem)?;
    // Write a temp file we can push.
    let tmp = std::env::temp_dir().join(format!("{hash}.0"));
    std::fs::write(&tmp, pem)?;
    let target = format!("/system/etc/security/cacerts/{hash}.0");

    run("adb", &["-s", serial, "root"]).await?;
    run("adb", &["-s", serial, "wait-for-device"]).await?;
    run("adb", &["-s", serial, "remount"]).await?;
    run("adb", &["-s", serial, "push", tmp.to_str().unwrap(), &target]).await?;
    run("adb", &["-s", serial, "shell", "chmod", "644", &target]).await?;
    let _ = run(
        "adb",
        &["-s", serial, "shell", "chcon", "u:object_r:system_file:s0", &target],
    )
    .await;
    Ok(())
}

/// Generate Android's `subject_hash_old` value (8 hex chars) for a PEM cert.
/// Simplified version: uses sha256 of the DER and truncates. Real Android uses
/// MD5 of the OpenSSL canonical-encoded subject; we approximate for now and
/// note this as a follow-up. CA installs that depend on exact match should be
/// regenerated after upgrading this routine.
pub fn subject_hash_old(pem: &str) -> Result<String> {
    let der = pem_to_der(pem)?;
    let mut hasher = Sha256::new();
    hasher.update(&der);
    let h = hasher.finalize();
    Ok(hex::encode(&h[..4]))
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>> {
    use base64::Engine as _;
    let payload = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<String>();
    Ok(base64::engine::general_purpose::STANDARD.decode(payload)?)
}

async fn run(bin: &str, args: &[&str]) -> Result<String> {
    let resolved = if bin == "adb" {
        resolve_adb()
            .ok_or_else(|| anyhow!(ADB_NOT_FOUND_MSG))?
            .to_string_lossy()
            .into_owned()
    } else {
        bin.to_string()
    };
    let output = Command::new(&resolved)
        .args(args)
        .output()
        .await
        .map_err(|e| anyhow!("{resolved}: {e}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "{bin} {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Locate `adb` without relying on the inherited shell PATH.
///
/// macOS GUI processes (Finder/Launchpad launches of `.app` bundles) get a
/// minimal PATH — `/usr/bin:/bin:/usr/sbin:/sbin` — so `adb` installed via
/// Homebrew or the Android SDK is invisible to `Command::new("adb")` even
/// though it works fine in a terminal. Same shape on Windows when launched
/// from Explorer. We probe the well-known install locations explicitly,
/// then fall through to PATH for completeness.
///
/// Probe order, first hit wins:
///   1. Sidecar next to the current exe (reserved for a future bundled-adb
///      build — cheap to check and matches Tauri's `externalBin` layout).
///   2. `$ANDROID_HOME` / `$ANDROID_SDK_ROOT` + `platform-tools/`.
///   3. OS-default Android SDK install
///      (`~/Library/Android/sdk` on macOS, `~/Android/Sdk` on Linux,
///      `%LOCALAPPDATA%/Android/Sdk` on Windows).
///   4. Common package-manager bin dirs (Homebrew, `/usr/local/bin`,
///      `/usr/bin`).
///   5. Walk `$PATH` ourselves — covers the case where PATH *is* populated
///      (e.g. dev runs from terminal) without relying on the OS PATH lookup
///      which behaves differently across `Command::new` impls.
fn resolve_adb() -> Option<PathBuf> {
    let exe_name = adb_exe_name();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for name in ["adb", "adb.exe"] {
                let p = parent.join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }

    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(root) = std::env::var(var) {
            let p = PathBuf::from(root).join("platform-tools").join(exe_name);
            if p.is_file() {
                return Some(p);
            }
        }
    }

    if let Some(home) = home_dir() {
        let parts: &[&str] = if cfg!(target_os = "macos") {
            &["Library", "Android", "sdk", "platform-tools"]
        } else if cfg!(target_os = "windows") {
            &["AppData", "Local", "Android", "Sdk", "platform-tools"]
        } else {
            &["Android", "Sdk", "platform-tools"]
        };
        let mut p = home;
        for part in parts {
            p.push(part);
        }
        p.push(exe_name);
        if p.is_file() {
            return Some(p);
        }
    }

    let common: &[&str] = if cfg!(target_os = "macos") {
        &["/opt/homebrew/bin/adb", "/usr/local/bin/adb"]
    } else if cfg!(target_os = "windows") {
        &[]
    } else {
        &["/usr/local/bin/adb", "/usr/bin/adb"]
    };
    for p in common {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }

    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let p = dir.join(exe_name);
            if p.is_file() {
                return Some(p);
            }
        }
    }

    None
}

fn adb_exe_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "adb.exe"
    } else {
        "adb"
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

/// Snippet for non-rooted dev-build path.
pub fn network_security_config_snippet() -> &'static str {
    r#"<network-security-config>
  <debug-overrides>
    <trust-anchors>
      <certificates src="@raw/my_charles_ca"/>
      <certificates src="system"/>
    </trust-anchors>
  </debug-overrides>
</network-security-config>
"#
}
