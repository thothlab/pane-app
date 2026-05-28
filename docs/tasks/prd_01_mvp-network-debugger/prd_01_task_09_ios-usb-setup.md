# Task 09 — iOS USB one-command setup (★ ключевая ставка)

## Goal
По одной кнопке "Add iOS device" получить полностью настроенный iPhone: CA в keychain, proxy включён через usbmux-туннель, первый трафик через 30 секунд. Без Settings, Safari, ручного trust в двух местах.

## Scope
**In:**
- Pairing через `idevicepair`.
- Push CA на устройство (через Apple Configurator-совместимый mobileconfig profile + `ideviceinstaller` или native iTunes Pairing API).
- Auto-trust: где возможно — через `ideviceactivation`/`ideviceenterprisedeploy`; где нет — guided wizard с двумя экранами (System Settings → General → VPN & Device Management → Profile; Settings → General → About → Cert Trust Settings).
- Proxy redirect через usbmuxd tunnel: `iproxy <local-port> <device-port>` + системная настройка proxy через mobileconfig.
- Cleanup на remove: revoke profile + снять proxy.
- Поддержка iOS 16/17/18.

**Out:**
- Wi-Fi fallback (Task 11).
- Pre-iOS 16 — out of scope (не тестируем, документируем "best-effort").

## Subtasks

### 9.1 Pairing flow
- [ ] `idevicepair pair` → on success получаем pairing record.
- [ ] Если устройство locked / requires user accept — UI showing "Tap Trust on your iPhone" с loading spinner; poll каждые 1.5 s.
- [ ] Timeout 60 s — fallback к QR (Task 11) с прямой передачей профиля.

### 9.2 CA profile генерация (mobileconfig)
- [ ] Crate `crates/mycharles-mobileconfig/`:
  - Build mobileconfig XML с: root CA payload + Wi-Fi/HTTP proxy payload (HTTP-Manual or HTTP-Auto).
  - Подпись профиля self-sign (отдельный signing cert, чтобы устройство показывало "Verified" UI).
- [ ] Profile UUID per device (для cleanup mapping).

### 9.3 Profile install via libimobiledevice
- [ ] Попытка через `ideviceinstaller`-aналог — на самом деле profile install идёт через `idevicepair`-stack + lockdownd `com.apple.misagent` service. Если в `libimobiledevice` нет высокоуровневой команды — пишем минимальный wrapper (Rust) поверх lockdownd protocol.
- [ ] Если direct push не сработал (iOS 17+ tighter restrictions) — пишем профиль в Files / iTunes share или fallback на QR (Task 11).

### 9.4 Trust wizard
- [ ] Detect "profile installed, awaiting trust" state (через lockdownd query).
- [ ] Wizard step 1: "Open Settings → General → VPN & Device Management". Картинка/гиф.
- [ ] Wizard step 2 (только если CA): "Settings → General → About → Certificate Trust Settings → toggle ON". Картинка/гиф.
- [ ] Авто-определение когда trust готов (через `ideviceinfo` или повторный TLS-handshake).

### 9.5 Proxy redirect через usbmux
- [ ] `iproxy <hostport> <deviceport>` — туннель. Но на самом деле для proxy redirect нужен другой подход: либо ставим Wi-Fi proxy с адресом `127.0.0.1:<port>` (это работает только в мобильном Hotspot back to Mac mode), либо используем `pymobiledevice3`'s `tunneld` (post-iOS 17).
- [ ] **Plan A:** mobileconfig содержит Wi-Fi proxy payload с `127.0.0.1` и `<port>`, плюс активный usbmuxd tunnel перенаправляет с устройства localhost → desktop proxy. Test на iOS 16/17/18.
- [ ] **Plan B (если A не работает на iOS 17+):** ставим device-side proxy = `localhost:8888`, плюс `iproxy 8888 8888` reverse — port forwarding с устройства на хост. Этот подход требует JIT-режим / developer mode на iOS 17 (Developer Mode toggle).
- [ ] Документируем какой план для какой iOS-версии.

### 9.6 Verification
- [ ] После всех шагов — открыть TCP к `127.0.0.1:<port>` с устройства (через `idevicesyslog`-symptom check или дёрнуть `curl` через `idevicesh`).
- [ ] Если capture не появился в течение 15 s — UI показывает "Setup completed but no traffic yet. Open any app on your device or visit a website."

### 9.7 Cleanup на remove
- [ ] Revoke profile: `idevicemobile-removeprofile <uuid>`.
- [ ] Stop iproxy tunnel.
- [ ] Mark `Device.state = removed`.

### 9.8 Error matrix → UX
| Error | Cause | UI |
|---|---|---|
| `pairing_denied` | User tapped "Don't trust" | "Please tap Trust on your iPhone and retry" |
| `developer_mode_required` | iOS 17+ without DevMode | wizard к Settings → Privacy → Developer Mode |
| `profile_install_failed` | iOS rejected mobileconfig | fallback к QR-installation (Task 11) |
| `tunnel_failed` | iproxy не запустился | "Cannot start tunnel — try unplug/replug" |

## Deliverables
- `crates/mycharles-ios/` с pairing + profile + tunnel.
- `crates/mycharles-mobileconfig/`.
- `src/views/devices/ios-wizard/`.
- Plan A vs Plan B документация в `docs/ios-setup-strategy.md`.

## Definition of Done
- [ ] AC1 из PRD: cold start → first capture ≤ 30 s (P50 на 10 прогонах) на iOS 17.
- [ ] Работает на iOS 16, 17, 18 (или есть документированный fallback).
- [ ] Remove device снимает profile + tunnel.
- [ ] Все ошибки имеют user-friendly UX (нет голых stack traces).
- [ ] Wizard для trust steps — gif/png на каждом шаге.

## Tests
- **Manual matrix:** iPhone SE 3 (iOS 16), iPhone 14 (iOS 17), iPhone 15 (iOS 18) × macOS / Windows / Linux.
- **Unit:** mobileconfig XML — validate против Apple's profile schema.
- **Integration:** mock lockdownd → проверка sequence calls.
- **Regression:** при каждом релизе — manual smoke на одном iOS device.

## Dependencies
- Task 08 (device manager core).
- Task 02 (CA для profile).
- Task 04 (IPC).
- Task 11 (QR fallback — может разрабатываться параллельно, но cross-link).
