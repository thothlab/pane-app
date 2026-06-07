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

/// Android package identifiers for the Pane companion APK. The helper
/// runs a tiny Foreground Service that holds a heartbeat socket to
/// Pane on the laptop (via adb-reverse). When that socket dies — Pane
/// closed, USB unplugged — the helper clears the device's http_proxy
/// setting so the user doesn't end up stranded with no internet.
const HELPER_PACKAGE: &str = "tech.thothlab.pane.helper";
const HELPER_LAUNCHER: &str = "tech.thothlab.pane.helper/.LauncherActivity";

pub struct AndroidPlatform {
    /// Path to the bundled `pane-helper.apk`, set once at Tauri setup
    /// time. `OnceLock` so the rest of the program can read it without
    /// holding a lock and so we don't accidentally swap it under a
    /// running pairing flow. When unset (dev runs before CI has built
    /// a real APK, or third-party builds without it), the watchdog
    /// just doesn't get installed — proxy still works, but the
    /// unplug-no-internet protection won't kick in.
    helper_apk: std::sync::OnceLock<PathBuf>,
}

impl Default for AndroidPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl AndroidPlatform {
    pub fn new() -> Self {
        Self {
            helper_apk: std::sync::OnceLock::new(),
        }
    }

    /// Publish the bundled-APK path. Called once during Tauri setup,
    /// after the app handle is available and `resource_dir()` resolves.
    /// Subsequent calls are silently ignored (OnceLock semantics).
    pub fn set_helper_apk(&self, path: PathBuf) {
        let _ = self.helper_apk.set(path);
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

        // Proxy + PAC setup over USB. Three reverses needed:
        //   8888 → the MITM proxy itself (direct http_proxy target)
        //   8889 → the PAC server (returns "PROXY 127.0.0.1:8888")
        //   8890 → the heartbeat server (companion APK pings it)
        //
        // We set BOTH http_proxy and http_proxy_pac:
        //   - http_proxy = "127.0.0.1:8888" — drives OkHttp, Retrofit,
        //     and most native Android HTTP stacks via
        //     ProxySelector.getDefault(). This is what Charles uses and
        //     what Pane used pre-0.1.21. *Required* for MITM to work
        //     with banking apps and most production OkHttp clients —
        //     they read http_proxy but ignore http_proxy_pac.
        //   - http_proxy_pac points at our PAC server. Chrome / WebView
        //     respect it as the "preferred" setting. When USB unplugs,
        //     PAC becomes unreachable → Chrome falls back to DIRECT.
        //     OkHttp doesn't get that benefit (stuck on dead http_proxy
        //     until Pane is restarted or stop() runs), but that's the
        //     unavoidable trade-off — the alternative (PAC-only) means
        //     OkHttp never goes through Pane at all, which is the
        //     regression that landed in 0.1.21 and was missed until now.
        //
        // Ordering: reverses first → helper APK running → then
        // http_proxy. If we set http_proxy before the helper's
        // heartbeat socket can connect, the helper might race ahead
        // and clear what we just wrote. (Watchdog only clears after
        // a real established session breaks, so the actual race
        // window is tiny — but ordering this way costs nothing.)
        if let Err(e) = run("adb", &["-s", serial, "reverse", "tcp:8888", "tcp:8888"]).await {
            tracing::error!(error = %e, serial, "adb reverse 8888 failed — device cannot reach proxy");
            last_error = Some(format!("adb reverse failed: {e}"));
        }
        if let Err(e) = run("adb", &["-s", serial, "reverse", "tcp:8889", "tcp:8889"]).await {
            tracing::warn!(error = %e, serial, "adb reverse 8889 (PAC) failed");
        }
        if let Err(e) = run("adb", &["-s", serial, "reverse", "tcp:8890", "tcp:8890"]).await {
            tracing::warn!(error = %e, serial, "adb reverse 8890 (heartbeat) failed");
        }

        // Best-effort: install + start the companion APK so the
        // watchdog can clear http_proxy on unplug. Errors here are
        // logged but don't fail the pair flow — the proxy still works,
        // the user just gets the old footgun back if they unplug
        // without stopping Pane first.
        if let Err(e) = ensure_helper_running(serial, self.helper_apk.get()).await {
            tracing::warn!(error = %e, serial, "companion helper APK setup failed");
        }

        // Direct http_proxy — primary, what OkHttp reads.
        if let Err(e) = run(
            "adb",
            &[
                "-s",
                serial,
                "shell",
                "settings",
                "put",
                "global",
                "http_proxy",
                "127.0.0.1:8888",
            ],
        )
        .await
        {
            tracing::warn!(error = %e, serial, "setting http_proxy failed");
        }
        // PAC URL — bonus for Chrome/WebView, which fall back to DIRECT
        // on unplug. Most native apps ignore it; harmless if set.
        let _ = run(
            "adb",
            &[
                "-s",
                serial,
                "shell",
                "settings",
                "put",
                "global",
                "http_proxy_pac",
                "http://127.0.0.1:8889/proxy.pac",
            ],
        )
        .await;
        let _ = run(
            "adb",
            &[
                "-s",
                serial,
                "shell",
                "settings",
                "put",
                "global",
                "global_proxy_pac_url",
                "http://127.0.0.1:8889/proxy.pac",
            ],
        )
        .await;

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
        // Clear proxy first so the device gets internet back before we
        // tear down the heartbeat reverse. Order matters: if we tear
        // down 8890 first, the helper APK sees its connection break
        // and *also* tries to clear http_proxy — redundant but not
        // wrong. Clearing here first means the helper sees an
        // already-clean state and doesn't bother.
        let _ = run(
            "adb",
            &[
                "-s",
                serial,
                "shell",
                "settings",
                "put",
                "global",
                "http_proxy",
                ":0",
            ],
        )
        .await;
        let _ = run(
            "adb",
            &[
                "-s",
                serial,
                "shell",
                "settings",
                "delete",
                "global",
                "http_proxy_pac",
            ],
        )
        .await;
        let _ = run(
            "adb",
            &[
                "-s",
                serial,
                "shell",
                "settings",
                "delete",
                "global",
                "global_proxy_pac_url",
            ],
        )
        .await;

        // Stop the helper service so it doesn't sit there in a
        // reconnect loop forever (cheap on battery, but noisy in
        // logcat). force-stop is idempotent. --user 0 to dodge Knox /
        // Secure Folder secondary-user surprises.
        let _ = run(
            "adb",
            &[
                "-s",
                serial,
                "shell",
                "am",
                "force-stop",
                "--user",
                "0",
                HELPER_PACKAGE,
            ],
        )
        .await;

        // Tear down reverses last.
        let _ = run("adb", &["-s", serial, "reverse", "--remove", "tcp:8888"]).await;
        let _ = run("adb", &["-s", serial, "reverse", "--remove", "tcp:8889"]).await;
        let _ = run("adb", &["-s", serial, "reverse", "--remove", "tcp:8890"]).await;
        Ok(())
    }
}

