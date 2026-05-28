# Task 11 — QR-based Wi-Fi fallback setup

## Goal
Когда USB не работает (iOS без developer mode, Linux без udev rules, удалённое устройство), пользователь может зайти на одну страницу с QR-кодом, отсканировать его телефоном и за 2 экрана инструкций получить рабочую setup'ку.

## Scope
**In:**
- Local HTTP setup server (на отдельном порту, e.g. `127.0.0.1:8889`, доступный в LAN).
- Setup page: iOS-specific (deeplink to install mobileconfig) и Android-specific (PEM + instructions).
- QR с URL на эту страницу.
- Mobileconfig для iOS (re-use из Task 09).
- LAN-aware hostname (показываем `http://<lan-ip>:8889/setup` чтобы device на той же сети мог открыть).

**Out:**
- HTTPS для setup server'а (это локальный fallback; HTTP с warning OK).
- Remote setup поверх Tailscale/WireGuard — beyond MVP.

## Subtasks

### 11.1 Setup server (axum)
- [ ] Tokio task, listens на `0.0.0.0:8889` (bind на всех интерфейсах для LAN access).
- [ ] Routes:
  - `GET /setup` — landing с device detection (UA-based) → iOS / Android branches.
  - `GET /setup/ios/profile.mobileconfig` — даёт подписанный профиль.
  - `GET /setup/android/ca.pem` — даёт PEM root CA.
  - `GET /setup/android/instructions` — HTML wizard.
  - `GET /setup/status` — long-poll: показывает host'у "device X opened setup", "profile installed".
- [ ] CSRF-token в URL (одноразовый на сессию setup'а) — чтобы случайный соседский браузер не получил CA.

### 11.2 QR generation
- [ ] Crate `qrcode` — generate PNG/SVG data URL.
- [ ] URL содержит LAN IP + port + session token: `http://192.168.1.42:8889/setup?t=abc123`.
- [ ] Determine LAN IP cross-platform (filter loopback, prefer Wi-Fi interface).

### 11.3 LAN IP picker
- [ ] Если несколько интерфейсов — UI dropdown в "Add device → Wi-Fi" с выбором.
- [ ] По умолчанию — самый "правдоподобный" (private subnet, RFC1918, не VPN).

### 11.4 iOS branch
- [ ] `/setup/ios` →  Safari открывает → автомат deeplink на `profile.mobileconfig`.
- [ ] Mobileconfig: root CA + Wi-Fi proxy payload (LAN IP : 8888).
- [ ] HTML инструкция (2 экрана с гифками): "Install Profile" → "Trust Certificate".
- [ ] Auto-detect когда trust завершён (через mTLS probe или TLS handshake к собственному endpoint).

### 11.5 Android branch
- [ ] `/setup/android` → wizard:
  1. Download `ca.pem`.
  2. Settings → Security → Encryption & Credentials → Install a certificate → CA certificate. (deep-link если возможно через `Intent.ACTION_VIEW`).
  3. Wi-Fi settings → modify network → proxy manual → host:port.
- [ ] Альтернатива: предложить установить через ADB если есть кабель ("Plug in for one-click").

### 11.6 Live status в desktop UI
- [ ] Desktop показывает "Waiting for device..." после показа QR.
- [ ] При `/setup/status` updates — UI меняет stage: "Profile downloaded" → "Trust granted" → "Proxy active".
- [ ] Если в течение 10 минут ничего — close QR с error UX.

### 11.7 Security
- [ ] Setup server убивается через 15 минут или после успешного pair.
- [ ] Token одноразовый.
- [ ] CA PEM endpoint — protected token'ом.
- [ ] Warning в UI: "Setup server is on your LAN — close once device connected".

## Deliverables
- `crates/mycharles-setup-server/`.
- `src/views/devices/qr-fallback/`.
- mobileconfig-template из Task 9 переиспользуем.

## Definition of Done
- [ ] Pixel 8 без USB → QR scan → wizard → first capture ≤ 90 s (более slow path).
- [ ] iPhone 15 без USB → QR scan → profile install → trust → first capture ≤ 90 s.
- [ ] Setup server недоступен после успешного pair или 15 мин timeout.
- [ ] Token-protection: запрос без token → 403.
- [ ] LAN IP detection не выбирает VPN-tap interface.

## Tests
- **Unit:** LAN IP picker — feed список интерфейсов, проверка приоритизации.
- **Integration:** http request к setup-server без token → 403, с валидным → 200.
- **Manual:** iPhone + Pixel на той же Wi-Fi сети, через QR — end-to-end.

## Dependencies
- Task 02 (CA).
- Task 08 (device manager — для unified state machine).
- Task 09 (mobileconfig generator).
