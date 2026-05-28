# Task 03 — Storage layer (SQLite + body blobs)

## Goal
Дать engine надёжный sink для captures: SQLite со схемой из PRD §3, body-blob storage с дедупликацией по sha256, миграции, индексы, ретеншн-политика. UI должен мочь читать 10 000 капчуров без лагов.

## Scope
**In:**
- SQLite schema (tables + indexes).
- Миграции (refinery).
- Body-blob storage (inline ≤64 KB, иначе файлом).
- Стриминговая запись response body (без буферизации всего в RAM).
- Ретеншн: cap по дискам / по времени / ручной clear.
- Async API через `tokio::task::spawn_blocking` (rusqlite — sync).

**Out:**
- Чтение из UI напрямую — UI ходит через Tauri commands из Task 04.
- Replay storage — Task 07.

## Subtasks

### 3.1 Schema + миграции
- [ ] Crate `crates/mycharles-storage/`.
- [ ] `refinery` миграции в `migrations/V001__init.sql` (все таблицы из PRD §3.1).
- [ ] Все индексы из PRD.
- [ ] WAL mode `PRAGMA journal_mode=WAL`, `synchronous=NORMAL`.

### 3.2 Body blob storage
- [ ] Trait `BodyStore { put_streaming(reader) -> BodyId; get_streaming(id) -> Reader }`.
- [ ] Inline: ≤ 64 KB → BLOB в SQLite (column `inline_blob`).
- [ ] File: → `<data_dir>/bodies/<sha-prefix-2>/<sha>`.
- [ ] Дедуп: при `put` сначала пишем в temp, считаем sha256, переименовываем в final path (atomic rename). Если файл с этим sha уже есть — дискарам temp.
- [ ] GC: при `clear_captures` находим осиротевшие body id и удаляем.

### 3.3 Capture writer
- [ ] `CaptureWriter::start(meta) -> WriterHandle`.
- [ ] `WriterHandle::request_body_chunk(bytes)`, `response_body_chunk(bytes)`.
- [ ] `WriterHandle::complete(meta_finish)` — atomic commit row + связь с body blob.
- [ ] Backpressure: если writer не успевает — engine ждёт; UI получает уже committed данные.

### 3.4 Async reader API
- [ ] `Captures::list(filter, limit, before)`.
- [ ] `Captures::get(id)` — header-row + список headers.
- [ ] `Captures::body(id, max_bytes)` — потоково отдаёт первые N байт + truncated flag.
- [ ] `Captures::clear(older_than?)`.

### 3.5 Retention
- [ ] Config: `max_total_size_mb` (default 5 GB), `max_age_days` (default 14), `max_count` (default 100 000).
- [ ] Фоновая задача `tokio::interval(60s)` — проверка и удаление сверху самых старых.

### 3.6 Filter DSL parser
- [ ] `nom`-based parser для grammar из PRD §5.3.
- [ ] Компилятор DSL → SQL `WHERE`-fragment (prepared statements, без SQL injection).
- [ ] Тесты на каждый узел grammar и edge cases (`!host:cdn.*`, range `5..`).

## Deliverables
- `crates/mycharles-storage/` с публичным API.
- Миграции V001.
- Unit + integration test suite.
- Бенчмарк: insert rate ≥ 5 000 capture/s (без body), ≥ 1 000 capture/s с body avg 50 KB.

## Definition of Done
- [ ] Все таблицы из PRD созданы и проиндексированы.
- [ ] Body blob dedup verified (положить один и тот же 1 MB файл дважды → один файл на диске).
- [ ] Concurrent insert (10 потоков по 1000 капчуров) без deadlock'ов.
- [ ] Retention срабатывает: после превышения cap удаляются старейшие записи + их blob'ы.
- [ ] Filter DSL parser проходит 30+ unit-тестов.
- [ ] `cargo clippy` clean.

## Tests
- **Unit:** schema migration up/down.
- **Unit:** body store inline vs file boundary (63 KB → inline, 65 KB → file).
- **Integration:** writer → reader на 1000 капчуров, проверка целостности sha256.
- **Integration:** retention при наполнении до cap.
- **Property:** filter DSL — генерируем queries, проверяем что SQL parsable + результат симметричен по семантике (через embedded sqlite).

## Dependencies
- Task 01.
