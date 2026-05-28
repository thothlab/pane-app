# PRD-01: MVP Network Debugger (my-charles)

**Статус:** Draft v1
**Дата:** 2026-05-28
**Источник:** [`docs/idea.md`](../../idea.md)
**Связанные документы:** [scratchpad](prd_01_scratch.md), [план задач](prd_01_rep_01_mvp-network-debugger.md)

---

## 1. Objective

Поставить MVP cross-platform desktop-приложения my-charles — HTTPS MITM proxy с современным UI, у которого **подключение нового iOS/Android устройства занимает ≤ 30 секунд через USB-команду**, против 5-15 минут ручной настройки в Charles/Proxyman/Reqable.

MVP считается достигнутым, когда выполнены все Acceptance criteria (§7) на трёх target-платформах (Windows 11, macOS 13+, Ubuntu 22.04+) и собраны installable бинари ≤ 80 MB.

## 2. Non-objectives (MVP)

- **Rewrites / Map Local / Map Remote / Breakpoints** — beyond MVP, отдельные эпики.
- **Throttling / bandwidth simulation** — beyond MVP.
- **WebSocket / SSE / gRPC inspector** — в MVP только маркируем `wss://`/`grpc-web` capture как long-running без detail view.
- **HTTP/3 (QUIC) MITM** — в MVP только детектим `h3` ALPN и показываем hint «downgrade to HTTP/2 чтобы инспектировать»; UDP-proxy не пишем.
- **HAR import / export** — beyond MVP (только in-memory replay в MVP).
- **Wi-Fi-only setup без USB** — есть как fallback (QR + mobileconfig), но не как primary.
- **WireGuard mode** — beyond MVP.
- **Git-native rule sharing** — beyond MVP (нет rules в MVP).
- **Production traffic monitoring / packet capture / load testing** — out of scope (другая ниша).
- **Обход cert pinning** — out of scope, продукт детектит и объясняет, но не лечит.
- **Использование против чужих apps без согласия** — out of scope, в README/UI явный disclaimer + Apache-2.0.

## 3. Data model

### 3.1 Сущности

#### Session
| field | type | notes |
|---|---|---|
| id | UUID | PK |
| started_at | TIMESTAMP | UTC |
| stopped_at | TIMESTAMP NULL | |
| listen_host | TEXT | default `127.0.0.1` |
| listen_port | INT | default `8888` |
| ca_id | UUID FK | → CaCertificate |
| status | ENUM | `starting/running/stopping/stopped/error` |

#### CaCertificate
| field | type | notes |
|---|---|---|
| id | UUID | PK |
| serial | TEXT | unique |
| sha256_fp | TEXT | SHA-256 fingerprint hex |
| subject | TEXT | `CN=my-charles Root CA` |
| valid_from | TIMESTAMP | |
| valid_to | TIMESTAMP | по умолчанию +3 года |
| pem_public | BLOB | публичный сертификат |
| key_ref | TEXT | reference в OS keychain (не сам ключ) |
| revoked_at | TIMESTAMP NULL | |

#### Device
| field | type | notes |
|---|---|---|
| id | UUID | PK |
| session_id | UUID FK | |
| platform | ENUM | `ios/android` |
| connection | ENUM | `usb/wifi` |
| serial | TEXT | UDID для iOS, serial для Android |
| display_name | TEXT | model + name |
| state | ENUM | `pairing/trust_install/proxy_setup/ready/error/removed` |
| ca_installed_at | TIMESTAMP NULL | |
| capabilities_json | JSON | `{rooted: bool, jailbroken: bool, os_version: "17.4"}` |
| last_error | TEXT NULL | |

#### Capture
| field | type | notes |
|---|---|---|
| id | UUID | PK |
| session_id | UUID FK | |
| started_at | TIMESTAMP | |
| ended_at | TIMESTAMP NULL | |
| client_addr | TEXT | `ip:port` |
| server_host | TEXT | |
| server_port | INT | |
| scheme | ENUM | `http/https` |
| http_version | ENUM | `1.1/2/3` |
| method | TEXT | |
| url_path | TEXT | indexed |
| status | INT NULL | |
| req_body_id | UUID NULL FK | → CaptureBody |
| res_body_id | UUID NULL FK | → CaptureBody |
| tls_info_id | UUID NULL FK | → TlsInfo |
| total_bytes | INT | |
| duration_ms | INT NULL | |
| state | ENUM | `opening/in_flight/completed/aborted/error` |
| error_kind | TEXT NULL | enum-like: `pinning/tls_handshake/timeout/connection_refused/protocol` |

