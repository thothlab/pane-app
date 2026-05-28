# Task 14 — Documentation, in-app onboarding, error reporting

## Goal
Свежий пользователь должен пройти от download до первого capture без чтения внешних доков. README объясняет проект за 60 секунд. В приложении — welcome wizard + контекстные хелпы. Errors в логе всегда дают понятное направление, не stack trace.

## Scope
**In:**
- README.md + LICENSE + DISCLAIMER.
- Полная документация — отдельный mdBook или Astro site (`docs/` + `mdbook.toml`).
- Welcome wizard в приложении (первый запуск).
- Contextual tooltips и help icons.
- Crash report → локальный лог-файл (без auto-send, opt-in копировать в clipboard).
- License + Acknowledgements view.

**Out:**
- Online help bot — beyond MVP.
- Self-serve issue tracker — используем GitHub Issues.

## Subtasks

### 14.1 README
- [ ] One-page intro: что это, screenshot, "30-second setup" GIF (демо на iOS/Android).
- [ ] Sections: Install, Quick start, Compared to (Charles/Proxyman/Reqable/mitmproxy), Why we don't bypass pinning, License, Disclaimer.
- [ ] Бейджи: CI status, release version, license.

### 14.2 Полная документация (mdBook)
- [ ] Structure:
  - **Getting Started:** Install / First setup iOS / First setup Android / Wi-Fi fallback.
  - **Features:** Capture list / Filters / Detail panes / Replay.
  - **Devices:** Per-OEM gotchas / iOS DevMode / Android no-root flow.
  - **Cert pinning:** What is / Why not bypass / Workarounds.
  - **Troubleshooting:** Common errors with steps.
  - **Architecture:** Engine / Storage / Sidecars — для контрибьюторов.
- [ ] Деплой на GitHub Pages при tag.

### 14.3 Welcome wizard (first-run)
- [ ] 4 экрана:
  1. "Welcome" + краткая ценность.
  2. "How it works" — диаграмма device → proxy → desktop.
  3. "Choose your first device": iOS USB / Android USB / Skip.
  4. "Ethical use" — copy про legitimate use cases + chekbox согласия.
- [ ] Persist completion в settings.

### 14.4 Contextual help
- [ ] Help icon (`?`) рядом с каждой нетривиальной кнопкой ("Rotate CA", "Map proxy", "Pause capture").
- [ ] Hover → tooltip; click → side-panel с короткой статьёй (~150 слов).
- [ ] Articles живут в `docs/` (markdown), импортируются как assets.

### 14.5 Error reporter
- [ ] При panic / unexpected error — write to `~/.local/share/my-charles/logs/error-<ts>.log`.
- [ ] Toast: "Something went wrong. Show details / Copy log / Open file location".
- [ ] No auto-upload (privacy).
- [ ] Tracing setup: rotation, level config через env (`MY_CHARLES_LOG=debug`).

### 14.6 License + acknowledgements
- [ ] `/about/license` — full Apache-2.0 + third-party licenses (auto-generated via `cargo about` + `license-checker` npm).
- [ ] Acknowledgements: explicit thanks для `mitmproxy-rs`, `libimobiledevice`, `rcgen`, etc.

### 14.7 Telemetry policy (explicit none)
- [ ] About → Privacy: "We collect zero telemetry. No phone-home. Error logs stay local unless you copy them."
- [ ] В коде — gate против любого auto-network outside user-initiated actions (lint: forbid `reqwest::get` outside `replay/` and `setup-server/`).

## Deliverables
- README.md.
- `docs/` mdBook content.
- Welcome wizard в UI.
- Error reporter с локальным лог-файлом.
- License view.
- Auto-deploy docs на GitHub Pages.

## Definition of Done
- [ ] AC10 из PRD: README + About + first-run wizard содержат disclaimer + Apache-2.0.
- [ ] Свежий пользователь на чистой машине без референса к идее-документу проходит от install до first capture (юзабилити-тест на 3 разработчиках).
- [ ] Docs site доступен на `https://thothlab.github.io/my-charles/` (или своём домене).
- [ ] Error reporter генерит читаемый лог при искусственно вызванном panic.
- [ ] `cargo about` отчёт без unknown-licenses.

## Tests
- **Usability:** 3 внешних мобильных-разработчика проходят onboarding без подсказок, time-to-first-capture ≤ 2 мин.
- **Docs build:** mdBook builds clean на CI, broken-link check passes.
- **Error reporter:** искусственный panic → log файл существует + содержит читаемый stack.

## Dependencies
- Все предыдущие tasks (документация описывает их).
- Task 13 — без packaging нечего ставить юзеру.
