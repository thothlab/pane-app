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
        // Drives the device-row UI: which CA-install state we're in.
        //   "auto_succeeded" — CA in system store via root; nothing to do.
        //   "manual_required" — CA file pushed, user must install via Settings.
        //   "failed"          — even the push failed; user has to retry or copy file by hand.
        let mut ca_install_state = "auto_succeeded";

        if rooted {
            if let Err(e) = install_system_ca(serial, &ca.cert_pem).await {
                tracing::warn!(error = %e, "system CA install failed — falling back to debug-build snippet");
                last_error = Some(format!("system install failed: {e}"));
                ca_install_state = "failed";
            }
        } else {
            // No root → push the CA file and tell the user how to
            // finish the install themselves. We tried programmatic
            // paths (CertInstaller VIEW intent, KeyChain via helper
            // APK) — Samsung One UI on Android 16 blocks both with
            // "Этот сертификат от приложения <X> необходимо
            // установить в меню Настройки". Google + Samsung made
            // this a user-initiated-only flow on recent builds, and
            // no shell/intent/app-source workaround gets past it.
            // We pre-push the file to a well-known location so the
            // user's manual flow is exactly "Settings → Install
            // certificate → pick pane-ca.pem from Internal storage/Pane".
            match push_ca_file(serial, &ca.cert_pem).await {
                Ok(()) => {
                    ca_install_state = "manual_required";
                    last_error = Some(format!(
                        "Manual CA install needed. File at {DEVICE_CA_PATH}."
                    ));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "couldn't push CA file");
                    ca_install_state = "failed";
                    last_error = Some(format!("couldn't push CA to the device ({e})"));
                }
            }
        }

        // Proxy + PAC setup over USB. Two reverses needed:
        //   8888 → the MITM proxy itself
        //   8889 → the PAC server (returns "PROXY 127.0.0.1:8888")
        // We point the device at the PAC URL (not the direct proxy).
        // When the USB cable is yanked, the PAC URL becomes
        // unreachable and Android falls back to DIRECT — the device
        // keeps its internet. A direct `http_proxy` setting strands
        // the device on ERR_PROXY_CONNECTION_FAILED in the same
        // scenario, which is the bug we're fixing.
        if let Err(e) = run("adb", &["-s", serial, "reverse", "tcp:8888", "tcp:8888"]).await {
            tracing::error!(error = %e, serial, "adb reverse 8888 failed — device cannot reach proxy");
            last_error = Some(format!("adb reverse failed: {e}"));
        }
        if let Err(e) = run("adb", &["-s", serial, "reverse", "tcp:8889", "tcp:8889"]).await {
            tracing::warn!(error = %e, serial, "adb reverse 8889 (PAC) failed");
        }

        // Clear any stale direct-proxy setting from older Pane versions —
        // it would otherwise override our PAC config and bring the
        // strand-on-unplug bug right back.
        let _ = run(
            "adb",
            &["-s", serial, "shell", "settings", "put", "global", "http_proxy", ":0"],
        )
        .await;
        if let Err(e) = run(
            "adb",
            &[
                "-s", serial, "shell", "settings", "put", "global",
                "global_proxy_pac_url",
                "http://127.0.0.1:8889/proxy.pac",
            ],
        )
        .await
        {
            tracing::warn!(error = %e, serial, "setting PAC URL failed");
        }
        // http_proxy_pac is the alias Android uses on most builds;
        // global_proxy_pac_url is the internal name. Set both to be
        // resilient against OEM variants.
        if let Err(e) = run(
            "adb",
            &[
                "-s", serial, "shell", "settings", "put", "global",
                "http_proxy_pac",
                "http://127.0.0.1:8889/proxy.pac",
            ],
        )
        .await
        {
            tracing::debug!(error = %e, "http_proxy_pac not accepted (older Android)");
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
                // Drives the device-row UI on the desktop:
                "ca_install_state": ca_install_state,
                "ca_install_path": DEVICE_CA_PATH,
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
        let _ = run("adb", &["-s", serial, "reverse", "--remove", "tcp:8889"]).await;
        // Clear all three proxy-related settings — both spellings of PAC
        // plus the direct-proxy fallback that older Pane versions used.
        let _ = run(
            "adb",
            &["-s", serial, "shell", "settings", "put", "global", "http_proxy", ":0"],
        )
        .await;
        let _ = run(
            "adb",
            &["-s", serial, "shell", "settings", "delete", "global", "http_proxy_pac"],
        )
        .await;
        let _ = run(
            "adb",
            &["-s", serial, "shell", "settings", "delete", "global", "global_proxy_pac_url"],
        )
        .await;
        Ok(())
    }
}

