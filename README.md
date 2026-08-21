**Русский** · [English](README.en.md)

# Pane

Современный HTTPS-отладчик сетевых запросов, заточенный под одну вещь: настройка мобильного устройства за 30 секунд вместо 15 минут. Подключи iPhone или Android по USB, нажми **Add**, и начни смотреть трафик — без танцев в Settings, без ручной возни с trust store, без редактирования Wi-Fi-прокси.

> **Статус:** активная разработка, регулярные релизы с авто-обновлением через GitHub Releases. На месте кроссплатформенный shell, proxy engine (HTTP/1.1 с TLS MITM), capture/replay storage, подмена и патчинг ответов, USB-настройка устройств, JSON-подсветка в редакторе правил, загрузка JSON-тела запроса/ответа из файла одним кликом, sticky-шапка body-панели, чекбокс-тогглеры коллекций, отдельное окно Logcat на устройство (фильтр-DSL + поиск, байт-безопасный поток, устойчивый к не-UTF-8 выводу устройства). CI/release-пайплайн собирает signed-бандлы для macOS / Linux / Windows на каждый tag. Пользовательская документация — на [pane.thothlab.tech/docs](https://pane.thothlab.tech/docs/).

## Что внутри

- **Tauri 2** desktop shell (Windows / macOS / Linux).
- **SolidJS + Tailwind** UI: виртуализированный список captures, filter DSL, detail panes, replay composer. Полная двуязычная локализация EN / RU через `@solid-primitives/i18n`, переключение реактивно из Settings.
- **Rust workspace** из сфокусированных крейтов: engine trait, нативный MITM-прокси, управление root-CA (rcgen + системный keychain), SQLite storage с content-addressed body blobs, пайплайны для iOS / Android (libimobiledevice + adb sidecars), сборщик Apple `mobileconfig`, QR-fallback setup server, эвристика детекции cert pinning.
- **CLI `pane` + MCP-сервер**: те же операции без GUI — из терминала, из CI и из LLM-агента. Приложение держит на директории данных control-сокет, к которому цепляются CLI и агент; `pane proxy run` поднимает такой же инстанс без окна.
- **CI** matrix на Windows, macOS, Linux — fmt + clippy + tests + Tauri debug build.

## Быстрый старт

```bash
# 1. Toolchain
rustup default stable
brew install pnpm   # или: corepack enable

# 2. Установить зависимости
pnpm install

# 3. (Один раз) положить sidecar-бинарники
./scripts/fetch-sidecars.sh    # выведет инструкции

# 4. Запустить
pnpm tauri:dev
```

Нажми **Start proxy** в нижнем левом углу. Дальше **Devices → Add device** — Pane через USB поставит root CA (полный авто на iOS и рутованном Android; на non-root Android — пушит файл и показывает inline-инструкцию для one-time manual install в сворачиваемом блоке), пробросит порты через `adb reverse` и выставит на Android **оба** `http_proxy` (для OkHttp/нативных стеков) и `http_proxy_pac` (для Chromium). На Android также автоматически ставится companion APK (~4 MB) — heartbeat-watchdog, который автоматически снимает прокси с устройства при выдёргивании USB, чтобы интернет не пропадал. После этого трафик начнёт попадать в **Captures**.

## Из терминала и из агента

GUI — не единственный способ управлять Pane. Бинарник `pane` делает то же самое
из скрипта, из CI и из LLM-агента: если приложение открыто, команды уходят в
него по control-сокету и результат сразу виден в окне; если закрыто — CLI
работает с той же директорией данных напрямую.

```bash
make cli-install                       # собрать и положить `pane` в PATH
export PANE_FORMAT=json                # один раз на сессию

pane doctor                            # прокси? устройства? adb? CA?
pane captures list --filter 'host:api.example.com status:500..599' --limit 20
pane captures body <id> --res --out /tmp/resp.json
pane rules mock --host api.example.com --status 500 --body '{"e":1}' --name orders-500
pane collections only orders-error     # переключить сценарий целиком
pane captures count --filter 'state:stubbed rule:orders-500'   # проверка: мок ответил

pane proxy run                         # инстанс без окна — для CI
pane mcp                               # MCP-сервер по stdio: Pane как инструменты агента
pane schema                            # всё дерево команд и грамматики фильтров, как JSON
```

Подробно: [CLI](https://pane.thothlab.tech/docs/cli/) ·
[Pane без окна](https://pane.thothlab.tech/docs/headless/) ·
[Агенты и MCP](https://pane.thothlab.tech/docs/agents/). Готовые скиллы для
агента лежат в `.claude/skills/`, шпаргалка на одну страницу — в `AGENTS.md`.

## Чем отличается от других

|                          | Charles | Proxyman | Reqable | mitmproxy | **Pane**          |
| ------------------------ | ------- | -------- | ------- | --------- | ----------------------- |
| Цена                     | $50     | $69/год  | freemium | free      | **free / Apache-2.0**   |
| Современный UI           | ✗       | ✓        | ✓       | partial   | ✓                       |
| Настройка устройства одной командой | ✗ | ✗ | ✗     | partial   | **★ главный фокус**     |
| UX cert pinning          | silent  | silent   | partial | manual    | **детект + объяснение** |
| Git-friendly конфиг      | ✗       | ✗        | ✗       | ✗         | планируется (post-MVP)  |

## Границы

Pane сделан для отладки **своих** приложений и для легитимной авторизованной security-работы. Он **не** обходит certificate pinning — когда приложение пинит, ты увидишь понятное объяснение и указатели на нужные (внешние) инструменты, а не тихий фейл.

Pane **не** монитор продакшен-трафика, **не** packet-level capture tool, и **не** harness для нагрузочного тестирования.

## Структура репо

```
src/                    SolidJS frontend (Tauri webview)
src/i18n/               EN + RU translation dictionaries + reactive translator
src-tauri/              Tauri main crate + IPC command modules
crates/
  pane-ipc/        Shared DTOs между Rust и TS
  pane-engine/     ProxyEngine trait + EngineEvent
  pane-engine-mitm/  Нативный HTTP/1.1 MITM движок
  pane-ca/         Root CA generation, rotation, keychain storage
  pane-storage/    SQLite + body blobs + filter DSL + replay
  pane-core/       Операции без GUI — общая база для приложения, CLI и headless
  pane-control/    Control-сокет + control.json: как к инстансу цепляются клиенты
  pane-cli/        Бинарник `pane` + MCP-сервер (`pane mcp`)
  pane-serve/      UI по локальному HTTP: /rpc, авторизация, встроенный SPA
  pane-devices/    Кроссплатформенный device manager + state machine
  pane-ios/        libimobiledevice wrapper
  pane-android/    adb wrapper, CA install paths, PAC server wiring
  pane-mobileconfig/  Сборщик Apple .mobileconfig
  pane-setup-server/  LAN HTTP server для QR-fallback pairing
  pane-pinning/    Эвристика pinning + hint kinds
apps/
  web/                  pane-web сервис (landing + docs + release endpoints)
  docs/                 Astro Starlight documentation site
.github/workflows/      CI + release
scripts/                fetch-sidecars, dev launcher
```

## Лицензия

[Apache-2.0](LICENSE). Third-party компоненты, используемые в runtime, сохраняют свои лицензии.
