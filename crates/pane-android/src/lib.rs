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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use pane_ca::CaMaterial;
use pane_ipc::{AndroidToolingStatusDto, DeviceDto, DiscoveredDeviceDto};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use uuid::Uuid;

/// Single source of truth for the "where to look for adb" failure message.
/// Surfaced in the UI verbatim, so phrase it as an instruction, not a log line.
pub mod logcat;

pub(crate) const ADB_NOT_FOUND_MSG: &str = "adb not found. Install Android platform-tools \
    (https://developer.android.com/tools/releases/platform-tools) and either add it to PATH, \
    set ANDROID_HOME, or install at the default Android SDK location.";

/// Last six characters of an ADB/USB serial — short enough to read at
/// a glance, long enough to disambiguate the handful of devices a user
/// will realistically plug in at once. Used in the device label so
/// two phones of the same model don't render identically.
fn serial_tail(serial: &str) -> String {
    let n = serial.chars().count();
    if n <= 6 {
        serial.to_string()
    } else {
        serial.chars().skip(n - 6).collect()
    }
}

/// The device-side proxy target. Never varies: the phone always talks to its
/// own `127.0.0.1:8888`, and `adb reverse` forwards that to whichever Mac-side
/// pool port this device was assigned. Used both when writing the setting and
/// when reading it back to check the device is still wired up.
const DEVICE_HTTP_PROXY: &str = "127.0.0.1:8888";

/// Manufacturer / marketing-model / Android-release for a device, in one
/// `adb shell` round-trip (three `getprop`s chained with `;` so the
/// attached-list refresh stays one call per device). Missing props come
/// back as empty strings — `getprop` prints a blank line for an unset
/// key, so the line count is stable.
async fn probe_device_props(serial: &str) -> (String, String, String) {
    let out = run(
        "adb",
        &[
            "-s",
            serial,
            "shell",
            "getprop ro.product.manufacturer; getprop ro.product.model; \
             getprop ro.build.version.release",
        ],
    )
    .await
    .unwrap_or_default();
    let mut lines = out.lines();
    let manufacturer = lines.next().unwrap_or("").trim().to_string();
    let model = lines.next().unwrap_or("").trim().to_string();
    let android_release = lines.next().unwrap_or("").trim().to_string();
    (manufacturer, model, android_release)
}

/// The device-row label, shared by the attached-over-USB list
/// (`discover`) and the paired list (`add_usb`) so the same device reads
/// identically in both: `"<manufacturer> <model> · Android <release> ·
/// <serial-tail>"`. The serial tail disambiguates two devices of the
/// same model. We drop the model only when getprop returned empty (very
/// old devices / privacy shells); manufacturer-only is the last resort.
fn format_device_name(
    manufacturer: &str,
    model: &str,
    android_release: &str,
    serial: &str,
) -> String {
    let head = match (manufacturer.is_empty(), model.is_empty()) {
        (false, false) => format!("{manufacturer} {model}"),
        (false, true) => manufacturer.to_string(),
        (true, false) => model.to_string(),
        // Both getprops came back empty (device paired while unreachable /
        // restricted shell). Without a fallback the head is "" and the name
        // reads " · Android  · <tail>" — a leading-dot ghost row in the UI.
        (true, true) => "Android device".to_string(),
    };
    format!("{head} · Android {android_release} · {}", serial_tail(serial))
}

/// Android package identifiers for the Pane companion APK. The helper
/// runs a tiny Foreground Service that holds a heartbeat socket to
/// Pane on the laptop (via adb-reverse). When that socket dies — Pane
/// closed, USB unplugged — the helper clears the device's http_proxy
/// setting so the user doesn't end up stranded with no internet.
const HELPER_PACKAGE: &str = "tech.thothlab.pane.helper";
const HELPER_LAUNCHER: &str = "tech.thothlab.pane.helper/.LauncherActivity";

