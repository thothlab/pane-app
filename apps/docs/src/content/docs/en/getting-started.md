---
title: Getting started
description: Install Pane, add a device, see your first capture.
---

This page walks you from "haven't downloaded Pane yet" to "every request my app
makes shows up in the capture list".

## Install

Grab the latest build for your OS from the [download page](https://pane.thothlab.tech/#download).

| Platform | File |
| --- | --- |
| macOS Apple Silicon | `.dmg` |
| Linux x86_64 | `.AppImage` (portable) or `.deb` / `.rpm` |
| Windows x86_64 | `.msi` (recommended) or `.exe` NSIS installer |

### macOS first launch

Closed-alpha builds aren't notarised yet, so the first launch is awkward.
One-liner that downloads, copies to `/Applications` and strips the
quarantine bit:

```sh
curl -fsSL https://pane.thothlab.tech/install-macos.sh | bash
```

…or after dragging the app from the dmg:

```sh
xattr -dr com.apple.quarantine /Applications/Pane.app
```

### Linux

```sh
chmod +x Pane_*_amd64.AppImage && ./Pane_*_amd64.AppImage
```

### Windows

The NSIS installer isn't signed with an EV cert yet, so SmartScreen will
show a warning — click **More info → Run anyway**.

## First device

1. Open Pane. Sidebar shows **Captures**, **Rules**, **Devices**;
   below them as a separate group: **Settings**, **Docs**, **About**;
   at the bottom — the **Start proxy** button.
2. Hit **Start proxy** first. If you skip this, **Add device** refuses
   on purpose: pushing the proxy setting onto a phone when nothing
   listens on `127.0.0.1:8888` silently kills all internet on the
   device.
3. Connect your phone via USB:
   - **Android** — enable **USB debugging** in Developer options. The
     device must have a **PIN / pattern / password** lock screen set;
     Android won't install user CAs without one.
   - **iOS** — trust the laptop the first time you plug in.
4. **Devices → Add device**. Pane discovers attached phones via `adb`
   / `libimobiledevice`. Pick yours → **+ Add**. Pane then:
   - generates a root CA (ECDSA P-256) and stores it locally;
   - **iOS** — pushes the mobileconfig profile over USB; user confirms
     the install in Settings;
   - **Android (rooted)** — drops the CA into the system trust store
     at `/system/etc/security/cacerts/`. **Fully automatic.**
   - **Android (no root)** — pushes the CA file to
     `/sdcard/Pane/pane-ca.pem` and the device row shows a step-by-step
     guide for the manual install (see below);
   - wires up `adb reverse tcp:8888 tcp:8888` plus `tcp:8889 tcp:8889`
     for the PAC server — traffic flows over USB, **no Wi-Fi setup
     needed**;
   - points the device at the PAC URL so DIRECT fallback kicks in on
     unplug (see PAC section below).
5. On Android without root — follow the manual-install guide in the
   Devices row (collapsible "How to install the CA certificate" block).
6. On iOS — confirm the profile in Settings → Profile Downloaded.

The next request your app makes is a capture in the list. Click a row
to see method / URL / status, headers, body and timing.

### Why CA install on Android needs a manual step

Android 11+ always routes CertInstaller through the SAF file picker
(scoped-storage). Samsung One UI on Android 16+ goes further and
blocks programmatic CA installs from every source (shell,
`KeyChain.createInstallIntent` from an app, MDM-style
`DevicePolicyManager`) — Google and Samsung made the flow strictly
user-initiated on recent builds. No programmatic workaround exists
without root.

Pane does everything it can to make the manual step painless:
- pushes `pane-ca.pem` into its own `/sdcard/Pane/` folder so Samsung
  Smart Manager doesn't sweep it (Downloads gets cleaned);
- the device row in Devices shows a collapsible guide with the exact
  click path for your Settings layout, a copy-to-clipboard for the
  file path, and the lock-screen PIN prerequisite reminder.

### PAC-based proxy

Pane sets `http_proxy_pac` on the device, not a direct `http_proxy`.
When USB is connected, the PAC server is reachable via `adb reverse`
and traffic flows through Pane. When USB is unplugged (or Pane
closes), the PAC URL becomes unreachable and Android transparently
falls back to DIRECT — **the device keeps its internet**. The old
direct-proxy approach used to strand the device on
ERR_PROXY_CONNECTION_FAILED in that scenario.

### When something goes sideways

- **`adb not found`** — Pane looks in `ANDROID_HOME`, `~/Library/
  Android/sdk`, `/opt/homebrew/bin`, `/usr/local/bin`. If the Devices
  page shows a yellow "Android tooling not found" banner, install
  `platform-tools` from the Android SDK or Android Studio.
- **`adb reverse failed`** — usually after USB reseating or an adb-
  server restart. Hit **Re-sync** on the paired device row.
- **Device has "no internet" after Stop proxy** — Pane v0.1.12+
  auto-clears the proxy setting on every paired Android when the
  proxy stops. On older builds: `adb shell settings put global
  http_proxy :0` to recover, or **Remove device**.
- **HTTPS in my app isn't decrypted** — the app has to trust user
  CAs. In a debug build, add `network_security_config.xml` with
  `<debug-overrides>` → `<trust-anchors><certificates src="user"/>
  </trust-anchors>`. Release builds need an explicit opt-in. Chrome
  / Samsung Internet ignore user CAs by design — test with Firefox
  or your own app instead.
- **TLS pinning** — Pane doesn't try to bypass pinning. Disable
  pinning in your debug build. For owned-device security research,
  Frida or Magisk modules layer on top of Pane.

## Updates

Pane checks for new releases:

- at launch,
- once an hour while the window is open,
- whenever the window regains focus.

When a newer build is out, an **Update to vX.Y.Z** button appears in
the sidebar under the version. Click it and Pane downloads the
minisign-signed bundle and relaunches. Force a check via
**About → Check for updates**.

## Reading captures

The capture list supports a small filter DSL on the search bar:

```text
host:api.example.com          # only requests to this host
status:5..                    # any 5xx
!error:tls_handshake          # exclude pinning + handshake failures
status:200..299 host:*.dev    # ranges + globs
google                        # bareword: substring of host or path
```

Save the current filter with the ☆ icon to pin it to the sidebar.

Right pane shows **Overview / Request / Response / Timing / TLS**. The
body viewer auto-detects JSON / XML / text:

- **Tree** — collapsible nodes, copy by path or by value.
- **Pretty** — formatted, syntax-highlighted text.
- **Raw** — bytes as they came off the wire.

## Next

- [Response stubs](/docs/en/rules/) — replace or patch responses for testing.
- [Release process](/docs/en/reference/releases/) — cutting tags, for maintainers.