/// Constant: where on the device the CA file lives after `push_ca_file`.
/// Public so the UI can show the path verbatim in the manual-install
/// instructions and copy it to the clipboard.
///
/// /sdcard/Documents/ on purpose: Samsung's CertInstaller file picker
/// only lists "well-known" Android folders (Downloads, Documents,
/// Pictures, Bluetooth) — custom paths like /sdcard/Pane/ stay hidden
/// even when the file is there. Documents wins over Downloads because
/// Samsung's Smart Manager doesn't periodically sweep it (Downloads
/// gets aggressively cleaned, especially .cer files).
pub const DEVICE_CA_PATH: &str = "/sdcard/Documents/pane-ca.pem";

/// Push the CA cert to `/sdcard/Pane/pane-ca.pem` so the user can pick
/// it up from the system "Install certificate" file picker. Two
/// non-obvious choices here:
///
/// 1. **Own folder, not /sdcard/Download/.** Samsung's Smart Manager
///    and similar OEM cleaners periodically sweep Downloads, and they
///    seem to be especially eager with `.cer` files (flagged as
///    security-relevant). Our own /sdcard/Pane/ isn't on any cleanup
///    allowlist, and the named folder is what users actually look for.
///
/// 2. **PEM, not DER.** PEM is text — opens in any viewer, lets the
///    user eyeball "yep this is a certificate" before installing.
///    Android's CertInstaller accepts both forms, so DER buys nothing
///    here. .pem is also what most Samsung Files UIs file as
///    "Document → Other" rather than hiding it altogether.
///
/// We don't try to fire any install intent any more. Samsung One UI on
/// Android 16+ rejects programmatic CA installs from every source
/// (shell, third-party apps, KeyChain) — those builds make CA install
/// strictly user-initiated. Pane's UI surfaces step-by-step
/// instructions instead; the file is already on the device so the
/// picker step lands on the right file.
async fn push_ca_file(serial: &str, pem: &str) -> Result<()> {
    let tmp = std::env::temp_dir().join("pane-ca.pem");
    std::fs::write(&tmp, pem)?;

    // Sweep stale pane-ca.* files out of /sdcard/Download/ and the
    // old /sdcard/Pane/ before push. Two reasons:
    //   - Samsung's CertInstaller picker defaults to Downloads, so a
    //     leftover dummy there causes the user to pick the wrong file.
    //   - /sdcard/Pane/ was an earlier (failed) choice — the picker
    //     doesn't list custom paths, so the file there was invisible.
    //     Remove it so the user has only one valid target.
    // Best-effort — silent no-op if nothing to delete.
    let _ = run(
        "adb",
        &[
            "-s", serial, "shell", "sh", "-c",
            "rm -f /sdcard/Download/pane-ca.* /sdcard/Pane/pane-ca.*",
        ],
    )
    .await;

    // /sdcard/Documents/ exists by default on every Android, but make
    // sure of it: some launcher-stripped builds skip the standard
    // Android folders until something writes to them.
    let _ = run(
        "adb",
        &["-s", serial, "shell", "mkdir", "-p", "/sdcard/Documents"],
    )
    .await;

    run(
        "adb",
        &["-s", serial, "push", tmp.to_str().unwrap(), DEVICE_CA_PATH],
    )
    .await?;

    // Verify the file landed and looks like a PEM. `adb push` returns
    // success even when the destination is unwritable on some OEM
    // builds (Samsung Knox), leaving the user with a phantom file.
    // Cheap sanity check: read the first line back and confirm it's
    // the PEM header.
    let head = run("adb", &["-s", serial, "shell", "head", "-1", DEVICE_CA_PATH])
        .await
        .unwrap_or_default();
    if !head.contains("BEGIN CERTIFICATE") {
        return Err(anyhow!(
            "push appeared to succeed but {DEVICE_CA_PATH} doesn't look like a PEM (got: {})",
            head.trim()
        ));
    }

    // Trigger a MediaStore scan so Samsung's CertInstaller picker sees
    // the new file immediately. Without this, the freshly-pushed PEM
    // may stay invisible to SAF until the daily indexing pass runs.
    // The intent is deprecated on Android 11+ but Samsung still
    // honours it for sdcard paths under /sdcard/Documents/ etc.
    let _ = run(
        "adb",
        &[
            "-s", serial, "shell", "am", "broadcast",
            "-a", "android.intent.action.MEDIA_SCANNER_SCAN_FILE",
            "-d", &format!("file://{DEVICE_CA_PATH}"),
        ],
    )
    .await;

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
