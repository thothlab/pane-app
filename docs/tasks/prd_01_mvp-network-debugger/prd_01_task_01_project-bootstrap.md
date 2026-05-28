# Task 01 — Project bootstrap (Tauri 2 + SolidJS + Tailwind)

## Goal
Поднять пустой, но рабочий каркас приложения с правильной структурой репо, dev-loop'ом (`tauri dev`), CI на трёх платформах и базовым layout shell'ом. К концу task'а — можно открыть окно с пустым sidebar/main split, hot-reload работает, lint/format/test проходят на всех трёх OS.

## Scope
**In:** repo structure, Tauri скаффолд, UI скаффолд, базовая навигация, CI matrix, lint/format, базовый dev-doc `README.md` для контрибьютора.
**Out:** любая proxy/capture логика, sidecars, persistence — это задачи 02-14.

## Subtasks

### 1.1 Repo skeleton
- [ ] Корень: `/Cargo.toml` (workspace), `/package.json`, `/.gitignore`, `/.editorconfig`, `/rust-toolchain.toml` (stable + components: clippy, rustfmt).
- [ ] `src-tauri/` — Rust side (Tauri main + plugins).
- [ ] `src/` — SolidJS front-end.
- [ ] `crates/` — workspace для собственных Rust крейтов (engine, storage, devices) — пустые crate-skeletons.
- [ ] `tests/e2e/` — Playwright-based UI smoke (запуск позже).
- [ ] `scripts/` — bash/ps1 helpers (`bootstrap.sh`, `dev.sh`).
- [ ] `docs/tasks/prd_01_*` (этот PRD уже создан).

### 1.2 Tauri init
- [ ] Tauri 2 latest stable. Identifier `tech.thothlab.mycharles` (заменить при ребрендинге).
- [ ] Allow-list минимальный: window controls, fs limited (только app data), shell scope пустой.
- [ ] Tray icon stub (без меню) — нужен на macOS чтобы окно не убивалось при close.
- [ ] Конфиг `tauri.conf.json`: title `my-charles`, размеры по умолчанию 1280×800, min 960×600.

### 1.3 UI скаффолд (SolidJS)
- [ ] `pnpm` как пакет-менеджер. `vite` + `vite-plugin-solid`.
- [ ] Routing: `solid-router` — 4 routes: `/` (captures), `/devices`, `/settings`, `/about`.
- [ ] Layout shell: левый sidebar (nav + device list placeholder) + main split. Цвета и шрифты из Tailwind theme (тёмная + светлая).
- [ ] Tailwind 4 + `tailwindcss-animate` + базовый design tokens файл (`src/styles/tokens.css`).
- [ ] Иконки — `lucide-solid`.

### 1.4 Dev tooling
- [ ] `eslint` + `prettier` + `oxlint` (быстрая проверка в pre-commit).
- [ ] `cargo fmt`, `cargo clippy --all-targets -D warnings`.
- [ ] Pre-commit hook (`lefthook` или `husky` + `lint-staged`) — gate на форматирование.
- [ ] VS Code workspace: рекомендованные расширения (rust-analyzer, Tauri, Solid, Tailwind).

### 1.5 CI matrix (GitHub Actions)
- [ ] Job `lint`: rustfmt + clippy + eslint.
- [ ] Job `test`: `cargo test --all` + `pnpm test` (Vitest).
- [ ] Job `build`: `tauri build` на `ubuntu-latest`, `macos-14`, `windows-2022`.
- [ ] Кеш `cargo` (`Swatinem/rust-cache`) и pnpm store.
- [ ] Required для merge в `main`.

### 1.6 README + CONTRIBUTING (минимум)
- [ ] `README.md`: одна страница — что это, как запустить dev, как собрать.
- [ ] `CONTRIBUTING.md`: ветки `codex/`, commit-стиль (Conventional Commits), как локально пройти CI.
- [ ] `LICENSE` — Apache-2.0.

## Deliverables
- Зелёный CI на трёх OS.
- `pnpm tauri dev` поднимает окно с навигацией.
- Все subtasks отмечены done в чек-листе.
- Стартовый коммит на ветке `codex/prd-01-task-01-bootstrap` (или main, если репо ещё пустое).

## Definition of Done
- [ ] `cargo clippy --all-targets -D warnings` passes на CI matrix.
- [ ] `cargo fmt --check` passes.
- [ ] `pnpm lint && pnpm typecheck` passes.
- [ ] `tauri build` производит installable артефакт на каждой из трёх OS (даже если он пустой).
- [ ] README отвечает на: «Что это», «Как запустить dev», «Как собрать», «Лицензия».
- [ ] Workspace открывается в VS Code без warning'ов на missing extensions.

## Tests
- Smoke: на CI после `tauri build` — `file <artifact>` подтверждает корректный тип (`.dmg`, `.msi`/`.exe`, `.deb`/`.AppImage`).
- UI smoke (Vitest): рендер layout, переход по 4 routes без ошибок в консоли.
- Rust smoke: `cargo test -p mycharles-core` (пустой test чтобы убедиться что workspace компилируется).

## Dependencies
Никаких. Это первый task.
