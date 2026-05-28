# Task 13 — Packaging, signing, auto-update (Win/Mac/Linux)

## Goal
Поставить готовые-к-распространению installable артефакты на всех трёх target-OS, с code-signing где требуется, и работающим auto-update механизмом через Tauri updater. Размер ≤ 80 MB на платформу.

## Scope
**In:**
- macOS: `.dmg` (universal x64 + arm64), notarization, Developer ID signing.
- Windows: `.msi` (x64), Authenticode signing.
- Linux: `.deb` + AppImage (x64), GPG signing artefactов.
- Tauri updater: endpoint, signature verification, UI prompt.
- CI/CD pipeline: tagged release → built + signed + uploaded to GitHub Releases.

**Out:**
- Mac App Store / Microsoft Store / Snap submission — beyond MVP.
- Auto-install at OS boot — beyond MVP.

## Subtasks

### 13.1 macOS build
- [ ] `tauri build --target universal-apple-darwin`.
- [ ] Entitlements: `com.apple.security.device.usb`, `com.apple.security.network.client/server`, `com.apple.security.files.user-selected.read-write`.
- [ ] Hardened runtime ON.
- [ ] Sign: `codesign --deep --force --sign "Developer ID Application: <Org>"`.
- [ ] Notarize: `xcrun notarytool submit ... --wait`.
- [ ] Staple: `xcrun stapler staple my-charles.dmg`.
- [ ] CI secrets: APPLE_ID, APP_PASSWORD, TEAM_ID, CERT_P12, CERT_PASSWORD.

### 13.2 Windows build
- [ ] `tauri build --target x86_64-pc-windows-msvc`.
- [ ] WiX или MSI built-in.
- [ ] Code signing с Authenticode (EV cert если есть; OV для начала).
- [ ] SmartScreen reputation — ожидаемо плохая в первые недели после релиза.
- [ ] CI secrets: WIN_CERT_P12, WIN_CERT_PASSWORD.

### 13.3 Linux build
- [ ] `tauri build --target x86_64-unknown-linux-gnu`.
- [ ] AppImage (через `tauri-bundler`).
- [ ] `.deb` для Ubuntu/Debian.
- [ ] GPG signing AppImage + `.deb`.
- [ ] CI secret: GPG_KEY.
- [ ] (Опционально) Flatpak manifest — отложить до v0.2.

### 13.4 Auto-update endpoint
- [ ] `releases.my-charles.tech/<channel>/<platform>/latest.json` — статика на S3+CloudFront или GitHub Releases.
- [ ] Schema: `{version, pub_date, url, signature, notes}`.
- [ ] Tauri updater подписи: `tauri signer generate` ключи, public ключ в `tauri.conf.json`.
- [ ] Каналы: `stable`, `beta`, `dev`.

### 13.5 Update UX
- [ ] Tauri updater event listener в UI.
- [ ] При available update — non-modal toast "v0.X.Y available. View notes / Update now / Skip".
- [ ] "Update now" → download with progress, restart.
- [ ] Settings → "Check for updates" + auto-check toggle (default ON).

### 13.6 CI pipeline (release)
- [ ] Trigger: `git tag v*`.
- [ ] Job matrix: 3 OS.
- [ ] Каждый job: checkout → fetch sidecars → tauri build → sign → upload artifact.
- [ ] Aggregator job: создаёт GitHub Release, прикладывает все артефакты + `latest.json`.
- [ ] Smoke-tests post-release: download → install in VM → launch → check version.

### 13.7 Bundle size optimization
- [ ] `cargo build --release` с `strip = true`, `lto = "fat"`, `codegen-units = 1`.
- [ ] UI bundle: tree-shake, lazy-load detail panes, avoid duplicate React-like libs.
- [ ] Sidecars: только нужные бинари (`adb` без `fastboot`, `libimobiledevice` без неиспользуемых CLI).
- [ ] Target ≤ 80 MB per artifact.

## Deliverables
- Working CI release pipeline.
- Signed installable для трёх OS, скачиваемые из GitHub Releases.
- Updater endpoint работает.
- `docs/release-process.md` — playbook для maintainer'а.

## Definition of Done
- [ ] AC9 из PRD: 3 installable артефакта ≤ 80 MB, signed, auto-update работает.
- [ ] Smoke test на чистой VM (Win11, macOS 14, Ubuntu 24.04) — install + launch + add device.
- [ ] Tag v0.1.0-rc1 → CI триггерит full release → артефакты в GitHub Releases.
- [ ] Updater тест: build v0.1.0, install, опубликовать v0.1.1 → клиент видит prompt → update → restart на новой версии.

## Tests
- **CI:** matrix build на каждый PR (без signing — только artifact creation).
- **Release smoke:** на каждый tag — пройти install matrix manually (или via VM automation).
- **Updater integration:** старая версия → пустить fake update server → проверить flow.

## Dependencies
- Task 01 (CI baseline).
- Task 08 (sidecar bundling).
- Все остальные tasks желательно завершены (но packaging можно делать инкрементально).
