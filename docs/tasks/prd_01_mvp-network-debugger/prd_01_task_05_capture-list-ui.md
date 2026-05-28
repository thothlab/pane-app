# Task 05 — Capture list UI + фильтр/поиск

## Goal
Левая (или верхняя) панель приложения: live-обновляемый список captures с virtualized scroll, фильтр-баром на DSL из PRD §5.3, сохранёнными фильтрами в sidebar, и быстрыми колоночными сортами.

## Scope
**In:**
- Виртуализированный list (10k+ rows, ≥55 fps scroll).
- Колонки: status, method, host, path, type (mime), size, duration, time.
- Live updates через event `capture.completed` (batched).
- Filter bar с DSL autocomplete.
- Saved filters в sidebar с count badges.
- Sort by column (client-side для текущей странички, server-side для большой выборки).
- Multi-select (для bulk clear/export).
- Pause/resume capture stream (без остановки proxy — UI-only freeze).

**Out:**
- Detail panes — Task 06.
- Replay — Task 07.
- Map Local rules в фильтр-баре — beyond MVP.

## Subtasks

### 5.1 Виртуализация
- [ ] `@tanstack/solid-virtual` (window: 50, overscan: 10).
- [ ] Row height фиксированный (32px) — упрощает виртуализацию.
- [ ] Тест: 50 000 рядов, scroll-jank ≤ 16ms кадр.

### 5.2 Колонки + sort
- [ ] Resizable columns (`solid-resizable-panels` или custom).
- [ ] Persisted column widths в localStorage.
- [ ] Sort indicator (стрелка) + click-to-toggle.
- [ ] Сорт по `started_at` (default), `duration_ms`, `total_bytes`, `status`, `server_host`.

### 5.3 Filter bar
- [ ] Input с подсветкой DSL (Lezer/Tree-sitter mini, либо regex highlighter).
- [ ] Live preview: на каждое нажатие — debounced 200 ms → `capture.list({filter})`.
- [ ] Кнопка "Save filter" → modal с name/color/pinned.
- [ ] Autocomplete на keys (`host:`, `method:`, `status:`, `mime:`, `path:`, `size:`, `duration:`).
- [ ] Validation: parse error показывается inline под input'ом.

### 5.4 Saved filters sidebar
- [ ] List в левом sidebar (под device list).
- [ ] Per-filter badge с count (запрашиваем `capture.list({filter, limit: 1, count_only: true})` — нужен extension в Task 04).
- [ ] Drag-to-reorder (`@thisbeyond/solid-dnd`).
- [ ] Click → применяет фильтр.

### 5.5 Live updates + pause
- [ ] Подписка на `capture.completed` (batched) → append к локальной store.
- [ ] Pause toggle в toolbar — копит batch в buffer, на resume merge'им.
- [ ] Auto-scroll-to-top toggle (sticky когда юзер прокрутил вверх).

### 5.6 Bulk actions
- [ ] Shift-click range select, Cmd/Ctrl-click toggle.
- [ ] Toolbar при ≥1 selected: "Clear selected", "Export selected (HAR)" — HAR-export в MVP только на этой кнопке (single-shot).

### 5.7 Empty/error states
- [ ] Empty при `proxy.status = stopped`: CTA "Start proxy".
- [ ] Empty при `running + 0 captures`: hint "Setup device → docs link".
- [ ] Error при `capture.error` в стриме: красная иконка в строке + hover tooltip с `error_kind`.

## Deliverables
- `src/views/captures/` с компонентами `List`, `Row`, `FilterBar`, `Toolbar`, `SavedFilters`.
- Унит-тесты Vitest.
- Storybook (или Histoire) entries для каждого компонента.

## Definition of Done
- [ ] AC4 из PRD: 10k записей scroll ≥ 55 fps, фильтр ≤ 100 ms.
- [ ] DSL парсер валидирует input live, ошибки кликабельные (jump to position).
- [ ] Saved filters persist через restart.
- [ ] Pause/resume не теряет captures (всё что было captured — попадает в store на resume).
- [ ] A11y: вся таблица — keyboard navigable (arrow keys, enter to open detail).

## Tests
- **Unit:** DSL parser ↔ UI highlighter консистентность.
- **Component:** `<FilterBar>` с фейковыми events, проверка debounce.
- **Performance:** Lighthouse / DevTools profiler — scroll 50k rows.
- **E2E (Playwright):** start mock proxy → emit 100 captures → проверить что все видны → применить фильтр → проверить count.

## Dependencies
- Task 04 (IPC).
- Task 03 (storage) — для не-stub `capture.list`.
