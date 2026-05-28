# my-charles

A modern HTTPS network debugger focused on one thing: making mobile-device setup take 30 seconds instead of 15 minutes. Plug your iPhone or Android in over USB, click **Add**, and start inspecting traffic — no Settings dance, no certificate trust spelunking, no Wi-Fi proxy editing.

> **Status:** early MVP scaffold. Cross-platform shell, proxy engine (HTTP/1.1), capture/replay storage, device-setup pipelines, and CI are wired up. TLS-decrypted MITM (HTTPS body inspection beyond CONNECT metadata) is the next focused milestone. See `docs/tasks/prd_01_mvp-network-debugger/` for the full PRD + 14-task delivery plan.

## What's inside

- **Tauri 2** desktop shell (Windows / macOS / Linux).
- **SolidJS + Tailwind** UI: virtualised capture list, filter DSL, detail panes, replay composer.
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

Click **Start proxy** in the lower-left, set your device's HTTP proxy to `127.0.0.1:8888`, install the root CA via **Settings → Export PEM** (or use the **Devices** view for USB-driven setup), and traffic will start populating the **Captures** view.

## How it compares

|                          | Charles | Proxyman | Reqable | mitmproxy | **my-charles**          |
| ------------------------ | ------- | -------- | ------- | --------- | ----------------------- |
| Price                    | $50     | $69/yr   | freemium | free      | **free / Apache-2.0**   |
| Modern UI                | ✗       | ✓        | ✓       | partial   | ✓                       |
| One-command device setup | ✗       | ✗        | ✗       | partial   | **★ primary focus**     |
| Cert-pinning UX          | silent  | silent   | partial | manual    | **detect + explain**    |
| Git-friendly config      | ✗       | ✗        | ✗       | ✗         | planned (post-MVP)      |

## Boundaries

my-charles is designed for inspecting **your own** apps and for legitimate, authorised security work. It does **not** bypass certificate pinning — when an app pins, you'll see a clear explanation and pointers to the appropriate (and external) tools instead of a silent failure.

It is **not** a production traffic monitor, **not** a packet-level capture tool, and **not** a load-testing harness.

## Repository layout

```
src/                    SolidJS frontend (Tauri webview)
src-tauri/              Tauri main crate + IPC command modules
crates/
  mycharles-ipc/        Shared DTOs between Rust and TS
  mycharles-engine/     ProxyEngine trait + EngineEvent
  mycharles-engine-mitm/  Native HTTP/1.1 MITM impl
  mycharles-ca/         Root CA generation, rotation, keychain storage
  mycharles-storage/    SQLite + body blobs + filter DSL + replay
  mycharles-devices/    Cross-platform device manager + state machine
  mycharles-ios/        libimobiledevice wrapper
  mycharles-android/    adb wrapper, CA install paths
  mycharles-mobileconfig/  Apple .mobileconfig builder
  mycharles-setup-server/  LAN HTTP server for QR-fallback pairing
  mycharles-pinning/    Pinning heuristic + hint kinds
docs/                   PRD, task decomposition, planning notes
.github/workflows/      CI + release
scripts/                fetch-sidecars, dev launcher
```

## License

[Apache-2.0](LICENSE). Third-party components used at runtime keep their respective licences.
