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
     `/sdcard/Download/pane-ca.pem` and the device row shows a
     step-by-step guide for the manual install (see below);
   - wires up `adb reverse tcp:8888 tcp:8888`, `tcp:8889 tcp:8889`
     (PAC), and `tcp:8890 tcp:8890` (helper heartbeat) — traffic flows
     over USB, **no Wi-Fi setup needed**;
   - sets `http_proxy=127.0.0.1:8888` (primary, what OkHttp/Retrofit
     and native HTTP stacks use) and `http_proxy_pac` pointing at the
     local PAC server (bonus, for Chromium-based browsers);
   - installs the companion APK `tech.thothlab.pane.helper` and grants
     it `WRITE_SECURE_SETTINGS` + `POST_NOTIFICATIONS` via adb — a
     watchdog that clears the proxy when you unplug USB (see "Helper
     APK" below).
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
- pushes `pane-ca.pem` into `/sdcard/Download/` — the only folder the
  system CertInstaller picker opens by default. Custom folders like
  `/sdcard/Pane/` are invisible to the SAF picker on most Android
  builds;
- pre-cleans stale `pane-ca*` files from `/sdcard/Pane/` and
  `/sdcard/Documents/` (legacy locations from earlier Pane versions)
  so the picker can't pull up the wrong file;
- the device row in Devices shows a collapsible guide with the exact
  click path for your Settings layout, a copy-to-clipboard for the
  file path, and the lock-screen PIN prerequisite reminder.

### Proxy configuration on the device

Pane sets **two** global settings keys on the device:

- `http_proxy=127.0.0.1:8888` — the primary one. OkHttp, Retrofit,
  native stacks (libcurl), HttpURLConnection — everything that reads
  `ProxySelector.getDefault()` honors it. This is **mandatory**:
  without it, 90% of Android apps don't route through Pane (Chrome
  and WebView do, but those aren't usually the app you're debugging).
- `http_proxy_pac=http://127.0.0.1:8889/proxy.pac` — bonus for
  Chromium-based stacks (Chrome, WebView, Samsung Internet) which
  prefer PAC.

A PAC-only setup broke OkHttp apps (banking, MTS, most business
apps), so Pane always sets both.

### Helper APK — why the device keeps internet on unplug

Earlier versions left `http_proxy` set after Stop proxy or USB
unplug, so the device kept dialling a dead `127.0.0.1:8888` — no
internet until you ran Remove device. Pane now installs a companion
APK `tech.thothlab.pane.helper` (~4 MB) that holds a heartbeat
socket back to Pane via adb-reverse'd `127.0.0.1:8890`.

When the heartbeat dies (USB unplug, Pane closed or crashed), the
on-device foreground service sets `http_proxy=:0` via
`WRITE_SECURE_SETTINGS` (no root, no Magisk — Pane grants this
permission via `adb shell pm grant` during first pair). Internet
comes back within ~6 seconds of disconnect.

Safety check: the helper only touches `http_proxy` if its current
value matches what Pane wrote. If you manually set your own proxy,
the helper leaves it alone.

A "Pane connected" / "Pane disconnected" notification stays in the
shade — your only visual signal that Pane is interacting with the
device.

If you don't want the helper (corporate VPN, MDM policies, etc.),
uninstall via Settings → Apps → Pane Helper. You lose the unplug
auto-cleanup but everything else keeps working.

### When something goes sideways

- **`adb not found`** — Pane looks in `ANDROID_HOME`, `~/Library/
  Android/sdk`, `/opt/homebrew/bin`, `/usr/local/bin`. If the Devices
  page shows a yellow "Android tooling not found" banner, install
  `platform-tools` from the Android SDK or Android Studio.
- **`adb reverse failed`** — usually after USB reseating or an adb-
  server restart. Hit **Re-sync** on the paired device row.
- **Device has "no internet" after Stop proxy / USB unplug** — from
  v0.1.41 the companion APK handles this autonomously even without
  the laptop attached, clearing the proxy within ~6 sec. If the phone
  never got the helper (first pair on this machine failed, or you
  uninstalled the helper), recover directly on the phone: **Settings
  → Wi-Fi → tap the active network → Proxy → None**. Or via adb if
  the cable is plugged: `adb shell settings put global http_proxy :0`.
- **HTTPS in my app isn't decrypted** — the app has to trust user
  CAs. In a debug build, add `network_security_config.xml` with
  `<debug-overrides>` → `<trust-anchors><certificates src="user"/>
  </trust-anchors>`. Release builds need an explicit opt-in. Chrome
  / Samsung Internet ignore user CAs by design — test with Firefox
  or your own app instead.
- **TLS pinning** — Pane doesn't try to bypass pinning. Disable
  pinning in your debug build. For owned-device security research,
  Frida or Magisk modules layer on top of Pane.

## Theme and text size

**Settings → Appearance → Theme** toggles light / dark (`System`
follows the OS setting). The choice is synced across every open Pane
window, including standalone Logcat windows.

