---
title: CLI pane
description: Управление Pane из терминала — captures, правила, коллекции, устройства и logcat без единого клика по GUI.
---

`pane` — отдельный бинарник, который делает всё то же, что и окно приложения:
читает captures, создаёт и переключает правила, добавляет устройства, тянет
logcat. Он нужен там, где GUI не помогает: в скриптах, в CI, в автотестах и
когда за клавиатурой сидит агент (см. [Агенты и MCP](/docs/agents/)).

Если приложение запущено — команды уходят в него по локальному control-сокету, и
результат виден в открытом окне сразу. Если не запущено — CLI открывает ту же
директорию с данными напрямую.

## Установка

CLI **не входит** в бандл приложения — это отдельный файл. Забрать его можно из
релиза:

| Платформа | Ассет релиза |
| --- | --- |
| macOS Apple Silicon | `pane-cli-darwin-aarch64` |
| Linux x86_64 | `pane-cli-linux-x86_64` |

```sh
curl -fsSL -o pane https://github.com/thothlab/pane-app/releases/latest/download/pane-cli-darwin-aarch64
chmod +x pane && mv pane /usr/local/bin/
pane --version
```

Или собрать из исходников — тогда версия гарантированно совпадёт с приложением:

```sh
make cli-install          # = cargo build --release -p pane-cli + install
# или вручную:
cargo build --release -p pane-cli
./target/release/pane-cli install       # симлинк в PATH под именем `pane`
```

`install` кладёт симлинк в `/usr/local/bin`, а если туда нельзя писать — в
`~/.local/bin`. Другой каталог — `install --dir <DIR>`.

:::caution[Windows]
Сборки CLI под Windows нет. CLI общается с приложением через Unix-сокет, у
которого нет windows-реализации, так что там каждая команда сообщала бы «нет
запущенного инстанса». Windows-сборка приложения при этом полноценная.
:::

:::note[Держи версии в паре]
CLI отказывается открывать директорию с данными, схема которой новее или старее
ожидаемой, — вместо того чтобы мигрировать её:

```text
pane: opening the Pane data directory
  → this database is at schema v11 and this build expects v12. Migrating it
    would stop the installed Pane app from launching, so it is left alone.
```

Миграция односторонняя: приложение, встретив незнакомую версию, не запустится
вовсе. Обнови отставшую половину — или уведи CLI на черновую директорию через
`--data-dir`. **Удалять базу, чтобы «проскочить», нельзя** — это вся история
captures и вся библиотека правил.
:::

## Две команды, с которых начинается всё

```console
$ pane doctor
instance   attached to a running Pane
proxy      running   127.0.0.1:8888      38 captures
adb        /Users/you/Library/Android/sdk/platform-tools/adb
ca         7c4db809d727457e…
devices    2 paired · 1 attached
```

`doctor` за один вызов отвечает: инстанс есть? прокси поднят? adb найден? CA
живой? устройства спарены? С него начинается любой разбор «почему ничего не
работает».

```sh
pane schema      # всё дерево команд, обе грамматики фильтров и exit-коды — как JSON
```

`schema` — машиночитаемая версия этой страницы, и она авторитетнее: она
генерируется из самого CLI, поэтому при расхождении с документацией права она.

## Формат вывода

По умолчанию — человекочитаемые колонки. Для скриптов:

```sh
export PANE_FORMAT=json     # один раз на сессию
pane --json doctor          # или флагом на конкретный вызов
```

Правило простое: **stdout — чистый канал для `| jq`**, всё остальное
(предупреждения, пояснения, ошибки) уходит в stderr. `captures body` пишет в
stdout сырые байты тела, а сообщение об обрезке — в stderr, поэтому
перенаправление файла не портится.

## Captures: лестница детализации

Каждая следующая ступень стоит примерно в десять раз дороже предыдущей —
начинай сверху, особенно если контекст ограничен.

```console
$ pane captures count --filter 'host:api.example.com status:500..599'
2
```
```console
$ pane captures list --filter 'status:500..599' --limit 5
SID       METHOD  STATUS HOST              PATH              MS  BYTES  RULE
c97cc13d  POST    500    api.example.com   /v2/orders       412    218
4cb21e58  GET     503    api.example.com   /v2/orders/8821 30011    244
```
```console
$ pane captures get c97cc13d              # строка целиком + заголовки запроса и ответа
$ pane captures body c97cc13d --res --max-bytes 4096 | jq .error
```

- `--filter` принимает **ту же строку**, что и поисковая строка в GUI, —
  см. [Фильтрация captures](/docs/filtering/).
- `--fields id,status,url_path` оставляет только нужные поля, `--full` отдаёт
  весь DTO вместо краткой проекции.
- Тело обрезается до 8 KiB. Для больших payload'ов — `--out FILE` (пишет
  целиком и ничего не печатает), а не `--max-bytes 0`.
