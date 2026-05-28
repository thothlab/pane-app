# Task 07 — Replay (compose + send + diff)

## Goal
Из любого capture открыть редактируемый "composer", изменить url/method/headers/body, отправить, получить новый capture, увидеть diff со старым. Это базовая ценность «API client built-in».

## Scope
**In:**
- Композер из capture: pre-fill, full edit.
- Headers editor: add/remove/edit rows.
- Body editor: text/JSON/binary (paste base64 или upload).
- Send via internal HTTP client (`reqwest` через proxy сам, чтобы и replay capture попал в общий лог).
- New capture создаётся как нормальный, плюс `ReplayRecord` со связью на source.
- Side-by-side diff (старый vs новый): headers diff, body JSON-aware diff.
- "Send N times" (1-10) — для проверки idempotency.

**Out:**
- Авторизация helper'ы (OAuth, AWS sig) — beyond MVP.
- Persisted collections / environments — beyond MVP.
- Pre-request scripts — beyond MVP.

## Subtasks

### 7.1 Composer UI
- [ ] Modal или dedicated view (`/replay/:capture_id`).
- [ ] Method dropdown (GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS + custom).
- [ ] URL input (валидация absolute http(s)).
- [ ] Headers table editor: add row, remove, sort.
- [ ] Body editor (Monaco/CodeMirror, режимы text/json/form/raw).
- [ ] Param tab (query string editor — синхронен с URL).

### 7.2 Send pipeline
- [ ] Tauri command `replay.send` — берёт RequestSpec, делает HTTP-вызов через ВНУТРЕННИЙ `reqwest::Client`, прокинутый через наш собственный proxy (так replay тоже захватывается).
- [ ] Альтернатива (если loopback кривой): обход proxy, но manual insert в captures + tag `source=replay`.
- [ ] Возвращает `ReplayRecord { result_capture_id }` после получения response.

### 7.3 Send N times
- [ ] Toggle "Send ×N" (1-10).
- [ ] При N>1 — sequential (не concurrent в MVP), progress bar.
- [ ] Все N капчуров линкуются на один `ReplayRecord`.

### 7.4 Diff view
- [ ] Two-pane: source capture слева, result справа.
- [ ] Headers diff: added (green) / removed (red) / changed (yellow).
- [ ] Body diff:
  - JSON: structural diff (`jsondiffpatch` или собственный — key by key).
  - Text/HTML: line diff (`diff-match-patch` или встроенный).
  - Binary: только metadata (size, sha) + "binary differ" badge.

### 7.5 Save & re-use (легкий)
- [ ] "Pin replay" — добавляет в sidebar "Saved replays" (не collections — просто пины, до 20 штук).
- [ ] Click pinned → открывает composer pre-filled.

### 7.6 Auth helper (минимум)
- [ ] Кнопка "Use Authorization from another capture" — выбор capture → копирует только `Authorization` header.

## Deliverables
- `src/views/replay/` с компонентами Composer, Diff, PinnedList.
- Tauri command `replay.send` (имплементация поверх `reqwest`).
- ReplayRecord persistence в storage (новая таблица — migration V002).

## Definition of Done
- [ ] AC6 из PRD: pre-fill + edit + send + diff.
- [ ] Replay сам захватывается как capture (если шлём через свой proxy) и виден в основном списке с tag `replay`.
- [ ] Diff корректно показывает JSON struct diff на nested objects.
- [ ] Send N=5 идёт последовательно, прогресс виден.
- [ ] При ошибке сети — composer показывает error inline без потери введённых данных.

## Tests
- **Unit:** JSON diff на edge-cases (added/removed keys, type change, arrays of objects).
- **Unit:** RequestSpec → wire bytes (HTTP/1.1 serialization для отладки).
- **Integration:** echo-server → composer → send → assert diff показывает изменённый header.
- **E2E:** open capture → Replay → edit method GET→POST + add body → send → проверить новый capture в списке.

## Dependencies
- Task 02 (proxy engine — для self-proxy replay).
- Task 03 (storage — ReplayRecord persistence).
- Task 04 (IPC).
- Task 05 + 06 (выбор source capture).