/// What the device currently believes about Pane's proxy. Read back from the
/// device rather than assumed, because every failure mode we've hit in the
/// field is "we told the phone something and it didn't stick".
#[derive(Debug, Clone, Copy)]
pub struct DeviceProxyState {
    /// The `http_proxy` global setting points at our `127.0.0.1:8888`.
    pub proxy_set: bool,
    /// `adb reverse` still maps the device-side `tcp:8888` to this machine.
    pub reverse_up: bool,
}

impl DeviceProxyState {
    /// Both halves present — traffic from the device can actually reach us.
    /// Either half missing means a silent blackhole, which is exactly the
    /// state the watchdog exists to notice.
    pub fn is_healthy(&self) -> bool {
        self.proxy_set && self.reverse_up
    }
}

pub struct AndroidPlatform {
    /// Path to the bundled `pane-helper.apk`, set once at Tauri setup
    /// time. `OnceLock` so the rest of the program can read it without
    /// holding a lock and so we don't accidentally swap it under a
    /// running pairing flow. When unset (dev runs before CI has built
    /// a real APK, or third-party builds without it), the watchdog
    /// just doesn't get installed — proxy still works, but the
    /// unplug-no-internet protection won't kick in.
    helper_apk: std::sync::OnceLock<PathBuf>,
    /// One mutex per serial, guarding the whole `add_usb` / `remove`
    /// sequence.
    ///
    /// Three callers race for the same device: the fire-and-forget reapply
    /// spawned by `proxy.start`, the watchdog's own reapply, and the user
    /// clicking Re-sync. Un-serialised, two `add_usb` runs interleave their
    /// `pm install -r` / `pm grant` / `settings put` calls on one device —
    /// which is how a device ends up with the helper half-installed and
    /// `http_proxy` written by the loser of the race. Worse, a reapply
    /// interleaved with a `remove` can leave the proxy set with the reverse
    /// torn down: the phone then has no internet at all.
    ///
    /// The map is guarded by a std mutex (no await inside), the per-serial
    /// guard is a tokio mutex because it's held across the whole adb
    /// sequence. Entries are never evicted — one empty mutex per serial the
    /// user has ever plugged in is not worth reclaiming.
    serial_locks: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// `getprop` results per serial. Device properties can't change while the
    /// device stays connected, but `discover()` runs on every watchdog tick
    /// (5 s) and used to pay a full `adb shell` round-trip per device for
    /// them. That polling is itself a contributor to the transient adb
    /// failures this module keeps tripping over, so cache instead.
    ///
    /// Only non-empty probes are cached: a device probed while unauthorized
    /// answers with blanks, and caching those would pin a device to
    /// "Android device · Android  · 300A30" until Pane restarts.
    props_cache: std::sync::Mutex<HashMap<String, (String, String, String)>>,
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
            serial_locks: std::sync::Mutex::new(HashMap::new()),
            props_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// The per-serial guard. Cloned out of the map so the map lock is released
    /// before anyone awaits on the guard itself.
    fn serial_lock(&self, serial: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self
            .serial_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        map.entry(serial.to_string()).or_default().clone()
    }

    /// Cached `probe_device_props`. See `props_cache`.
    async fn device_props(&self, serial: &str) -> (String, String, String) {
        if let Some(hit) = self
            .props_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(serial)
        {
            return hit.clone();
        }
        let probed = probe_device_props(serial).await;
        let (manufacturer, model, release) = &probed;
        if !manufacturer.is_empty() || !model.is_empty() || !release.is_empty() {
            self.props_cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(serial.to_string(), probed.clone());
        }
        probed
    }