**Index:** `(session_id, started_at DESC)`, `(server_host)`, `(status)`, `(method)`.

#### Headers (вложенная таблица для denormalized join)
| field | type | notes |
|---|---|---|
| id | INTEGER PK | |
| capture_id | UUID FK | |
| direction | ENUM | `request/response` |
| name | TEXT | |
| value | TEXT | |
| order_idx | INT | |

#### CaptureBody
| field | type | notes |
|---|---|---|
| id | UUID | PK |
| sha256 | TEXT | dedup |
| encoding | ENUM | `identity/gzip/br/deflate` (как пришло по сети) |
| mime | TEXT NULL | |
| size_bytes | INT | |
| storage | ENUM | `inline/file` |
| inline_blob | BLOB NULL | до 64 KB inline |
| file_path | TEXT NULL | путь относительно `bodies/` |

#### TlsInfo
| field | type | notes |
|---|---|---|
| id | UUID PK | |
| sni | TEXT | |
| alpn | TEXT | `h2/http/1.1/h3` |
| cipher | TEXT | |
| version | TEXT | `TLS1.2/TLS1.3` |
| cert_chain_fps | TEXT[] | SHA-256 fingerprints цепочки сервера |
| pinning_detected | BOOL | |

#### ReplayRecord
| field | type | notes |
|---|---|---|
| id | UUID PK | |
| source_capture_id | UUID FK | |
| edited_request_json | JSON | |
| result_capture_id | UUID NULL FK | → Capture результата (новый capture) |
| created_at | TIMESTAMP | |

#### Filter (saved)
| field | type | notes |
|---|---|---|
| id | UUID PK | |
| name | TEXT | |
| query | TEXT | DSL (см. §5.2) |
| color | TEXT | hex |
| pinned | BOOL | |

#### PinningIncident
| field | type | notes |
|---|---|---|
| id | UUID PK | |
| capture_id | UUID FK | |
| host | TEXT | |
| alpn | TEXT | |
| hint_kind | ENUM | `app_pin/system_pin/ct_required/unknown` |
| occurred_at | TIMESTAMP | |

### 3.2 Хранилище

- **SQLite** WAL mode, файл `~/.local/share/my-charles/captures.db` (XDG paths cross-platform через `directories` крейт).
- **Bodies** хранятся как файлы под `~/.local/share/my-charles/bodies/<sha256-prefix>/<sha256>`.
- **CA private key** — в OS keychain (`security` macOS, Credential Manager Windows, `libsecret` Linux). На диске только PEM публичного.

## 4. API list

Все вызовы — Tauri `invoke` команды. Возвраты — JSON, ошибки — `Result<T, ApiError>`.

### 4.1 Session / Proxy

| command | input | output | errors |
|---|---|---|---|
| `proxy.start` | `{host?: str, port?: int}` | `Session` | `port_in_use`, `ca_missing` |
| `proxy.stop` | `{}` | `{stopped_at}` | `not_running` |
| `proxy.status` | `{}` | `{status, listen, since, captures_count}` | — |

### 4.2 CA

| command | input | output | errors |
|---|---|---|---|
| `ca.current` | `{}` | `CaCertificate` (без приватного ключа) | `no_ca` |
| `ca.rotate` | `{}` | `CaCertificate` (новый) | `keychain_unavailable` |
| `ca.export` | `{format: "pem"\|"der"\|"qr"\|"mobileconfig"}` | `{path: str}` или `{data: base64}` | `unsupported_format` |

### 4.3 Devices

| command | input | output | errors |
|---|---|---|---|
| `device.list_attached_usb` | `{}` | `[{platform, serial, name}]` | `tooling_missing` |
| `device.add_ios_usb` | `{serial: str}` | `Device` | `pairing_denied`, `trust_failed`, `unsupported_ios` |
| `device.add_android_usb` | `{serial: str}` | `Device` | `adb_unauthorized`, `root_required`, `ca_install_failed` |
| `device.remove` | `{id: uuid}` | `{cleaned: bool, pending_cleanup: bool}` | `device_offline` |
| `device.get` | `{id: uuid}` | `Device` | `not_found` |
| `device.list` | `{}` | `Device[]` | — |

