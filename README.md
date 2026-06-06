**Русский** · [English](README.en.md)

# Pane

Современный HTTPS-отладчик сетевых запросов, заточенный под одну вещь: настройка мобильного устройства за 30 секунд вместо 15 минут. Подключи iPhone или Android по USB, нажми **Add**, и начни смотреть трафик — без танцев в Settings, без ручной возни с trust store, без редактирования Wi-Fi-прокси.

> **Статус:** v0.1.0 — первый публичный релиз. Кроссплатформенный shell, proxy engine (HTTP/1.1 с TLS MITM), capture/replay storage, подмена и патчинг ответов, USB-настройка устройств, CI/release-пайплайн — всё на месте. Пользовательская документация и инструкции по настройке — на [pane.thothlab.tech/docs](https://pane.thothlab.tech/docs/).

## Что внутри

- **Tauri 2** desktop shell (Windows / macOS / Linux).
- **SolidJS + Tailwind** UI: виртуализированный список captures, filter DSL, detail panes, replay composer.
- **Rust workspace** из сфокусированных крейтов: engine trait, нативный MITM-прокси, управление root-CA (rcgen + системный keychain), SQLite storage с content-addressed body blobs, пайплайны для iOS / Android (libimobiledevice + adb sidecars), сборщик Apple `mobileconfig`, QR-fallback setup server, эвристика детекции cert pinning.
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

Нажми **Start proxy** в нижнем левом углу. Дальше **Devices → Add device** — Pane через USB поставит root CA (полный авто на iOS и рутованном Android; на non-root Android — пушит файл и показывает inline-инструкцию для one-time manual install), пробросит порт через `adb reverse` и выставит PAC-прокси (которое корректно fallback'ает на DIRECT при unplug, не оставляя устройство без интернета). После этого трафик начнёт попадать в **Captures**.

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
src-tauri/              Tauri main crate + IPC command modules
crates/
  pane-ipc/        Shared DTOs между Rust и TS
  pane-engine/     ProxyEngine trait + EngineEvent
  pane-engine-mitm/  Нативный HTTP/1.1 MITM движок
  pane-ca/         Root CA generation, rotation, keychain storage
  pane-storage/    SQLite + body blobs + filter DSL + replay
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
