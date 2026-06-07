[Русский](README.md) · **English**

# Pane

A modern HTTPS network debugger focused on one thing: making mobile-device setup take 30 seconds instead of 15 minutes. Plug your iPhone or Android in over USB, click **Add**, and start inspecting traffic — no Settings dance, no certificate trust spelunking, no Wi-Fi proxy editing.

> **Status:** v0.1.0 — first public release. Cross-platform shell, proxy engine (HTTP/1.1 with TLS MITM), capture/replay storage, response stubs and patches, device-setup pipelines, and CI/release pipeline are wired up. See the [documentation](https://pane.thothlab.tech/docs/) for user-facing features and setup.

## What's inside

- **Tauri 2** desktop shell (Windows / macOS / Linux).
- **SolidJS + Tailwind** UI: virtualised capture list, filter DSL, detail panes, replay composer. Full EN / RU localization via `@solid-primitives/i18n`, switch reactively from Settings.
- **Rust workspace** of focused crates: engine trait, native MITM proxy, root-CA management (rcgen + OS keychain), SQLite storage with content-addressed body blobs, iOS / Android device pipelines (libimobiledevice + adb sidecars), Apple `mobileconfig` builder, QR-fallback setup server, cert-pinning heuristic.
- **CI** matrix on Windows, macOS, Linux — fmt + clippy + tests + Tauri debug build.

## Quick start

```bash
# 1. Toolchain
rustup default stable
brew install pnpm   # or: corepack enable

# 2. Install deps
pnpm install

# 3. (One-time) place sidecar binaries
./scripts/fetch-sidecars.sh    # prints instructions

# 4. Run
pnpm tauri:dev
```

Click **Start proxy** in the lower-left. Then **Devices → Add device** — Pane installs the root CA over USB (fully auto on iOS and rooted Android; on non-root Android it pushes the file and shows an inline manual-install walkthrough), wires up `adb reverse`, and sets both `http_proxy` (primary, for OkHttp/native stacks) and `http_proxy_pac` (bonus for Chromium) on Android. On Android, Pane also installs a tiny companion APK (~4 MB) — a heartbeat watchdog that automatically clears the proxy when you unplug USB so the device keeps its internet. Traffic starts populating the **Captures** view.

## How it compares

|                          | Charles | Proxyman | Reqable | mitmproxy | **Pane**          |
| ------------------------ | ------- | -------- | ------- | --------- | ----------------------- |
| Price                    | $50     | $69/yr   | freemium | free      | **free / Apache-2.0**   |
| Modern UI                | ✗       | ✓        | ✓       | partial   | ✓                       |
| One-command device setup | ✗       | ✗        | ✗       | partial   | **★ primary focus**     |
| Cert-pinning UX          | silent  | silent   | partial | manual    | **detect + explain**    |
| Git-friendly config      | ✗       | ✗        | ✗       | ✗         | planned (post-MVP)      |

## Boundaries

Pane is designed for inspecting **your own** apps and for legitimate, authorised security work. It does **not** bypass certificate pinning — when an app pins, you'll see a clear explanation and pointers to the appropriate (and external) tools instead of a silent failure.

It is **not** a production traffic monitor, **not** a packet-level capture tool, and **not** a load-testing harness.

## Repository layout

```
src/                    SolidJS frontend (Tauri webview)
src/i18n/               EN + RU translation dictionaries + reactive translator
src-tauri/              Tauri main crate + IPC command modules
crates/
  pane-ipc/        Shared DTOs between Rust and TS
  pane-engine/     ProxyEngine trait + EngineEvent
  pane-engine-mitm/  Native HTTP/1.1 MITM impl
  pane-ca/         Root CA generation, rotation, keychain storage
  pane-storage/    SQLite + body blobs + filter DSL + replay
  pane-devices/    Cross-platform device manager + state machine
  pane-ios/        libimobiledevice wrapper
  pane-android/    adb wrapper, CA install paths, PAC server wiring
  pane-mobileconfig/  Apple .mobileconfig builder
  pane-setup-server/  LAN HTTP server for QR-fallback pairing
  pane-pinning/    Pinning heuristic + hint kinds
apps/
  web/                  pane-web service (landing + docs + release endpoints)
  docs/                 Astro Starlight documentation site
.github/workflows/      CI + release
scripts/                fetch-sidecars, dev launcher
```

## License

[Apache-2.0](LICENSE). Third-party components used at runtime keep their respective licences.