### 4.4 Captures

| command | input | output | errors |
|---|---|---|---|
| `capture.list` | `{filter?: str, limit: int, before?: timestamp}` | `Capture[]` (без body) | — |
| `capture.get` | `{id: uuid}` | `Capture` + headers | `not_found` |
| `capture.get_body` | `{body_id: uuid, max_bytes?: int}` | `{mime, encoding, bytes_base64, truncated}` | `not_found` |
| `capture.clear` | `{older_than?: timestamp}` | `{deleted: int}` | — |
| `capture.export_one` | `{id, format: "curl"\|"har_single"}` | `{text}` | — |

### 4.5 Replay

| command | input | output | errors |
|---|---|---|---|
| `replay.send` | `{source_id?: uuid, request: RequestSpec}` | `ReplayRecord` (с `result_capture_id`) | `network`, `tls` |

`RequestSpec`: `{method, url, headers: [{name, value}], body_base64?, body_text?, http_version?}`.

### 4.6 Filters

| command | input | output | errors |
|---|---|---|---|
| `filter.save` | `{name, query, color, pinned}` | `Filter` | `invalid_query` |
| `filter.list` | `{}` | `Filter[]` | — |
| `filter.delete` | `{id}` | `{deleted: true}` | `not_found` |

### 4.7 Events (UI subscribes via Tauri event bus)

| event | payload |
|---|---|
| `capture.started` | `{id, server_host, method, url_path, started_at}` |
| `capture.completed` | `{id, status, duration_ms, total_bytes}` |
| `capture.error` | `{id, error_kind, host}` |
| `device.state_changed` | `{id, state, last_error?}` |
| `pinning.detected` | `{capture_id, host, hint_kind}` |
| `proxy.status_changed` | `{status}` |

## 5. Validation & state transitions

### 5.1 Device state machine

```
new ──pair──▶ pairing ──trust──▶ trust_install ──proxy──▶ proxy_setup ──ok──▶ ready
                │                       │                      │
                fail                    fail                   fail
                ▼                       ▼                      ▼
                                      error  ──user_remove──▶ removed
```

Правила:
- Переход `pairing → trust_install` только если `idevicepair pair` (iOS) или `adb start-server + adb devices` (Android) вернули success.
- Переход в `ready` означает: CA установлен в системе устройства И proxy redirect активен (usbmux tunnel / `adb reverse`).
- `device.remove` пытается revoke CA на устройстве и снять proxy; если устройство offline — оставляет в `removed` с флагом `pending_cleanup=true`.

### 5.2 Capture lifecycle

```
opening ──hdrs──▶ in_flight ──response──▶ completed
   │                  │
   │                  ├──abort──▶ aborted
   │                  └──err────▶ error (error_kind set)
   │
   └──tls_fail──▶ error (error_kind=pinning|tls_handshake)
```

- Capture создаётся в `opening` при TCP accept.
- `state=error, error_kind=pinning` ставится, когда: TLS handshake провален со стороны клиента (rejected серверный leaf), И клиент закрыл соединение в течение 200 ms, И SNI host совпал с известным pinned-pattern (heuristic v1 — отдельный список + fail-fast после ClientHello rejection).

### 5.3 Filter query DSL

Grammar (v1, парсер на `nom`):
```
expr     := term (WS term)*
term     := negation? atom
negation := "!"
atom     := key ":" value | bare_value
key      := "host" | "method" | "status" | "mime" | "path" | "size" | "duration"
value    := quoted_string | bareword | range
range    := number ".." number   // для size, status, duration
```

Примеры:
- `host:api.example.com status:5..`
- `method:POST !host:cdn.*`
- `mime:application/json duration:1000..`

### 5.4 Validation rules

- `proxy.start.port`: 1024-65535, default 8888.
- `replay.send.url`: должен быть absolute URL с `http`/`https` scheme.
- `replay.send.headers`: имена ASCII, значения — UTF-8, max 8 KB на header, max 64 headers.
- `filter.save.name`: 1-64 chars, unique per user.
- `ca.rotate`: запрещён если есть `Device.state=ready` без подтверждения (warning + re-confirm).

