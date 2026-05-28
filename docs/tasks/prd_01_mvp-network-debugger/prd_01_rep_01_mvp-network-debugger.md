# Report 01 — Planning for PRD-01: MVP Network Debugger (my-charles)

**Дата:** 2026-05-28
**Источник:** [`docs/idea.md`](../../idea.md)

## What was done

Превратил рабочий vision-документ `docs/idea.md` (Charles-конкурент с акцентом на one-command device setup) в формальный delivery-пакет:

1. **Scratchpad** (`prd_01_scratch.md`) — анализ: проблема, пользователи, scope in/out, domain model (10 сущностей), integrations (mitmproxy-rs / libimobiledevice / adb / rcgen / rustls), lifecycle state machines (Device, Capture), acceptance criteria, open questions.
2. **PRD** (`prd_01_mvp-network-debugger.md`) — детальная спецификация по гейту качества:
   - Objective + Non-objectives.
   - Data model: 10 таблиц с полями, типами, индексами; SQLite + body-blob storage в файлах.
   - API: 17 Tauri-команд (`proxy.*`, `ca.*`, `device.*`, `capture.*`, `replay.*`, `filter.*`) + 6 событий.
   - Validation & state transitions: device FSM, capture FSM, filter DSL grammar.
   - Risks (10 пунктов с mitigations).
   - 10 acceptance criteria — измеримые.
   - Зафиксирован tech stack (Tauri 2 + SolidJS + mitmproxy-rs + rusqlite + keyring).
3. **14 task-файлов**, организованные по 5 эпикам:
   - **E1 Foundation:** 01 bootstrap, 03 storage, 04 IPC.
   - **E2 Proxy core:** 02 engine+CA, 12 pinning detection.
   - **E3 Capture UX:** 05 list+filter, 06 detail panes, 07 replay.
   - **E4 Device setup (ставка):** 08 device manager core, 09 iOS USB, 10 Android USB, 11 QR fallback.
   - **E5 Ship:** 13 packaging, 14 docs+onboarding.

Каждая задача содержит Goal / Scope (in/out) / Subtasks / Deliverables / DoD / Tests / Dependencies. Зависимости между задачами явно прописаны.

## Files produced

```
docs/tasks/prd_01_mvp-network-debugger/
├── prd_01_scratch.md
├── prd_01_mvp-network-debugger.md          # PRD
├── prd_01_task_01_project-bootstrap.md
├── prd_01_task_02_proxy-engine-ca.md
├── prd_01_task_03_storage-layer.md
├── prd_01_task_04_ipc-contracts.md
├── prd_01_task_05_capture-list-ui.md
├── prd_01_task_06_detail-panes.md
├── prd_01_task_07_replay.md
├── prd_01_task_08_device-manager-core.md
├── prd_01_task_09_ios-usb-setup.md
├── prd_01_task_10_android-usb-setup.md
├── prd_01_task_11_qr-fallback.md
├── prd_01_task_12_pinning-detection.md
├── prd_01_task_13_packaging.md
├── prd_01_task_14_docs-onboarding.md
└── prd_01_rep_01_mvp-network-debugger.md   # this file
```

## Deviations

Что отличается от исходного idea.md и почему:

1. **Tech stack из «гипотезы» сделан зафиксированным для MVP.** В `idea.md` §8 выбор `mitmproxy-rs` vs собственный engine был open вопросом. Зафиксировал embed `mitmproxy-rs` (Task 02) с trait-обёрткой `ProxyEngine` — даёт ~2 месяца экономии без потери возможности свапа.
2. **HTTP/3 явно исключён из MVP** (только детект ALPN). В idea.md упоминалось как риск, теперь — non-objective.
3. **WebSocket / SSE / gRPC inspector вынесены в beyond MVP.** В idea.md они в core feature set, но MVP-объём с честным таймлайном на 6-8 недель не вмещает inspector'ы — только маркер в списке.
4. **Tab "Wi-Fi proxy mode без USB"** в beyond MVP — но как fallback для iOS/Android wizard'ов реализован в Task 11 (QR-based). Primary остаётся USB.
5. **HAR export** — в MVP только single-capture export. Полный bulk-export в beyond MVP.
6. **Naming** — оставил рабочее `my-charles`. Финальный нейминг (idea §10) — отдельный задачник post-MVP.
7. **iOS proxy redirect — две стратегии (Plan A/Plan B)** документированы в Task 09. На iOS 17+ usbmuxd2 нестабилен, поэтому Plan B (port-forwarding + Developer Mode) — обязательная альтернатива. Это уточнение, не отклонение.
8. **Telemetry zero — explicit policy.** В idea.md этого не было; добавил в Task 14 как сильный USP в категории.
9. **Add device — checkbox согласия "I own / have authorization"** — в idea.md только в README; вынес в UI для anti-yolo (AC10).
10. **Git init не сделан** — проект не в git репо, branch/commit шаги skill'а пропустил. Рекомендую `git init` + первый коммит перед стартом Task 01.

## Critical path и оценка

Эпики E1 → E2 → E3/E4 (параллельно после foundation) → E5.

| Task | Эпик | Зависимости | Оценка (solo, weeks) |
|---|---|---|---|
| 01 bootstrap | E1 | — | 0.5 |
| 02 engine+CA | E2 | 01 | 1.5 |
| 03 storage | E1 | 01 | 1.0 |
| 04 IPC | E1 | 01 | 0.5 |
| 05 list+filter | E3 | 03, 04 | 1.0 |
| 06 detail panes | E3 | 05 | 1.0 |
| 07 replay | E3 | 02, 03, 04, 06 | 0.5 |
| 08 device core | E4 | 01, 04 | 0.5 |
| 09 iOS USB | E4 | 02, 08 | 1.5 ← high-risk |
| 10 Android USB | E4 | 02, 08 | 1.0 |
| 11 QR fallback | E4 | 09 | 0.5 |
| 12 pinning UX | E2 | 02, 03, 04, 05, 06 | 0.5 |
| 13 packaging | E5 | 01, 08 | 1.0 |
| 14 docs+onboard | E5 | все | 0.5 |
| **Σ** | | | **~11 weeks solo** |

Это выше чем idea.md прогнозировал (6-8 нед). Источник дельты: упаковка cross-platform, iOS-нюансы (Plan A/B), QR-fallback, документация. В пределах допуска для serious solo-MVP.

## Next step

**Старт с Task 01 (project bootstrap).** Подзадачи 1.1-1.6 — самодостаточны, не имеют зависимостей. Через 0.5 недели на руках — рабочий Tauri-скелет с зелёным CI на трёх OS, готовый принимать engine/storage код.

Параллельно — **Task 04 (IPC contracts)** можно начать сразу после 1.3 (UI скаффолд готов), потому что DTO-кодогенерация даёт жёсткий контракт ещё до имплементации команд. Это разморозит UI-работу (Task 05/06) пока пишется engine (Task 02).

Перед Task 01 рекомендую:
1. `git init` в `/Users/shaukat/Documents/Projects/my-charles/`, первый коммит с idea.md + PRD-пакетом.
2. Решение по naming (idea §10) можно отложить до выпуска v0.1.0-beta.
3. Прочитать `mitmproxy-rs` README + примеры — это снимет 30% риска по Task 02.