- `captures body` по умолчанию отдаёт ответ; тело запроса — `--req`. Бинарное
  тело — `--base64`.

Отдать запрос коллеге или воспроизвести его вне Pane:

```sh
pane captures export c97cc13d --format curl          # однострочник
pane captures export c97cc13d --format har --out req.har
```

`captures clear --yes` стирает всё — полезно между сценариями, чтобы проверка
не сматчила прошлый прогон.

### Живой поток

```console
$ pane captures tail --filter 'host:api.example.com' --count 1 --timeout 30
{"event":"ready","filter":"host:api.example.com","count":1,"timeout":30}
{"event":"capture","id":"c97cc13d-…","method":"POST","url_path":"/v2/orders",
 "status":500,"state":"completed","duration_ms":412,"matched_rule_name":null}
{"event":"end","reason":"count","captures":1,"elapsed_ms":1014}
```

NDJSON, **первая строка всегда `ready`**, последняя — всегда `end`. Дождись
`ready` и только потом дёргай приложение: иначе гонка на первом же запросе,
которую обычно «лечат» вставкой `sleep`, — оттуда и берутся флакающие скрипты.
`--count` — это утверждение, `--timeout` (в секундах, без суффиксов) — дедлайн;
недобрал — [exit 7](#exit-коды).

`tail` — одна из немногих команд, которым нужен **запущенный инстанс**: без него
она падает с exit 3. Тем же свойством обладают `proxy stop`, `devices add`,
`devices rm`, `logcat attach` и `logcat detach`; всё остальное — чтение captures,
правки правил, коллекции — работает и с закрытым приложением.

## Правила

```sh
pane rules ls
pane rules mock --host api.example.com --path '/v2/orders*' --method POST \
  --status 500 --body '{"error":"internal"}' --name orders-500
pane rules from-capture c97cc13d --status 500 --name orders-500
```

`mock` создаёт stub-правило одной строкой (по умолчанию `--status 200`,
`--mime application/json`); `from-capture` берёт host, path и method из
настоящего захваченного запроса. Большое тело — `--body-file fixtures/x.json`.
Флаг `--disabled` создаёт правило выключенным.

**Всегда указывай `--name`.** Это и селектор для `enable | disable | rm`, и
значение для `rule:` в фильтрах. Без него имя соберётся из host+path, и
проверять его неудобно.

Включение и выключение — по одному или пачкой:

```sh
pane rules enable orders-500          # по подстроке имени или id
pane rules disable --all              # сброс в известное состояние
pane rules enable --collection base   # все правила одной коллекции
pane rules enable --ungrouped         # все, что вне коллекций
```

Пачечные флаги существуют не для красоты: перебор большой библиотеки по одному
правилу — это по запуску процесса на правило, и каждый заново вычитывает весь
список, чтобы разрешить селектор.

Обмен наборами моков — тот же формат `pane-rules`, что читает и пишет GUI, тела
внутри файла:

```sh
pane rules export --out fixtures/all-rules.json
pane rules import fixtures/all-rules.json --dry-run    # сначала посмотреть
pane rules import fixtures/all-rules.json
```

Импорт всегда создаёт новые сущности и никогда не перезаписывает по имени.

## Коллекции

Коллекция — это один сценарий. Переключение сценария целиком:

```console
$ pane collections ls
ID        STATE    RULES  NAME
b2d5f2a0  off          8  base — база 500 ₽
a647e8c1  off          8  noamount — QR без суммы

$ pane collections only noamount
pane: only `noamount — QR без суммы` is live — 8 rule(s) enabled, everything else off
```

`only` = «выключить всё, затем включить эту коллекцию». Именно так и нужно
переходить от сценария к сценарию: если оставить предыдущую коллекцию включённой,
два правила на один host и path дадут ответ от того, у которого приоритет выше, —
а проверка на `state:stubbed` всё равно позеленеет и ничего не докажет.

`collections rm <sel> --yes` удаляет коллекцию, её правила переезжают в
Ungrouped; `--with-rules` удаляет и их тоже.

:::caution[Колонка STATE — историческая]
`enable | disable | only` тикают **сами правила** — потому что правило
срабатывает по собственному флагу `enabled` (и по области действия этого флага,
см. ниже), а второго выключателя на уровне коллекции у движка нет. Колонка
`STATE` в `collections ls` показывает старое поле в базе, которое ни на что не
влияет; смотреть надо на `pane rules ls`.
:::

## Разные сценарии на разных устройствах

По умолчанию правило действует на все устройства, и `only` переключает сценарий
сразу на всех — на одном телефоне это то, что нужно, на четырёх уже нет. Флаг
`--device <sel>` ограничивает изменение одним устройством, остальные продолжают
работать как работали.

```console
$ pane devices ls
ID        STATE  PLATFORM  NAME
a91f3c02  ready  android   Google Pixel 7 · Android 14 · r-0XXH
77b1e4de  ready  android   Google sdk_gphone64 · Android 14 · r-5554

$ pane rules disable --all --device pixel
pane: disabled 14 of 21 rules for a91f3c02
pane: 14 rule(s) are now pinned to named devices — a device paired from here on
      will not get them (undo with `pane rules enable --all`)
$ pane collections only checkout --device pixel
$ pane collections only errors --device 77b1e4de
```

Библиотека правил при этом остаётся общей: одно и то же правило просто включено
на одном устройстве и выключено на другом — копировать его под каждый девайс не
нужно. Флаг понимают `rules enable | disable | mock` и
`collections enable | disable | only`.

`pane rules ls --device <sel>` показывает список глазами этого устройства:
колонка `LIVE` вместо `STATE`, плюс колонка `SCOPE` — `all` у общих правил и
`N dev` у закреплённых. Строки не прячутся: вопрос «почему мой мок не сработал»
должен решаться одной командой, а не догадками о том, что отфильтровалось.

Селектор устройства — подстрока имени, серийник или id, то есть та же строка,
которую принимает `--filter 'device:…'`. `__host__` — трафик самого Mac.

:::caution[Закрепление одностороннее]
Выключение правила для одного устройства перечисляет устройства, на которых оно
остаётся включённым. Значит девайс, подключённый **после** этого, правило не
получит. Команда об этом сообщает; `pane rules enable --all` без `--device`
возвращает всю библиотеку в глобальное состояние.

iOS в это не входит: его трафик идёт через общий порт прокси и не
атрибутируется на устройство, поэтому `--device <ios>` отвечает ошибкой вместо
того, чтобы молча ничего не сделать. Такие устройства видят только правила,
включённые для всех.
:::

Проверять результат нужно тоже с `device:` на обеих сторонах — иначе трафик
соседнего телефона ответит за этот:

```console
$ pane captures count --filter 'device:pixel state:stubbed rule:checkout-ok'
1
$ pane captures count --filter 'device:pixel rule:orders-500'
0
```

## Устройства, logcat, CA, прокси

```sh
pane devices attached                  # что воткнуто прямо сейчас (exit 4 — ничего)
pane devices add emulator-5554         # спарить; нужен запущенный прокси
pane devices rm <sel> --yes

pane logcat attach --serial emulator-5554
pane logcat query --serial emulator-5554 --filter 'app:dev.shop.app level:E' --limit 20
pane logcat pids --serial emulator-5554
pane logcat clear --serial emulator-5554 --yes

pane ca show                           # отпечаток и срок действия
pane ca export --format pem --out ca.pem   # ещё: der, qr, mobileconfig

pane proxy status
pane proxy start --port 8888
pane proxy stop                        # снимает и настройки прокси с устройств
```

Грамматика фильтра logcat — `tag: msg: level: pid: app:`, плюс `~regex`; голое
слово ищется по тегу или сообщению, `level` принимает `V D I W E F S` и
диапазоны вида `W..F`. `app:` CLI сам разворачивает в PID'ы. Подробнее —
[Окно Logcat](/docs/logcat/).

## Селекторы

Везде, где команда ждёт `<id>` или `<sel>`, подойдёт полный id, **уникальный
префикс id** или **подстрока имени** правила / коллекции / устройства.
Неоднозначность — это ошибка со списком кандидатов: CLI никогда не выбирает за
тебя.

Ничего не спрашивает интерактивно. Разрушающим командам нужен `--yes`.

## Exit-коды

| Код | Значение |
| --- | --- |
| 0 | ок |
| 1 | ошибка |
| 2 | ошибка использования (разбор аргументов) |
| 3 | нет запущенного инстанса / прокси остановлен |
| 4 | нет устройства или не найден adb |
| 5 | не найдено |
| 6 | битый фильтр |
| 7 | таймаут или `--count` не набран — **проваленная проверка** |
| 8 | конфликт (порт занят, предусловие не выполнено) |

Код 7 стоит отдельно: это не сбой инструмента, а вердикт. На нём удобно строить
`set -e`-скрипты.

## Директория с данными

```sh
pane doctor --data-dir /tmp/scratch-pane
export PANE_DATA_DIR=/tmp/scratch-pane
```

По умолчанию берётся платформенная: `~/Library/Application Support/tech.thothlab.pane`
на macOS, `~/.local/share/pane` на Linux. Отдельная директория — это отдельная
база, отдельная блокировка и отдельный инстанс: так и надо гонять эксперименты,
не трогая реальную историю captures.

## Что дальше

- [Pane без окна](/docs/headless/) — `pane proxy run` для CI и скриптов.
- [Агенты и MCP](/docs/agents/) — те же операции как инструменты для LLM-агента.
- [Фильтрация captures](/docs/filtering/) — грамматика `--filter`.
