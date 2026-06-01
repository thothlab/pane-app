---
title: Начало работы
description: Установить Pane, добавить устройство, увидеть первый перехваченный запрос.
---

Страница ведёт от «ещё не качал Pane» до «каждый запрос моего приложения
виден в списке captures».

## Установка

Скачай свежую сборку для своей OS со [страницы загрузки](https://pane.thothlab.tech/#download).

| Платформа | Файл |
| --- | --- |
| macOS Apple Silicon | `.dmg` |
| Linux x86_64 | `.AppImage` (портативный) или `.deb` / `.rpm` |
| Windows x86_64 | `.msi` (рекомендуется) или `.exe` NSIS-инсталлер |

### macOS — первый запуск

Closed-alpha-сборки пока не нотаризованы, так что первый запуск неуютный.
Однострочник, который качает, копирует в `/Applications` и снимает
quarantine-флаг:

```sh
curl -fsSL https://pane.thothlab.tech/install-macos.sh | bash
```

…или после перетаскивания из dmg:

```sh
xattr -dr com.apple.quarantine /Applications/Pane.app
```

### Linux

```sh
chmod +x Pane_*_amd64.AppImage && ./Pane_*_amd64.AppImage
```

### Windows

NSIS-инсталлер пока не подписан EV-сертификатом, SmartScreen покажет
предупреждение — **More info → Run anyway**.

## Первое устройство

1. Открой Pane. В sidebar — **Captures**, **Rules**, **Devices**,
   **Settings**.
2. Подключи телефон по USB. На Android включи **USB debugging** в
   Developer options. На iOS — доверь компьютеру при первом подключении.
3. **Devices → Add device**. Pane находит подключённые телефоны через
   `adb` / `libimobiledevice` и показывает список.
4. Выбери телефон → **Install CA + set proxy**. Pane:
   - сгенерирует цепочку leaf-сертификатов от локального CA,
   - запушит root CA в trust store устройства,
   - настроит Wi-Fi-прокси на локальный listener
     (`127.0.0.1:8888` по умолчанию).
5. Нажми **Start proxy** в sidebar.

Следующий запрос приложения уже окажется в списке captures. Клик по
строке открывает method / URL / status, заголовки, тело и timing.

## Чтение captures

Строка поиска поддерживает маленький filter DSL:

```text
host:api.example.com          # только запросы к этому хосту
status:5..                    # любой 5xx
!error:tls_handshake          # исключить pinning + handshake failures
status:200..299 host:*.dev    # ranges + globs
google                        # bareword: substring of host or path
```

Сохранить текущий фильтр — иконка ☆, он появится в sidebar.

Правая панель — **Overview / Request / Response / Timing / TLS**. Body
viewer определяет JSON / XML / text:

- **Tree** — разворачиваемые узлы, копирование по path или value.
- **Pretty** — отформатированный, подсвеченный текст.
- **Raw** — байты как пришли.

## Дальше

- [Подмена ответов](/docs/rules/) — Stub / Patch правила.
- [Релизный процесс](/docs/reference/releases/) — для мейнтейнеров.
