# Task 06 — Detail panes (headers / body / timing)

## Goal
Правая (или нижняя) панель: подробности выбранного capture. Tab'ы Headers / Body / Timing / TLS. Body view знает про JSON / HTML / XML / image / hex.

## Scope
**In:**
- Tab bar: Overview, Request, Response, Timing, TLS.
- Headers с сортировкой/копированием.
- Body: JSON pretty-print + fold + search; HTML/XML pretty; image preview; hex dump для бинарного; auto-detect по mime.
- Timing waterfall (DNS / connect / TLS / send / wait / receive).
- TLS info card (SNI, ALPN, cipher, version, cert chain summary).
- Streaming body load (truncate ≥ 5 MB с "Load full" button).

**Out:**
- Edit body (это Replay → Task 07).
- WebSocket frame stream view — beyond MVP.

## Subtasks

### 6.1 Tab shell
- [ ] `<DetailPanes capture={selected}>` с tab'ами.
- [ ] При смене selected — preserve tab choice (sticky).
- [ ] Split-pane resize между list и detail (`solid-split-pane`).

### 6.2 Overview tab
- [ ] Карточка: status, method, full URL, host, scheme, http_version, total_bytes, duration_ms.
- [ ] Если `error_kind=pinning` — карточка-banner с CTA "Why pinning?" (Task 12).

### 6.3 Headers tab
- [ ] Two-column таблица для request/response (или toggle).
- [ ] Click-to-copy на любой row (header value).
- [ ] Кнопка "Copy as cURL" (генерим из request).
- [ ] Подсветка известных headers (Content-Type, Authorization, Set-Cookie).

### 6.4 Body view — JSON
- [ ] Monaco Editor (light) или CodeMirror 6 с JSON language.
- [ ] Pretty-print по умолчанию.
- [ ] Fold/unfold all.
- [ ] Поиск (Cmd/Ctrl+F).
- [ ] "View as raw" toggle.

### 6.5 Body view — HTML/XML
- [ ] CodeMirror 6 + html/xml language.
- [ ] Pretty (через `prettier` standalone).
- [ ] Preview-iframe toggle (sandboxed, no JS, no network).

### 6.6 Body view — image
- [ ] PNG/JPEG/WebP/GIF/SVG — `<img>` со sandboxed src (data URL).
- [ ] Info: dimensions, file size.

### 6.7 Body view — binary / fallback
- [ ] Hex dump (16 bytes per row), ASCII column.
- [ ] Toggle: hex / ascii / try-utf8.
- [ ] Header "Looks like <type>?" если magic bytes recognized (e.g. `PK` → zip).

### 6.8 Body streaming + truncation
- [ ] При first render — fetch first 256 KB (`capture.get_body({max_bytes:262144})`).
- [ ] Если truncated — banner "Showing first 256 KB of 4.2 MB" + "Load full".
- [ ] Full load с прогрессом (для очень больших — chunked).

### 6.9 Timing waterfall
- [ ] Горизонтальный bar с разноцветными секциями.
- [ ] Численные значения справа.
- [ ] Если фаза отсутствует (e.g. connection reused) — серая полоска "reused".

### 6.10 TLS tab
- [ ] SNI, ALPN, TLS version, cipher.
- [ ] Cert chain: subject CN + issuer + valid range + SHA-256 fingerprint.
- [ ] Если `pinning_detected=true` — карточка с объяснением (см. Task 12).

## Deliverables
- `src/views/captures/detail/` с компонентами по табам.
- Vitest + Storybook entries.

## Definition of Done
- [ ] AC5 из PRD: headers sort + copy, JSON pretty + fold + search, image preview, hex dump.
- [ ] Body ≥ 5 MB truncates по умолчанию, "Load full" работает с прогрессом.
- [ ] cURL-копирование валидно (тест: вернуть curl через `bash -c` — получаем ответ).
- [ ] Timing waterfall показывает все фазы с числами.
- [ ] Tab choice sticky при смене capture.

## Tests
- **Unit:** mime-detector (по `Content-Type` + magic bytes).
- **Unit:** cURL serializer (escape quotes, multiline body).
- **Component:** JSON view с large response (1 MB).
- **E2E:** select capture → переключение всех tab'ов без ошибок.

## Dependencies
- Task 03 (storage `capture.get_body`).
- Task 04 (IPC).
- Task 05 (capture list — для selected state).