/// Make sure the companion APK is installed, granted
/// WRITE_SECURE_SETTINGS, and the heartbeat service is running.
///
/// Each step is idempotent:
///   - `pm install -r` no-ops on identical APK
///   - `pm grant` no-ops if already granted
///   - `am start` on an already-running activity is a quick re-show
///
/// All operations explicitly target `--user 0` (the primary user).
/// Without this, Samsung devices with Secure Folder / Knox set up a
/// secondary user (often `150`) as the foreground user, and `pm grant`
/// defaults to that user — which adb shell can't access, so the grant
/// fails with "Shell does not have permission to access user 150".
/// Forcing `--user 0` on every command pins us to the primary user and
/// works identically on non-Samsung Android (where 0 is the only user
/// anyway). Discovered empirically on a Galaxy S25 with Secure Folder
/// enabled.
///
/// `apk_path = None` means there's no bundled APK (dev build before CI
/// produced one, or third-party builds). We bail early — proxy still
/// works, watchdog just won't.
async fn ensure_helper_running(serial: &str, apk_path: Option<&PathBuf>) -> Result<()> {
    let apk = apk_path.ok_or_else(|| anyhow!("no helper APK bundled"))?;
    if !apk_is_present(apk) {
        return Err(anyhow!(
            "helper APK at {} is missing or zero-byte placeholder",
            apk.display()
        ));
    }

    run(
        "adb",
        &[
            "-s",
            serial,
            "install",
            "-r",
            "--user",
            "0",
            apk.to_str().unwrap(),
        ],
    )
    .await
    .map_err(|e| anyhow!("pm install failed: {e}"))?;

    // WRITE_SECURE_SETTINGS is signature|privileged|development. The
    // `development` bit makes it grantable via `pm grant` over adb —
    // which sticks across reboots, no root required. If this fails the
    // service runs but can't actually clear http_proxy; we log so the
    // failure is debuggable but don't abort, since the rest of the
    // pair still works.
    if let Err(e) = run(
        "adb",
        &[
            "-s",
            serial,
            "shell",
            "pm",
            "grant",
            "--user",
            "0",
            HELPER_PACKAGE,
            "android.permission.WRITE_SECURE_SETTINGS",
        ],
    )
    .await
    {
        tracing::warn!(error = %e, serial, "pm grant WRITE_SECURE_SETTINGS failed — watchdog won't be able to clear http_proxy");
    }

    // Launch via the LauncherActivity (not the service directly) so
    // POST_NOTIFICATIONS gets requested on first run. The activity
    // calls startForegroundService and finishes immediately —
    // no UI flash for the user.
    run(
        "adb",
        &[
            "-s",
            serial,
            "shell",
            "am",
            "start",
            "--user",
            "0",
            "-n",
            HELPER_LAUNCHER,
        ],
    )
    .await
    .map_err(|e| anyhow!("am start failed: {e}"))?;

    Ok(())
}

