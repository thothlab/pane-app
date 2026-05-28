# Task 12 — Cert pinning detection + honest UX

## Goal
Когда инспекция не получается из-за cert pinning, не молчать (как Charles), а явно показать пользователю **что произошло, почему, и какие есть пути дальше**. Это одно из ключевых differentiators.

## Scope
**In:**
- Detection эвристика на стороне engine.
- `PinningIncident` запись в БД.
- UI banner / toast / detail-card с объяснением.
- Documentation page (in-app) с FAQ.

**Out:**
- Bypass pinning (Frida/Magisk autoinjection) — explicitly out of scope (см. PRD §2).

## Subtasks

### 12.1 Detection heuristic v1
Срабатывает если ВСЕ true:
- [ ] TLS handshake провален со стороны клиента после нашего ServerHello.
- [ ] Клиент закрыл TCP соединение в течение 500 ms после ServerHello.
- [ ] SNI hostname матчит pinned-pattern из known-list ИЛИ есть повторный faily handshake к тому же host в течение 30 s (≥ 3 раз).
- [ ] ALPN успел согласоваться (= это не сетевая ошибка).

Известные индикаторы:
- Получили `SSL_ALERT` `certificate_unknown` (тип 46) или `bad_certificate` (тип 42).
- TCP RST сразу после Certificate frame.

### 12.2 Known-pinned host list
- [ ] Файл `assets/pinned-hints.json`: list patterns `["api.facebook.com", "*.googleapis.com", "*.apple.com"]` с hint kind (`system_pin` / `app_pin` / `ct_required`).
- [ ] Updatable через простую web-fetch при старте (с offline-cache).

### 12.3 PinningIncident persistence
- [ ] Таблица из PRD §3 — миграция V004.
- [ ] При detection — create row + emit `pinning.detected` event.
- [ ] Dedup: если тот же host triggered за последние 60 s — не спамим UI.

### 12.4 UI banner
- [ ] Top-banner: "Detected cert pinning on `<host>`. Inspection won't work without bypass."
- [ ] CTA: "Learn more" → docs view.
- [ ] Dismissible per host (LocalStorage, 24h).

### 12.5 Capture row indicator
- [ ] Иконка замка в строке списка для capture с `error_kind=pinning`.
- [ ] Tooltip: "Cert pinning detected".

### 12.6 Detail pane card
- [ ] При selected capture с pinning — отдельная карточка:
  - Host, SNI, ALPN.
  - Detected hint (`app_pin` / `system_pin` / `ct_required`).
  - 3-4 bullet points "Why this happens".
  - Список workarounds (с честностью):
    - "If it's your own app: disable pinning in debug build (link to docs)".
    - "If you own the device: Frida / Magisk (external tools, not provided)".
    - "If neither: this app cannot be inspected".

### 12.7 In-app docs page
- [ ] `/about/pinning` — однастраничный markdown view внутри приложения.
- [ ] Sections: "What is cert pinning?", "Why we don't bypass it", "What can you do?".
- [ ] Ссылки на Frida.re, Magisk repo, debug-build guide.

### 12.8 False positive mitigation
- [ ] Если detection срабатывает но через 60 s capture к тому же host прошёл нормально — пометить incident как `false_positive` (e.g. был network glitch).
- [ ] Metric: track FP rate в local stats (только для опционального share).

## Deliverables
- `crates/mycharles-pinning/` — detector логика.
- `assets/pinned-hints.json`.
- `src/views/captures/pinning-card/`.
- `src/views/about/pinning-docs.tsx`.

## Definition of Done
- [ ] AC7 из PRD: pinning capture видны в списке с иконкой, banner с CTA, detail card с объяснением.
- [ ] Detector тестируется на demo-app с certificate pinning (e.g. собрать тестовый APK с OkHttp CertificatePinner).
- [ ] False positive rate ≤ 5% на baseline traffic (Chrome browsing 100 sites).
- [ ] Docs page рендерится корректно.

## Tests
- **Unit:** detector heuristic on synthetic event sequences.
- **Integration:** локальный test server, который сначала отдаёт нормальный TLS, потом cert-pinning симуляцию (через настройку клиента).
- **Manual:** real-world apps с известным pinning (test-mode только на своих сборках).

## Dependencies
- Task 02 (engine — для PinningSuspected events).
- Task 03 (storage — миграция V004).
- Task 04 (IPC).
- Task 05 + 06 (UI integration).