## 6. Risks & mitigations

| # | Риск | Вероятность | Impact | Mitigation |
|---|---|---|---|---|
| R1 | `libimobiledevice` нестабилен на iOS 17+ (usbmuxd2 разломан) | High | High | Fallback на Wi-Fi proxy + mobileconfig (Task 11). В MVP UI: если USB-flow упал — автоматически предлагаем QR-fallback. Тестируем на iOS 16 / 17 / 18. |
| R2 | OEM Android (Xiaomi/Huawei/Samsung One UI 6) блокируют CA даже с root | Med | Med | Если установка system-store упала — fallback на dev-сборку через `network_security_config.xml` snippet (генерим + копируем в clipboard + показываем как вставить). Документация по матрице OEM. |
| R3 | HTTP/3 (QUIC) растёт — много трафика не пойдёт через классический MITM | Med | Med (растёт) | MVP: детект `h3` ALPN → downgrade-hint в UI. Полноценный QUIC-MITM — отдельный эпик post-MVP. |
| R4 | Cert pinning у production-apps | Высокая | Med (фундаментальный лимит) | Честная UX: pinning detection + объяснение. Не пытаемся обойти. Документация: ссылка на Frida/Magisk для пользователей которым нужно. |
| R5 | Юридический фронт (использование против чужих apps) | Low | Med | Apache-2.0 + disclaimer в README, About dialog, и первом запуске. UI копирайтинг везде упирает на «свои сборки». |
| R6 | `mitmproxy-rs` API ломается между версиями | Med | Med | Pin minor version в Cargo.toml; обёртка в собственный trait, чтобы свапнуть на собственный engine без переписывания core. |
| R7 | Tauri 2 sidecar bundling cross-platform (libimobiledevice deps) | High | High | Спецзадача (Task 13) на упаковку. Тестируем CI matrix Win/Mac/Linux на каждом merge. Если cross-bundling не получается — выпускаем macOS-первый, остальные следом. |
| R8 | OS Keychain отказывает silently на Linux без keyring daemon | Med | Low | Fallback на encrypted-at-rest файл с derived key + UI-warning «secure store unavailable». |
| R9 | SQLite WAL может разрастаться при 10k+ capture/min нагрузке | Low | Med | Auto-checkpoint каждые N seconds; cap на bodies inline. Бодиs ≥ 64 KB на диск. |
| R10 | Размер бандла > 80 MB из-за sidecars | Med | Low | Минимальный набор `libimobiledevice` (только нужные CLIs), platform-tools только `adb` (без `fastboot`). UPX-сжатие если нужно. |

## 7. Acceptance criteria

Измеримые, проверяемые. Каждый — green-light только при подтверждении ручным или автоматическим тестом.

### AC1 — Cold-start iOS USB ≤ 30 секунд
Запуск приложения с чистой машины (нет ранее установленного CA, нет paired device) → подключаешь iPhone 14 / iOS 17 USB-кабелем → нажимаешь "Add iOS device" → выбираешь устройство → видишь первый capture в Capture list ≤ 30 секунд (P50 по 10 прогонам).

### AC2 — Cold-start Android USB ≤ 30 секунд
Аналогично для Pixel / Android 14 с включённым USB debugging.

### AC3 — CA management
- При первом запуске генерится CA, fingerprint виден в Settings → CA.
- Кнопка "Rotate CA" создаёт новый CA, маркирует старый `revoked`. Если есть active devices — показывается confirm с количеством устройств которые нужно пере-install.
- `ca.export` отдаёт PEM, DER, QR (data URL), mobileconfig (для iOS profile install).

### AC4 — Capture list performance
- 10 000 записей в SQLite → scroll virtualized без визуального лага (≥ 55 fps в DevTools).
- Применение фильтра `host:api.example.com status:5..` к 10k записей ≤ 100 ms (P95 на M1 Air / Ryzen 5 5600U).