fn apk_is_present(path: &std::path::Path) -> bool {
    match std::fs::metadata(path) {
        // Treat 0-byte placeholder as "no APK available" — the helper
        // CI hasn't produced one yet. Caller will bail before trying
        // to install garbage.
        Ok(m) => m.len() > 0,
        Err(_) => false,
    }
}

/// Constant: where on the device the CA file lives after `push_ca_file`.
/// Public so the UI can show the path verbatim in the manual-install
/// instructions and copy it to the clipboard.
///
/// /sdcard/Download/ wins for one decisive reason: Samsung's
/// CertInstaller file picker opens there by default. The user taps
/// the file immediately without navigating away. Documents was the
/// second-best (auto-cleanup-safe) but cost the user an extra step
/// of switching directories. Cleanup risk with .pem is low: Samsung
/// Smart Manager targets `.cer` (security-flagged extension) more
/// aggressively than `.pem`, the file is push-fresh on every
/// Re-sync, and the install happens in the same session as the push.
pub const DEVICE_CA_PATH: &str = "/sdcard/Download/pane-ca.pem";

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

    // Sweep stale pane-ca files out of legacy locations from earlier
    // Pane versions: /sdcard/Pane/ (custom folder, invisible to SAF
    // picker) and /sdcard/Documents/ (used by 0.1.32 only). Leaves
    // /sdcard/Download/pane-ca.pem alone — that's where we're about
    // to write. Best-effort — silent no-op if nothing to delete.
    let _ = run(
        "adb",
        &[
            "-s",
            serial,
            "shell",
            "sh",
            "-c",
            "rm -f /sdcard/Pane/pane-ca.* /sdcard/Documents/pane-ca.*",
        ],
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
    let head = run(
        "adb",
        &["-s", serial, "shell", "head", "-1", DEVICE_CA_PATH],
    )
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
            "-s",
            serial,
            "shell",
            "am",
            "broadcast",
            "-a",
            "android.intent.action.MEDIA_SCANNER_SCAN_FILE",
            "-d",
            &format!("file://{DEVICE_CA_PATH}"),
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
    run(
        "adb",
        &["-s", serial, "push", tmp.to_str().unwrap(), &target],
    )
    .await?;
    run("adb", &["-s", serial, "shell", "chmod", "644", &target]).await?;
    let _ = run(
        "adb",
        &[
            "-s",
            serial,
            "shell",
            "chcon",
            "u:object_r:system_file:s0",
            &target,
        ],
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
