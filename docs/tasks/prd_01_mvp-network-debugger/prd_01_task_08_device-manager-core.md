# Task 08 — Device manager core (USB discovery, state machine, sidecar bundling)

## Goal
Cross-platform слой управления устройствами: обнаружение подключённых iOS/Android по USB, унифицированная state machine из PRD §5.1, bundling sidecar бинарей (`libimobiledevice` CLIs + `adb`) внутрь Tauri-пакета, права/permissions/первый-запуск-нюансы по OS.

## Scope
**In:**
- `crates/mycharles-devices/` — trait + dispatch к iOS/Android реализациям.
- USB-discovery (poll + event), unification API.
- State machine с persistence в SQLite.
- Sidecar binary management: какие бинари везём, как вызываем, как ищем (`tauri::api::process::Command::new_sidecar`).
- UI: Devices view (list + Add device modal).

**Out:**
- iOS-specific flows (Task 09).
- Android-specific flows (Task 10).
- Wi-Fi/QR fallback (Task 11).

## Subtasks

### 8.1 Devices crate
- [ ] `pub trait DevicePlatform { async fn discover() -> Vec<DiscoveredDevice>; async fn add(...) -> Result<Device>; async fn remove(...) -> Result<()>; }`.
- [ ] `IosPlatform` и `AndroidPlatform` (stub имплементации — наполняем в 09/10).
- [ ] `DeviceManager` агрегирует обе платформы.

### 8.2 USB discovery
- [ ] iOS: `idevice_id -l` (вывод UDID list).
- [ ] Android: `adb devices -l` (вывод serial + features).
- [ ] Poll каждые 2 s (event-based на macOS через `IOKit` — оптимизация для post-MVP).
- [ ] Diff → emit `device.attached`/`device.detached` events.

### 8.3 State machine
- [ ] `enum DeviceState { Pairing, TrustInstall, ProxySetup, Ready, Error(String), Removed }`.
- [ ] Transitions через `DeviceManager::transition(id, new_state)` — gate'им invalid переходы.
- [ ] Persistence: на каждом переходе update в SQLite + emit `device.state_changed`.
- [ ] При старте приложения — load `Ready` устройства, попробовать re-attach.

### 8.4 Sidecar bundling
- [ ] iOS бинари (per OS):
  - macOS: brew-built `libimobiledevice` (universal binary) — bundling через Tauri sidecar (`*.app/Contents/MacOS/sidecar-bin/`).
  - Windows: `imobiledevice-net` или precompiled mingw artefacts.
  - Linux: брать `libimobiledevice` из apt + bundling в AppImage.
- [ ] Android: `adb` (Platform Tools) для трёх OS — версия зафиксирована, ~5 MB.
- [ ] Скрипты в `scripts/fetch-sidecars.{sh,ps1}` — скачивает в `src-tauri/binaries/<target-triple>/`.
- [ ] CI step: проверка что все sidecar binaries присутствуют до `tauri build`.

### 8.5 Sidecar runner
- [ ] `SidecarCmd::run(bin, args) -> Result<Output>` — обёртка с timeout, error mapping.
- [ ] Логирование всех вызовов в trace (`debug` уровень, без stdout утечки в release).
- [ ] Per-OS path resolution.

### 8.6 Devices UI
- [ ] `/devices` view: list устройств + status badge per state.
- [ ] "Add device" modal: tabs iOS / Android.
- [ ] Per-device row: name, serial, OS version, state, last activity. Кнопки "Re-trust", "Remove", "Open setup wizard".
- [ ] Live state via `device.state_changed` event.

### 8.7 Permissions / OS specifics
- [ ] macOS: app должен иметь `com.apple.security.device.usb` entitlement (для libimobiledevice). Зафиксировать в `entitlements.plist`.
- [ ] Windows: WinUSB driver hint при `add iOS` если устройство не обнаружено (link на Apple Mobile Device Support).
- [ ] Linux: `udev` rules — first-run скрипт пишет в `/etc/udev/rules.d/51-apple.rules` (через elevated помощника? нет — даём инструкцию, не пишем сами в системные пути).

## Deliverables
- `crates/mycharles-devices/` с trait + manager.
- `src/views/devices/`.
- Sidecar binaries в repo (или CI artefacts), bundling в `tauri.conf.json`.
- Документ `docs/sidecars.md` — как обновлять.

## Definition of Done
- [ ] `device.list_attached_usb` возвращает реальный список на всех трёх OS.
- [ ] State machine отвергает invalid переходы (тест с обходом DSL).
- [ ] `tauri build` включает sidecars; артефакт работает на чистой VM без brew/apt.
- [ ] UI devices view live-обновляется при attach/detach.

## Tests
- **Unit:** state machine transitions (matrix).
- **Integration (CI):** mock-sidecars (echo binaries) → discovery flow.
- **Manual (matrix):** реальный iPhone + реальный Android Pixel на трёх OS — discovery работает.

## Dependencies
- Task 01, Task 04.
- Task 03 (state persistence в SQLite — миграция V003 для `devices` таблицы, если не сделана в Task 03).
