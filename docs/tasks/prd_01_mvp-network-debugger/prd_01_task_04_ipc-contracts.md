# Task 04 — IPC contracts (Tauri commands + event stream)

## Goal
Зафиксировать жёсткий контракт между UI (Solid) и Rust-core, чтобы UI можно было разрабатывать параллельно engine. Все команды, события, типы — в одном месте, кодогенерация TS-типов из Rust.

## Scope
**In:**
- Все команды из PRD §4 (proxy, ca, devices, captures, replay, filters).
- Все события из PRD §4.7.
- Кодогенерация TS-типов из Rust через `ts-rs` или `specta`.
- Error handling convention.
- Backpressure для event stream (UI медленнее engine).

**Out:**
- Имплементация команд по сути — каждая в своём task (proxy → 02, capture → 03, device → 08-10, etc.). Здесь только wire-up + контракт.

## Subtasks

### 4.1 Команды proxy/ca/captures/replay/filters
- [ ] Зарегистрировать в `tauri::Builder` (через `generate_handler!`).
- [ ] Каждая команда → `Result<T, ApiError>`.
- [ ] Типы DTO в `crates/mycharles-ipc/` (workspace).

### 4.2 ApiError
- [ ] Enum с `kind: String` (стабильный) + `message: String` (UI-friendly) + `details: Option<JSON>`.
- [ ] Mapping из внутренних errors (e.g. `engine::Error::PortInUse` → `ApiError { kind: "port_in_use", ... }`).

### 4.3 Event bus
- [ ] `tauri::Window::emit_all("event_name", payload)`.
- [ ] Throttle: для `capture.completed` группируем по 50 ms — UI получает batch (`Vec<CaptureRow>`) чтобы не рендерить по событию.
- [ ] Если UI отстаёт > 1000 событий в очереди — переходим в "missed mode": UI вызывает `capture.list` для refresh.

### 4.4 TS codegen
- [ ] `specta` или `ts-rs` derive macros на DTO.
- [ ] Build-step генерит `src/ipc/types.ts`.
- [ ] CI gate: если Rust-структура изменилась и TS не пересобран — fail.

### 4.5 IPC client wrapper (UI side)
- [ ] `src/ipc/client.ts` — обёртки `invoke<T>(cmd, args)`.
- [ ] `src/ipc/events.ts` — реактивные сигналы Solid (`createSignal` + listener auto-unsubscribe).
- [ ] Test harness: моки команд для UI dev (`vite-plugin-mock`).

### 4.6 Версионирование
- [ ] Заголовок `X-MyCharles-IPC-Version: 1` в каждом call (для будущей миграции).
- [ ] При несовпадении — UI показывает "core/UI version mismatch, please update".

## Deliverables
- `crates/mycharles-ipc/` с DTO + codegen.
- `src/ipc/types.ts` (generated).
- `src/ipc/client.ts`, `src/ipc/events.ts`.
- `docs/ipc-contract.md` — человекочитаемая таблица (зеркалит PRD §4).

## Definition of Done
- [ ] Все 17 команд из PRD §4 зарегистрированы и возвращают stub (until имплементированы в task-owner'ах).
- [ ] Все 6 событий из PRD §4.7 имеют DTO + emit infrastructure.
- [ ] TS-codegen работает: `pnpm build` падает если DTO рассинхронизирован.
- [ ] E2E happy path: UI → `invoke("proxy.start")` → получает Session stub → видит event `proxy.status_changed`.

## Tests
- **Unit:** ApiError mapping для каждого внутреннего error kind.
- **Integration:** Tauri test harness — invoke каждой команды, проверка что возвращает корректную shape (даже если stub).
- **Codegen:** изменить Rust DTO → проверить что TS файл переписался корректно.

## Dependencies
- Task 01 (bootstrap).
- Желателен Task 02/03 для не-stub имплементации, но контракт можно зафиксировать раньше.