    /// Read back whether this device is still actually wired up to us.
    ///
    /// Returns `Err` when the serial is mid-`add_usb`/`remove` (its lock is
    /// held) — the answer would be a half-applied state, and the caller should
    /// simply skip this round rather than count it as a failure. Also `Err`
    /// when adb itself can't be reached, for the same reason: "we couldn't
    /// tell" must not be confused with "the device is broken".
    pub async fn probe_proxy_state(&self, serial: &str, mac_port: u16) -> Result<DeviceProxyState> {
        let lock = self.serial_lock(serial);
        let _guard = lock
            .try_lock()
            .map_err(|_| anyhow!("device {serial} is mid-setup; skipping probe"))?;

        // Lines read `<serial> tcp:8888 tcp:8891`; we need our device-side spot
        // mapped to the pool port this device was assigned. Matching the port
        // too catches a stale reverse left by a previous session pointing at a
        // port nothing listens on any more.
        let reverses = run("adb", &["-s", serial, "reverse", "--list"]).await?;
        let want = format!("tcp:{mac_port}");
        let reverse_up = reverses
            .lines()
            .any(|l| l.contains("tcp:8888") && l.contains(&want));

        let proxy_set = read_http_proxy(serial).await?;

        Ok(DeviceProxyState {
            proxy_set,
            reverse_up,
        })
    }