### AC5 — Detail panes
- Headers tab: ключи отсортированы, copy-on-click по строке.
- Body tab JSON: pretty-print с фолдингом, syntax highlight, поиск (Cmd/Ctrl+F).
- Body tab image: предпросмотр png/jpg/webp/gif/svg.
- Body tab бинарный: hex dump со сменой режима ASCII / hex / pretty.
- Timing waterfall: DNS / connect / TLS / send / wait / receive с числами в ms.

### AC6 — Replay
- Из capture → "Replay" открывает форму с pre-filled method/url/headers/body.
- Любое поле редактируемо.
- "Send" создаёт новый capture, помечает `ReplayRecord` со связью с source.
- Side-by-side diff (старый vs новый) для headers и body (JSON-aware).

### AC7 — Cert pinning UX
При TLS-handshake failure на pinning-эвристике:
- В Capture list строка с иконкой замка и `error_kind=pinning`.
- Banner-toast: «`api.example.com` использует cert pinning. Inspection невозможна без bypass через Frida/Magisk. Подробнее →» с ссылкой на док.
- В Detail pane карточка с hint_kind и list possible reasons.

### AC8 — Device removal cleanup
"Remove device" в UI → пытается:
- iOS: отозвать profile через `ideviceinstaller` / снять proxy.
- Android: `adb shell` снять system CA (если рутован) и `adb reverse --remove`.
Если устройство offline — статус `removed` + `pending_cleanup=true` + при следующем connect авто-cleanup и тост.

### AC9 — Cross-platform builds
- Windows 11 (x64), macOS 13+ (universal: x64 + arm64), Ubuntu 22.04+ (x64) — три installable артефакта (.msi / .dmg / .deb + AppImage).
- Размер каждого ≤ 80 MB.
- Auto-update механизм работает (Tauri updater) — проверка version, скачивание, верификация подписи.

### AC10 — Honest pinning + boundaries copy
- README, About dialog и первый-запуск-welcome содержат: цель продукта (свои сборки + legitimate security), Apache-2.0, и явный disclaimer про чужие apps.
- При попытке add device — checkbox «I own or have authorization to inspect this device» (anti-yolo, неблокирующий, но в логе).

## 8. Tech stack (зафиксировано для MVP)

- **Shell:** Tauri 2.
- **UI:** SolidJS + Tailwind + solid-router; виртуализация — `@tanstack/solid-virtual`.
- **Proxy engine:** embed `mitmproxy-rs` (latest stable на момент Task 02), обёрнут в trait `ProxyEngine` чтобы можно было свапнуть.
- **TLS:** `rustls` через `mitmproxy-rs`; CA на `rcgen`.
- **Storage:** SQLite через `rusqlite` (bundled feature), WAL mode; миграции через `refinery`.
- **iOS:** sidecar `libimobiledevice` — `idevice_id`, `ideviceinfo`, `idevicepair`, `ideviceimagemounter`, `ideviceinstaller`, `idevicesyslog`. Bundled per-platform.
- **Android:** sidecar `adb` (Android Platform Tools, минимальная сборка).
- **Secrets:** `keyring` крейт (cross-platform OS keychain).
- **Logging:** `tracing` + `tracing-subscriber`, ротация по дням.

## 9. Дорожная карта по эпикам (high-level)

| # | Эпик | Tasks |
|---|---|---|
| E1 | Foundation | Task 01 (bootstrap), Task 03 (storage), Task 04 (IPC) |
| E2 | Proxy core | Task 02 (engine + CA), Task 12 (pinning detection) |
| E3 | Capture UX | Task 05 (list + filter), Task 06 (detail panes), Task 07 (replay) |
| E4 | Device setup ("ставка") | Task 08 (device manager core), Task 09 (iOS USB), Task 10 (Android USB), Task 11 (QR fallback) |
| E5 | Ship | Task 13 (packaging), Task 14 (docs + onboarding) |

## 10. Out of scope deferrals (что будет в beyond-MVP)

- Rewrites engine + Map Local/Remote.
- Breakpoints + pause/edit/resume.
- WebSocket / SSE / gRPC inspector.
- HTTP/3 (QUIC) MITM.
- Throttling / bandwidth simulation.
- HAR import/export.
- Wi-Fi proxy primary mode (без USB).
- WireGuard mode.
- Git-native rules sharing.
- Workspaces / multi-session.

Все они трекаются отдельными PRD после релиза v0.1.