**Settings → Appearance → Text size** scales the whole UI in four
steps: `Small / Medium / Large / Extra large`. It sets `font-size`
on `<html>`, so every text size, padding, and icon grows
proportionally. Default is `Small` — matches the pre-0.1.65 look.

## Interface language

Pane ships in **English** and **Russian**. English is the default.
Switch from **Settings → Appearance → Language**. The choice persists
in `localStorage` and applies reactively — no restart needed.

Every UI screen is translated (Captures, Rules, Devices, Settings,
About, Replay, body viewer, manual-install guide). Backend-side
`last_error` strings (from `pane-android` / `pane-engine`) stay in
English on purpose — same policy as logs and source.

Adding a new locale is one file: drop `src/i18n/<lang>.ts` with the
same shape as `en.ts` (the `Dict` type contract enforces it at
compile time), and register it in the `LOCALES` array in
`src/i18n/index.ts`.

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
host:api.example.com                # only requests to this host
host:api.foo.com,api.bar.com        # OR — comma-separated alternatives
method:POST,PUT,DELETE              # OR across methods
status:200,500..599                 # mix: exact 200 OR any 5xx
status:5..                          # any 5xx
!error:tls_handshake                # exclude pinning + handshake failures
!host:cdn.*,fonts.*                 # "neither cdn.* nor fonts.*"
status:200..299 host:*.dev          # ranges + globs (tokens AND'd)
google                              # bareword: substring of host or path
```

Save the current filter with the ☆ icon to pin it to the sidebar.
Typing the name of an existing saved filter in the Save dialog
turns the button into **Update** — overwrites query/color/pin
in place, no duplicate row.

The horizontal split between **Headers** and **Body** on the
Request/Response tabs is draggable. Position is remembered per pane
(stored in localStorage) and persists across restart. Double-click
the splitter to reset.

Right pane shows **Overview / Request / Response / Timing / TLS**. The
body viewer auto-detects JSON / XML / text:

- **Tree** — collapsible nodes, copy by path or by value.
- **Pretty** — formatted, syntax-highlighted text.
- **Raw** — bytes as they came off the wire.

## Logcat window

A **Logcat** button shows up next to **+ Add** on Android devices in
the Devices view. It opens a **separate non-modal window** streaming
`adb logcat` from that device, so you can keep captures running in
the main window while reading filtered logs alongside. Independent
windows, one per device — clicking Logcat again on the same device
focuses the existing window rather than spawning a duplicate.

What's inside:

- **Virtualized 100k-entry buffer** (~5 min of history even on a
  chatty firehose): Time · PID · Level (coloured V/D/I/W/E/F) · Tag ·
  Message.
- **Pause** (Space) — freezes the buffer, the upstream stream keeps
  running on the backend. **Clear** (⌘K) — wipes the buffer.
  **Follow** — auto-scroll to newest entry; turns off automatically
  if you scroll up.
- **Follow app** — dropdown of installed third-party packages.
  Pick one → backend resolves PID via `adb shell pidof` every 5s,
  the view filters down to that PID. App restart → PID changes → the
  filter transparently picks up the new process. No stale `pid:1234`
  literals to update.
- **Filter DSL** — in-memory (the buffer is already in renderer
  memory, no need to go through SQL):
  ```text
  OkHttp                              # bare word: substring in tag or message
  tag:OkHttp,Retrofit                 # comma-joined positives — OR
  tag:!CatalogParser,!TrafficStats    # negatives via ! — all must NOT match
  tag:!Spam,!Noise,SSH                # mixed: (not Spam AND not Noise) AND contains SSH
  level:E                             # error only
  level:W..F                          # range: warn and above
  pid:1234                            # exact PID
  app:com.foo,!com.foo.helper         # pids of com.foo minus pids of com.foo.helper
  ~^(?!.*Connection)                  # regex via ~, matches tag or message
  !tag:OkHttp                         # outer ! — flips the whole token
  tag:OkHttp !msg:keep-alive          # AND between tokens
  ```
  A comma in a value combines alternatives: positives OR together,
  negatives (`!value`) all must NOT match, the two groups AND together.
  An outer `!key:foo` flips the entire token.
- **Export** — save the currently-visible filtered view to a `.log`
  file in `threadtime` format (drop-in for Android Studio / any
  grep pipeline).
- **⌘F** — focuses the filter input.

Safety bits:
- Closing the window kills the `adb logcat` subprocess (via
  `WindowEvent::Destroyed` + `kill_on_drop`).
- EOF / stream break (USB reseat, adb-server restart) → automatic
  reconnect with backoff 0.5s → 10s, capped at 5 attempts.
- Entries are emitted in **batches** (50 lines or 100ms, whichever
  comes first) so a 1000+ lines/sec firehose doesn't lock the
  Solid reactor.

## Next

- [Response stubs](/docs/en/rules/) — replace or patch responses for testing.
- [Release process](/docs/en/reference/releases/) — cutting tags, for maintainers.
