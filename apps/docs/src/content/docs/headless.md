---
title: Pane без окна
description: "`pane proxy run` — headless-инстанс для CI, скриптов и агентов, control.json и разбор конфликтов за директорию данных."
---

Pane — это один процесс, который владеет директорией данных и держит на ней
control-сокет. Всё остальное — [CLI](/docs/cli/), [MCP-сервер](/docs/agents/),
второй терминал — к нему прицепляется. От способа запуска зависит ровно одно:
будет окно или нет.

| Что нужно | Чем запускать | Окно |
| --- | --- | --- |
| Обычная работа | приложение Pane | да |
| CI, скрипты, агенты | `pane proxy run` | нет |
| Работа над кодом Pane | `make tauri-dev` | да |

## Запуск

```console
$ pane proxy run
pane: proxy listening on 127.0.0.1:8888
pane: control socket at /Users/you/Library/Application Support/tech.thothlab.pane/control.sock
pane: ready — Ctrl-C to stop
{"data_dir":"/Users/you/Library/Application Support/tech.thothlab.pane","event":"ready","kind":"headless","proxy":true}
```

Пояснения идут в stderr, а машиночитаемая строка — в stdout, поэтому
`pane proxy run | head -1` в скрипте не заглушается логами.

Процесс живёт в foreground до Ctrl-C. Флаги:

```sh
pane proxy run --port 9999                    # по умолчанию 127.0.0.1:8888
pane proxy run --host 0.0.0.0                 # слушать не только loopback
pane proxy run --no-proxy                     # поднять инстанс, но не стартовать прокси
pane proxy run --data-dir /tmp/scratch-pane   # отдельная база, отдельная блокировка
```

**Первая строка stdout — всегда `ready`.** В скрипте нужно блокироваться на ней,
а не опрашивать сокет в цикле и не спать фиксированные секунды.

Инстанс хостит тот же control-сокет, что и десктопное приложение, поэтому
`pane captures tail`, `pane devices add` и всё остальное в соседнем терминале
ведут себя ровно так же, как с открытым окном.

## Остановка

Ctrl-C и `kill` (SIGTERM) делают одно и то же: закрывают прокси и **снимают
настройки прокси с спаренных устройств**. Пропустить эту уборку — значит
оставить телефон смотрящим в мёртвый `127.0.0.1:8888`, то есть без интернета.

:::caution
`kill -9` этот путь пропускает. Если инстанс всё же был убит жёстко, верни
устройство в чувство, спарив его заново после следующего запуска, — или сними
настройки вручную через `adb shell settings delete global http_proxy`.
:::

`pane proxy stop` — другое: он останавливает **прокси внутри** работающего
инстанса, не выключая сам инстанс. Без запущенного инстанса — exit 3.

## Кто сейчас владеет директорией

Инстанс, поднявшись, пишет рядом с базой файл `control.json` (права 0600):

```console
$ cat ~/Library/Application\ Support/tech.thothlab.pane/control.json
{
  "protocol": 1,
  "pid": 14055,
  "app_version": "0.2.12",
  "kind": "headless",
  "endpoint": "/Users/you/Library/Application Support/tech.thothlab.pane/control.sock",
  "data_dir": "/Users/you/Library/Application Support/tech.thothlab.pane",
  "started_at": "2026-08-11 14:07:48.148151 +00:00:00"
}
```

- `kind` — `gui` (десктопное приложение) или `headless` (`pane proxy run`);
- `pid` — кого смотреть, если что-то залипло;
- `data_dir` — какой именно базой владеет этот процесс.

Быстрая проверка «а вообще кто-то запущен» — это `attached_to_instance` в
`pane doctor`.

## Конфликт за директорию

Директорией одновременно владеет ровно один процесс. Второй получает **exit 8**
и сообщение с именем директории — это работает защита, а не баг:

```console
$ pane proxy run
pane: another Pane instance already owns /Users/you/Library/Application Support/tech.thothlab.pane — use it instead of starting a second one
$ echo $?
8
```

Найти виновника:

```sh
pgrep -fl "Pane.app/Contents/MacOS/pane"   # GUI
pgrep -fl "pane proxy run"                 # headless
```

Дальше — либо закрыть его, либо увести новый инстанс в сторону:

```sh
pane proxy run --data-dir /tmp/scratch-pane
```

Обратная ситуация — «всё падает с exit 3, хотя Pane точно открыт» — обычно
значит, что приложение работает на **другой** директории данных: сравни
`data_dir` из `control.json` с тем, что показывает `pane doctor --data-dir …`.

## Черновая директория

`--data-dir` (или `PANE_DATA_DIR`) — штатный способ гонять эксперименты рядом с
рабочим Pane: своя база, своя блокировка, конфликта нет.

```sh
export PANE_DATA_DIR=/tmp/scratch-pane
pane proxy run --port 9999 &
# … прогон …
pane captures count --filter 'status:500..599'
```

Правила в такой директории пустые — набор моков туда можно завезти файлом:
`pane rules import fixtures/all-rules.json` (см. [CLI](/docs/cli/#правила)).

## Шаг в CI

```yaml
- name: Поднять Pane и прогнать сценарий
  run: |
    set -euo pipefail
    export PANE_FORMAT=json
    export PANE_DATA_DIR="$RUNNER_TEMP/pane"

    pane proxy run --port 8888 &
    PANE_PID=$!
    trap 'kill -TERM $PANE_PID' EXIT          # SIGTERM, не -9: нужна уборка

    until pane doctor | jq -e '.attached_to_instance' >/dev/null; do sleep 0.2; done

    pane rules import fixtures/all-rules.json
    pane collections only orders-error
    pane captures clear --yes

    ./gradlew connectedAndroidTest

    test "$(pane captures count --filter 'state:stubbed rule:orders-500')" -ge 1
    test "$(pane captures count --filter 'host:api.example.com state:completed')" -eq 0
```

CLI под Windows не собирается (см. [оговорку в CLI](/docs/cli/#установка)), так
что headless-шаг живёт на macOS- и Linux-раннерах.

## Что дальше

- [CLI `pane`](/docs/cli/) — все команды, фильтры и exit-коды.
- [Агенты и MCP](/docs/agents/) — те же операции для LLM-агента, плюс паттерн
  «докажи, что ответ пришёл из мока».
