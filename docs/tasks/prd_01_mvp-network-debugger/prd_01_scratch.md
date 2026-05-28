# Scratchpad — PRD-01: MVP Network Debugger (my-charles)

> Рабочий черновик анализа. Не финальный документ.

## Проблема и пользователи

**Проблема:** существующие network-debugger'ы (Charles, Proxyman, Reqable, mitmproxy) требуют 5-15 минут ручной настройки на каждом мобильном устройстве: установка CA в двух местах iOS, network_security_config на Android, Wi-Fi proxy, разбор cert pinning. Этот setup-барьер отсекает большинство пользователей, которым «надо просто посмотреть запросы».

**Primary users:**
- Mobile-разработчик (Android/iOS), отлаживающий dev-сборку своего приложения.
- Backend / full-stack dev, проверяющий запросы своего фронта.
- QA, повторяющий баг с подменой ответа.

**Secondary:** security researcher / reverse-engineer (легальные кейсы).

**Не target:** production network monitoring, packet capture, обход cert pinning в чужих apps без согласия.

## Scope

### In (MVP)
1. HTTPS MITM proxy локально.
2. Auto-generated root CA + экспорт (QR / файл / USB push).
3. Capture list (URL, method, status, timing, размер).
4. Detail panes (headers, body — JSON pretty / image / hex; timing waterfall).
5. Filter / search по URL / method / status / host.
6. Replay (отредактировать и переотправить).
7. **One-command device setup** для iOS USB и Android USB.
8. Cert pinning detection + честное объяснение failure.

### Out (явно)
- Map Local / Map Remote / Rewrites / Breakpoints.
- Throttling / bandwidth simulation.
- WebSocket / SSE / gRPC inspector (вне MVP).
- HAR import / export.
- Wi-Fi-only mode без USB (есть только как fallback).
- WireGuard mode.
- Production monitoring, packet capture, load testing.
- Обход cert pinning.

## Domain model (ключевые сущности)

- **Session** — running proxy instance: порт, CA fingerprint, время старта, набор подключённых устройств.
- **Device** — подключённое устройство: id, type (ios/android), connection (usb/wifi), state (pairing/ready/error), trust_status (ca_installed/missing/unknown), capabilities (rooted/jailbroken).
- **CaCertificate** — root CA: serial, fingerprint (SHA-256), valid_from, valid_to, private_key_path (encrypted на диске).
- **Capture** — единичный transaction: id, timestamp_start, timestamp_end, client_ip, client_port, server_host, server_port, scheme, http_version (1.1 / 2 / 3), request, response, tls_info, error.
- **Request** — method, url, path, query, headers (List<Header>), body_blob_id, body_size, body_mime.
- **Response** — status, headers, body_blob_id, body_size, body_mime.
- **TlsInfo** — sni, alpn, cipher_suite, cert_chain_fingerprints, pinning_detected (bool).
- **CaptureBody** — blob: id, content_hash, encoding (raw/gzip/br), size, path_on_disk.
- **ReplayRecord** — sourced_from_capture_id, modified_request, response, timestamp.
- **Filter** — saved filter: name, query (DSL), color tag.
- **PinningIncident** — host, captured_at, alpn, sni, cert_chain_hash, hint_text.

### Связи
- Session 1—N Capture.
- Session 1—N Device.
- Session 1—1 CaCertificate.
- Capture 1—1 Request, 1—0..1 Response (response может отсутствовать при error/abort), 1—0..1 TlsInfo, 1—N PinningIncident (0 или 1 обычно).
- Request 1—1 CaptureBody, Response 1—1 CaptureBody.
- ReplayRecord 1—1 Capture (source).

## Integration / API needs

### Внешние интеграции
- **`mitmproxy-rs`** (embed как Rust dependency) — TLS termination + HTTP/1.1, HTTP/2 parsing. (Решение зафиксировать в Technical Notes — но MVP берёт embed как baseline.)
- **`libimobiledevice`** — sidecar бинарь (ideviceinstaller, idevicepair, ideviceinfo, idevicesyslog). FFI слишком хрупкий cross-platform; идём через CLI.
- **`adb`** — bundled бинарь (Android Platform Tools, минимальный набор).
- **`rcgen`** — генерация CA и leaf-сертификатов на лету.
- **`rustls`** — TLS на стороне MITM.

### Internal IPC (Tauri commands + events)
- Команды UI → engine: `start_proxy`, `stop_proxy`, `list_captures`, `get_capture(id)`, `get_body(id)`, `replay(modified_request)`, `add_device_usb_ios`, `add_device_usb_android`, `remove_device(id)`, `export_ca(format)`, `clear_captures`, `save_filter`.
- События engine → UI: `capture.started`, `capture.completed`, `capture.error`, `device.state_changed`, `pinning.detected`, `proxy.status_changed`.

## Lifecycle / status модели

### Device state machine
```
new → pairing → trust_install → proxy_setup → ready
                       │              │
                       ↓              ↓
                    error          error
                       │              │
                       └──→ removed ←─┘
```

### Capture state machine
```
opening → request_received → request_sent → response_received → completed
                                  │                   │
                                  ↓                   ↓
                               aborted             error
```

### Session state
`stopped → starting → running → stopping → stopped` (linear, + `error` ветка с любой точки).

## Acceptance criteria (high-level — детализация в PRD)

1. Свежая установка → запуск приложения → нажатие "Add iOS device" с подключённым по USB iPhone → первый capture виден в списке за ≤ 30 секунд (P50).
2. Аналогично для Android (USB + USB debugging).
3. CA генерится при первом запуске; fingerprint виден в Settings; ротация по кнопке работает (старый отзывается, новый ставится на подключённые устройства).
4. Capture list выдерживает 10 000 записей без UI-лагов (виртуализация); фильтр по host применяется ≤ 100 ms.
5. Replay сохраняет тело запроса как было, позволяет редактировать method/url/headers/body и переотправляет; результат отображается рядом с оригиналом.
6. При cert pinning failure пользователь видит явный toast/банер с хостом и текстом-объяснением, а не пустой ряд в списке.
7. Удаление device из UI снимает CA и proxy settings на устройстве (best-effort; если устройство офлайн — отметить как «pending cleanup»).
8. Tauri-сборки под Windows / macOS / Linux, размер бандла ≤ 80 MB.

## Open questions (закрыть в Technical Notes)

- HTTP/3 (QUIC) — в MVP: только детект и downgrade-hint в логе; UDP-MITM откладываем.
- WebSocket — в MVP отображается как один «long-running» capture без detail pane; полноценный inspector — beyond MVP.
- Шифрование private key для CA на диске — пароль / OS keychain? Делаем OS keychain (macOS Keychain, Windows Credential Manager, Linux libsecret).
- iOS 17+ usbmuxd2 — план B на случай нестабильности: переходим на Wi-Fi-proxy fallback + mobileconfig (см. Task 11).

## Risks (детально в PRD)

- libimobiledevice на iOS 17+
- adb / OEM-блокировки CA на Android 14+
- TLS 1.3 + HTTP/3 рост
- Cert pinning как фундаментальный лимит
- Юридический фронт (license + disclaimer)
