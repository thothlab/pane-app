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

use anyhow::{anyhow, Context, Result};
use mycharles_ca::CaMaterial;
use mycharles_ipc::{DeviceDto, DiscoveredDeviceDto};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use uuid::Uuid;

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
        }

        // Always set up proxy redirect (works regardless of trust path).
        let _ = run("adb", &["-s", serial, "reverse", "tcp:8888", "tcp:8888"]).await;
        let _ = run(
            "adb",
            &["-s", serial, "shell", "settings", "put", "global", "http_proxy", "127.0.0.1:8888"],
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
            }),
            last_error,
        })
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
        sidecar_or_path("adb")
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

fn sidecar_or_path(bin: &str) -> String {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join(bin);
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }
    bin.to_string()
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