    /// Cheap half of `probe_proxy_state`: does the device still route through
    /// us at all?
    ///
    /// Used when the proxy is stopped, where the reverse tunnel is irrelevant
    /// by definition and the only question is whether the phone is stranded
    /// pointing at a dead port. Halves the adb traffic in what is a very common
    /// idle state — Pane open, capture not running — and that polling is itself
    /// a contributor to the transient adb failures this module works around.
    pub async fn is_proxy_pointed_at_us(&self, serial: &str) -> Result<bool> {
        let lock = self.serial_lock(serial);
        let _guard = lock
            .try_lock()
            .map_err(|_| anyhow!("device {serial} is mid-setup; skipping probe"))?;
        read_http_proxy(serial).await
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
            // Probe the same props add_usb() uses so the attached row
            // reads identically to the paired row for the same device
            // ("vivo V2036 · Android 13 · 300A30"), instead of the bare
            // `adb devices -l` model (`V2036 · 300A30`). Falls back to
            // the -l model when getprop comes back empty (offline /
            // unauthorized shells).
            let (manufacturer, mut model, android_release) = self.device_props(serial).await;
            if model.is_empty() {
                // `adb devices -l` reports model with underscores in
                // place of spaces (`Pixel_7_Pro`); flip them back.
                model = parts
                    .find_map(|p| p.strip_prefix("model:"))
                    .unwrap_or("Android device")
                    .replace('_', " ");
            }
            devices.push(DiscoveredDeviceDto {
                platform: "android".into(),
                serial: serial.to_string(),
                name: format_device_name(&manufacturer, &model, &android_release, serial),
            });
        }
        Ok(devices)
    }

    /// `mac_port` is the device-specific Mac-side proxy port allocated by the
    /// `DevicePortRegistry`. The device-side reverse target stays `tcp:8888`
    /// (the phone's `http_proxy` is always `127.0.0.1:8888`); only the local
    /// forwarding port differs per device, so the proxy can attribute each
    /// connection back to its device. First device gets 8888 (backward-compat).
    pub async fn add_usb(
        &self,
        serial: &str,
        ca: &CaMaterial,
        mac_port: u16,
        presence: Presence,
    ) -> Result<DeviceDto> {
        // Serialise against any other add_usb/remove on this same serial. See
        // `serial_locks` for why this matters — concurrent runs corrupt each
        // other's work and used to leave devices silently unproxied.
        let lock = self.serial_lock(serial);
        let _guard = lock.lock().await;

        if presence == Presence::Wait {
            wait_for_device(serial).await?;
        }

        // Probe root + version.
        let rooted = run("adb", &["-s", serial, "shell", "which", "su"])
            .await
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        // Manufacturer + marketing model (`Pixel 7`, `SM-S931B`) +
        // Android release, in one round-trip. Same source the attached
        // list uses, so a device reads identically in both lists.
        let (manufacturer, model, android_release) = self.device_props(serial).await;

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
        // Device-side stays tcp:8888 (the phone's http_proxy never changes);
        // the Mac-side target is this device's assigned pool port so the proxy
        // can tell devices apart by the local port a connection lands on.
        //
        // Load-bearing vs best-effort: the data reverse and `http_proxy` below
        // are the two steps without which the device provably cannot reach the
        // proxy, so they propagate as `Err`. Everything else (CA push, PAC,
        // heartbeat, helper APK) degrades gracefully and only annotates
        // `last_error`.
        //
        // This distinction is the whole point. Until now every adb failure in
        // this function was swallowed into a `warn!` and the function still
        // returned `Ok(state: "ready")` — so a device on which literally no
        // command succeeded was reported as paired, the reapply paths logged
        // "auto-reapplied", and the UI row stayed green while no traffic could
        // possibly flow. Silent failure was the single biggest reason this bug
        // took so long to pin down.
        let mac_port_spec = format!("tcp:{mac_port}");
        run(
            "adb",
            &["-s", serial, "reverse", "tcp:8888", &mac_port_spec],
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, serial, mac_port, "adb reverse 8888 failed — device cannot reach proxy");
            anyhow!("couldn't open the data tunnel to the device (adb reverse tcp:8888 -> tcp:{mac_port}): {e}")
        })?;
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

        // Direct http_proxy — primary, what OkHttp reads. Load-bearing: without
        // it nothing on the device routes through Pane at all, so a failure
        // here is a failure of the whole operation.
        run(
            "adb",
            &[
                "-s",
                serial,
                "shell",
                "settings",
                "put",
                "global",
                "http_proxy",
                DEVICE_HTTP_PROXY,
            ],
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, serial, "setting http_proxy failed — device will not route through Pane");
            anyhow!("couldn't set http_proxy on the device: {e}")
        })?;
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

        // Label unique even with two of the same phone plugged in.
        // Shared with the attached-over-USB list (see format_device_name).
        let display_name = format_device_name(&manufacturer, &model, &android_release, serial);
        Ok(DeviceDto {
            id: Uuid::new_v4(),
            platform: "android".into(),
            connection: "usb".into(),
            serial: serial.to_string(),
            display_name,
            state: "ready".into(),
            ca_installed_at: Some(time::OffsetDateTime::now_utc().to_string()),
            capabilities: serde_json::json!({
                "rooted": rooted,
                "android_release": android_release,
                "manufacturer": manufacturer,
                "model": model,
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

    /// List third-party installed packages on the device. Used by the
    /// Logcat window's "Follow app" dropdown — system packages would
    /// dwarf the user's apps and aren't usually what people want to
    /// trace. `pm list packages -3` returns one `package:com.foo`
    /// line per app, sorted lexicographically by Android already.
    pub async fn list_third_party_packages(&self, serial: &str) -> Result<Vec<String>> {
        let out = run(
            "adb",
            &["-s", serial, "shell", "pm", "list", "packages", "--user", "0", "-3"],
        )
        .await?;
        Ok(out
            .lines()
            .filter_map(|line| line.strip_prefix("package:"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }

    /// Snapshot of every running PID with its process name. Used by
    /// the Logcat table to label rows with the package they came
    /// from. One `ps -A` round-trip (~50ms over USB), polled every
    /// 10s. PID reuse on Android is rare enough at that cadence that
    /// stale entries aren't worth tracking separately.
    pub async fn pid_names(
        &self,
        serial: &str,
    ) -> Result<std::collections::HashMap<u32, String>> {
        let out = run(
            "adb",
            &["-s", serial, "shell", "ps", "-A", "-o", "PID,NAME"],
        )
        .await?;
        let mut map = std::collections::HashMap::new();
        for line in out.lines().skip(1) {
            let mut parts = line.split_whitespace();
            let pid_s = match parts.next() {
                Some(s) => s,
                None => continue,
            };
            let name = match parts.next() {
                Some(s) => s,
                None => continue,
            };
            if let Ok(p) = pid_s.parse::<u32>() {
                map.insert(p, name.to_string());
            }
        }
        Ok(map)
    }

    pub async fn remove(&self, serial: &str) -> Result<()> {
        // Same serialisation as add_usb: a remove interleaved with a reapply
        // can leave http_proxy set while the reverse is torn down, which is
        // the worst of both worlds — the device has no internet at all.
        let lock = self.serial_lock(serial);
        let _guard = lock.lock().await;

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

/// Whether the device's global `http_proxy` currently points at Pane.
///
/// A device we never configured answers `null`; one we cleared answers `:0`.
/// Both are "not ours", which is what the caller needs to distinguish.
async fn read_http_proxy(serial: &str) -> Result<bool> {
    let out = run(
        "adb",
        &[
            "-s", serial, "shell", "settings", "get", "global", "http_proxy",
        ],
    )
    .await?;
    Ok(out.trim() == DEVICE_HTTP_PROXY)
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

    install_helper_apk(serial, apk).await?;

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

    // POST_NOTIFICATIONS is a runtime permission (Android 13+). Without
    // it, the helper's LauncherActivity gets stuck on the system
    // `GrantPermissionsActivity` dialog waiting for the user to tap
    // Allow — and if they don't (easy to miss, since pairing is a
    // background flow), `startForegroundService` never runs and the
    // HeartbeatService stays dead. Same `pm grant` trick as
    // WRITE_SECURE_SETTINGS dodges the dialog entirely on Android 13+
    // (no-op on 12 and below — permission doesn't exist there).
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
            "android.permission.POST_NOTIFICATIONS",
        ],
    )
    .await
    {
        // Pre-Android-13 devices return "Unknown permission" — that's
        // expected, not a failure. Only log if it looks like something
        // else (e.g. install in wrong user).
        if !e.to_string().contains("Unknown permission") {
            tracing::warn!(error = %e, serial, "pm grant POST_NOTIFICATIONS failed — FGS notification will be hidden but service still runs");
        }
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

/// `adb install -r` the helper APK, with one critical refinement: if
/// the device refuses with `INSTALL_FAILED_UPDATE_INCOMPATIBLE`
/// (signature mismatch with an existing install), uninstall the stale
/// one and retry. This happens routinely because the helper is signed
/// with a debug.keystore — different keystores across the user's CI,
/// my local builds, and the user's own machines all produce
/// incompatible signatures, and Android refuses updates between them.
///
/// Without the retry the install error short-circuits the whole
/// pairing flow (no `pm grant`, no `am start`), leaving the stale
/// helper installed but its service dead — so the unplug-watchdog
/// silently doesn't fire. Caused every paired-on-machine-B then
/// re-paired-on-machine-A flow to break.
async fn install_helper_apk(serial: &str, apk: &std::path::Path) -> Result<()> {
    let install = || async {
        run_for(
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
            ADB_INSTALL_TIMEOUT,
        )
        .await
    };
    match install().await {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE")
                || msg.contains("signatures do not match")
            {
                tracing::info!(
                    serial,
                    "helper APK signature differs from installed copy — uninstalling stale and retrying"
                );
                let _ = run(
                    "adb",
                    &[
                        "-s",
                        serial,
                        "shell",
                        "pm",
                        "uninstall",
                        "--user",
                        "0",
                        HELPER_PACKAGE,
                    ],
                )
                .await;
                install()
                    .await
                    .map_err(|e2| anyhow!("pm install failed after sig-mismatch uninstall: {e2}"))?;
                Ok(())
            } else {
                Err(anyhow!("pm install failed: {e}"))
            }
        }
    }
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
    // adbd restarts as root here, so the device drops off the bus and comes
    // back. Its own budget: the default 15 s is a plausible round-trip on a
    // slow device, and timing out mid-restart would abort a system-CA install
    // that was about to succeed.
    run_for(
        "adb",
        &["-s", serial, "wait-for-device"],
        ADB_INSTALL_TIMEOUT,
    )
    .await?;
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

/// Wall-clock budget for an ordinary adb invocation. Everything except an APK
/// install finishes well under a second over USB, so this is pure headroom.
///
/// The bound matters because these calls are made in sequence: pairing and
/// reapply walk a device through a dozen commands, and `reapply_all` walks
/// every paired device in turn. One wedged adb call — a device sitting on the
/// "Allow USB debugging?" dialog, an adb server mid-restart — used to stall
/// that entire chain indefinitely, so every *other* device silently went
/// unconfigured too.
const ADB_TIMEOUT: Duration = Duration::from_secs(15);

/// `adb install` pushes a multi-MB APK over USB and then waits on the package
/// manager. Minutes would be wrong, but 15 s is genuinely too tight on a cold
/// or busy device.
const ADB_INSTALL_TIMEOUT: Duration = Duration::from_secs(90);

/// Whether `add_usb` should wait for adb to see the device before working on it.
///
/// The two callers want opposite things. A user clicking Add or Re-sync is
/// acting on the on-screen attached list, which is a snapshot of `adb devices`
/// from whenever they last hit Refresh — and adb needs a few seconds to
/// re-enumerate a phone after a replug. Clicking inside that window used to
/// fail outright with "device 'X' not found": a device visibly listed on screen
/// that Pane insists doesn't exist.
///
/// The background reapply that runs on `proxy.start`, by contrast, walks *every*
/// paired device in turn, including ones that haven't been plugged in for weeks.
/// Waiting there would add the full timeout per absent device to the delay
/// before the device that *is* connected gets its proxy back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Interactive: give adb a moment to find the device, then fail clearly.
    Wait,
    /// Background: the caller already knows the device is attached, or doesn't
    /// care enough to pay for finding out.
    Assume,
}

/// How long to let adb re-enumerate a device before giving up on pairing it.
/// A USB replug settles in a couple of seconds; anything past this is a device
/// that genuinely isn't there, or one stuck on the "Allow USB debugging?"
/// prompt, and the user is better served by an error than by a longer wait.
const ADB_APPEAR_TIMEOUT: Duration = Duration::from_secs(8);

/// Block until adb can actually see `serial`.
///
/// `adb wait-for-device` returns the moment the device reaches the `device`
/// state, so on an already-connected phone this costs one round-trip. Its
/// error is rewritten because the raw one ("timed out after 8s") reads like an
/// internal fault rather than the actionable "your phone isn't connected".
async fn wait_for_device(serial: &str) -> Result<()> {
    // Check this first: `run_for` reports a missing adb through the same Err
    // channel, and rewriting *that* into "replug the cable" would send a user
    // with no platform-tools chasing a hardware problem they don't have.
    if resolve_adb().is_none() {
        return Err(anyhow!(ADB_NOT_FOUND_MSG));
    }
    run_for(
        "adb",
        &["-s", serial, "wait-for-device"],
        ADB_APPEAR_TIMEOUT,
    )
    .await
    .map(|_| ())
    .map_err(|e| {
        anyhow!(
            "adb can't see device {serial} ({}s). Replug the cable, confirm the \
             \"Allow USB debugging?\" prompt on the phone, then hit Refresh. \
             ({e})",
            ADB_APPEAR_TIMEOUT.as_secs()
        )
    })
}

async fn run(bin: &str, args: &[&str]) -> Result<String> {
    run_for(bin, args, ADB_TIMEOUT).await
}

async fn run_for(bin: &str, args: &[&str], budget: Duration) -> Result<String> {
    let resolved = if bin == "adb" {
        resolve_adb()
            .ok_or_else(|| anyhow!(ADB_NOT_FOUND_MSG))?
            .to_string_lossy()
            .into_owned()
    } else {
        bin.to_string()
    };
    // kill_on_drop so a timeout actually reaps the child. Without it the
    // future is dropped but the wedged adb process keeps holding whatever it
    // was holding, and the next call inherits the same jam.
    let child = Command::new(&resolved)
        .args(args)
        .kill_on_drop(true)
        .output();
    let output = match tokio::time::timeout(budget, child).await {
        Ok(res) => res.map_err(|e| anyhow!("{resolved}: {e}"))?,
        Err(_) => {
            return Err(anyhow!(
                "{bin} {args:?} timed out after {}s",
                budget.as_secs()
            ))
        }
    };
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
pub(crate) fn resolve_adb() -> Option<PathBuf> {
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
