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

1. Открой Pane. В sidebar — **Captures**, **Rules**, **Devices**;
   ниже отдельной группой **Settings**, **Docs**, **About**; в самом
   низу — кнопка **Start proxy**.
2. Нажми **Start proxy** (иначе **Add device** откажет — иначе
   устройство будет ходить на мёртвый порт и потеряет интернет).
3. Подключи телефон по USB:
   - **Android** — включи **USB debugging** в Developer options. На
     устройстве должен быть выставлен **PIN / паттерн / пароль**
     блокировки экрана (без него Android не даёт ставить user CA).
   - **iOS** — доверь компьютеру при первом подключении.
4. **Devices → Add device**. Pane находит подключённые телефоны через
   `adb` / `libimobiledevice`. Выбери телефон → **+ Add**. Дальше Pane
   автоматически:
   - сгенерирует root CA (ECDSA P-256) и сохранит локально;
   - **iOS** — пушит mobileconfig профиль через USB; пользователь
     подтверждает установку в Settings;
   - **Android (rooted)** — кладёт CA в системный trust store
     `/system/etc/security/cacerts/`, **полный авто**;
   - **Android (без root)** — пушит файл `/sdcard/Download/pane-ca.pem`
     на устройство, в строке Devices показывает пошаговую инструкцию
     «Settings → Install certificate → выбрать pane-ca.pem» (см. ниже);
   - проложит `adb reverse tcp:8888 tcp:8888` + `tcp:8889 tcp:8889`
     (PAC) + `tcp:8890 tcp:8890` (helper heartbeat) — трафик с
     устройства идёт через USB, **Wi-Fi не нужен и не настраивается**;
   - выставит на устройстве `http_proxy=127.0.0.1:8888` (основное,
     что читают OkHttp/Retrofit/нативные стеки) и `http_proxy_pac` на
     локальный PAC-сервер (дополнительно для Chromium);
   - установит companion APK `tech.thothlab.pane.helper` и грантнет
     ему через adb `WRITE_SECURE_SETTINGS` + `POST_NOTIFICATIONS` —
     это watchdog, который снимет прокси с устройства когда выдернешь
     шнур (см. раздел «Helper APK» ниже).
5. На Android без root — выполни manual install по инструкции в Devices
   (раскрывающийся блок «How to install the CA certificate»).
6. На iOS — подтверди установку профиля в Settings → Profile Downloaded.

Следующий запрос приложения окажется в списке captures. Клик по
строке открывает method / URL / status, заголовки, тело и timing.

### Почему install CA на Android требует ручного шага

На Android 11+ CertInstaller всегда ведёт через SAF file-picker
(scoped-storage). На Samsung One UI с Android 16+ установка CA
вообще заблокирована из любых программных источников (shell,
`KeyChain.createInstallIntent` из приложения, и т.д.) — Google +
Samsung сделали процесс **user-initiated-only**. У этой блокировки
нет программного обхода без root.

Pane всё, что в его силах, делает за тебя:
- пушит `pane-ca.pem` в `/sdcard/Download/` — это единственная папка,
  которую системный CertInstaller-picker открывает по умолчанию (свои
  папки `/sdcard/Pane/` для него невидимы из-за SAF whitelist);
- предварительно чистит стейл `pane-ca*` файлы в `/sdcard/Pane/` и
  `/sdcard/Documents/` от прежних версий Pane, чтобы picker не подтянул
  не тот файл;
- в UI строки Devices показывает раскрывающийся блок с точной
  последовательностью кликов под твою прошивку, копируемым путём к
  файлу и напоминанием про lock-screen PIN.

### Прокси-настройка на устройстве

Pane выставляет на устройстве **сразу два** Global-settings ключа:

- `http_proxy=127.0.0.1:8888` — основное. Через него идут OkHttp,
  Retrofit, нативные стеки (libcurl), HttpURLConnection — всё что
  читает `ProxySelector.getDefault()`. Это **обязательно**: без него
  90% Android-приложений не пойдут через Pane (Chrome и WebView —
  пойдут, но это не то приложение которое тебе нужно отлаживать).
- `http_proxy_pac=http://127.0.0.1:8889/proxy.pac` — бонус для
  Chromium-based стеков (Chrome, WebView, Samsung Internet). Они
  предпочитают PAC.

PAC-only setup ломал OkHttp-приложения (банковские, MTS, многие
бизнес-приложения), поэтому Pane всегда ставит обе настройки.

### Helper APK — почему интернет не пропадает при выдёргивании USB

В прошлых версиях `http_proxy` оставался на устройстве после Stop
proxy или unplug USB → устройство ломилось в мёртвый `127.0.0.1:8888`
→ инета нет до Remove device. Теперь Pane ставит на телефон
companion APK `tech.thothlab.pane.helper` (~4 MB), который держит
heartbeat-сокет к Pane через adb-reverse `127.0.0.1:8890`.

Когда heartbeat умирает (USB выдернут, Pane закрыт, упал) —
foreground service на телефоне сам выставляет `http_proxy=:0` через
`WRITE_SECURE_SETTINGS` (без root и Magisk — Pane грантит этот
permission через `adb shell pm grant` при первой паре). Интернет
возвращается за ~6 секунд после потери коннекта.

Safety: helper трогает `http_proxy` только если его текущее
значение — то что Pane туда записал. Если ты руками выставишь свой
proxy, helper его не тронет.

В шторке уведомлений висит иконка «Pane connected» / «Pane
disconnected» — это твой единственный визуальный сигнал что Pane
сейчас взаимодействует с устройством.

Если companion APK не нужен (корпоративный VPN, MDM-политики),
удали его в Settings → Apps → Pane Helper. Потеряешь только
auto-cleanup при unplug — всё остальное продолжит работать.

### Когда что-то идёт не так

- **`adb not found`** — Pane ищет adb в `ANDROID_HOME`, `~/Library/
  Android/sdk`, `/opt/homebrew/bin`, `/usr/local/bin`. Если в Pane
  на странице Devices жёлтый баннер «Android tooling not found»,
  поставь `platform-tools` от Android SDK или Android Studio.
- **`adb reverse failed`** — обычно после переключения USB-кабеля
  или перезапуска adb-сервера. Нажми **Re-sync** на устройстве.
- **На телефоне «нет интернета» после Stop proxy / unplug USB** —
  начиная с v0.1.41 помогает companion APK: даже без подключения к
  компьютеру он сам снимает прокси на устройстве за ~6 сек. Если
  телефон не успел получить helper (первое подключение к этой
  машине Pane упало, или helper удалён вручную), восстанови
  настройку прямо на телефоне: **Settings → Wi-Fi → активная сеть →
  Proxy → None**. Или через adb если шнур подключён: `adb shell
  settings put global http_proxy :0`.
- **HTTPS не расшифровывается в моём приложении** — приложение должно
  доверять user CA. В debug-сборке достаточно
  `network_security_config.xml` с `<debug-overrides>` →
  `<trust-anchors><certificates src="user"/></trust-anchors>`. В
  release-сборке нужен явный opt-in. Chrome / Samsung Internet
  игнорируют user CA принципиально — для теста бери Firefox или
  своё приложение.
- **TLS pinning** — Pane не пытается обходить пиннинг. В debug-
  сборке выключи pinning в коде. Для own-device security research —
  Frida или Magisk-модули bypass'a поверх Pane.

## Тема и размер текста

**Settings → Appearance → Theme** переключает светлую/тёмную тему
(`System` следует настройке ОС). Выбор синхронизируется между всеми
открытыми окнами Pane, включая отдельные окна Logcat.

**Settings → Appearance → Text size** меняет масштаб всего UI:
четыре ступени `Small / Medium / Large / Extra large`. Меняется
`font-size` на `<html>`, поэтому все размеры в интерфейсе (тексты,
паддинги, иконки) растут пропорционально. По умолчанию — `Small`
(совпадает с поведением до 0.1.65).

## Язык интерфейса

Pane доступен на **English** и **русском**. По умолчанию — English.
Переключить: **Settings → Appearance → Language**. Выбор сохраняется
в `localStorage`, применяется реактивно без перезапуска приложения и
сразу синхронизируется со всеми открытыми окнами Pane, включая
отдельные окна Logcat.

Переведены все экраны UI (Captures, Rules, Devices, Settings, About,
Replay, body viewer, manual-install guide). Технические сообщения от
backend (`last_error` от `pane-android` / `pane-engine`) остаются
английскими — это совпадает с политикой логов и исходников.

Новый язык можно добавить за один файл: создай
`src/i18n/<lang>.ts` со структурой совпадающей с `en.ts`
(TypeScript-тип `Dict` это обеспечит), и зарегистрируй в `LOCALES`
массиве в `src/i18n/index.ts`.

## Обновления

Pane сам проверяет новые релизы:

- при запуске,
- каждый час пока окно открыто,
- при возврате окна в фокус.

Когда выходит новая версия — в sidebar под версией появляется кнопка
**Update to vX.Y.Z**. Жмёшь, Pane скачивает подписанный bundle (minisign)
и перезапускается. Принудительная проверка — **About → Check for updates**.

## Чтение captures

Строка поиска поддерживает маленький filter DSL:

```text
host:api.example.com               # только запросы к этому хосту
host:api.foo.com,api.bar.com       # OR — список альтернатив через запятую
method:POST,PUT,DELETE             # OR по методам
status:200,500..599                # mix: точный 200 OR диапазон 5xx
status:5..                         # любой 5xx
!error:tls_handshake               # исключить pinning + handshake failures
!host:cdn.*,fonts.*                # «ни cdn.*, ни fonts.*»
status:200..299 host:*.dev         # ranges + globs (между токенами AND)
google                             # bareword: substring of host or path
```

Сохранить текущий фильтр — иконка ☆, он появится в sidebar. Если в
диалоге Save ввести имя уже существующего фильтра, кнопка превращается
в **Update** — обновит query/color/pin без создания дубликата.

Между строками — тонкие hairline-разделители; ошибочные запросы (5xx
и транспортные ошибки — pinning, TLS-handshake и др.) подсвечиваются
красным целиком. Остальные статусы строку не красят, цветом
обозначается только код в колонке Status. Правый клик по строке
открывает меню «Добавить в правила» (см. [Подмена ответов](/docs/rules/)
→ «Быстрый способ: из Captures»).

Кнопка **Follow** на тулбаре прибивает список к последней записи.
Если её выключить (или прокрутить вверх) — список замирает: периодические
обновления приостанавливаются, пока ты не нажмёшь Follow обратно или не
проскроллишь до низа. Так можно спокойно читать старые записи без того,
чтобы таблица перерисовывалась под рукой. Смена фильтра при этом
всё равно срабатывает мгновенно.

Правая панель — **Overview / Request / Response / Timing / TLS**. Body
viewer определяет JSON / XML / text:

- **Tree** — разворачиваемые узлы, копирование по path или value.
- **Pretty** — отформатированный, подсвеченный текст.
- **Raw** — байты как пришли.

Разделитель между **Headers** и **Body** на вкладках Request/Response
двигается мышью; высота запоминается отдельно для каждой панели
(в localStorage) и переживает рестарт. Двойной клик по разделителю
сбрасывает к default.

## Logcat-окно

В **Devices** рядом с кнопкой **+ Add** для Android-устройств появилась кнопка **Logcat**. Клик — открывается отдельное **не модальное** окно с поток `adb logcat` от выбранного устройства. Можно одновременно смотреть captures в главном окне и Logcat в отдельном — окна независимы, по одному на устройство (повторный клик фокусирует существующее, не плодит дубликаты).

Что внутри окна:

- **Виртуализованная таблица** на 100k последних записей (~5 мин истории даже на болтливом firehose): время · PID · level (символ V/D/I/W/E/F) · tag · message. Вся строка раскрашена по уровню: `verbose/silent` — приглушённый, `debug` — синий, `info` — зелёный, `warn` — жёлтый, `error` — красный, `fatal` — жирный красный с лёгким красным фоном.
- **Pause** (Space) — буфер замирает, поток продолжается на бэке. **Clear** (⌘K) — чистит буфер. **Follow** — авто-прокрутка к новым строкам (отключается автоматически если пользователь сам прокрутил вверх).
- **Follow app** — дропдаун со списком установленных third-party пакетов. Выбираешь приложение → каждые 5 сек резолвится PID через `adb shell pidof`, фильтр накладывается автоматически. При рестарте приложения PID обновляется, фильтр продолжает работать без вмешательства.
- **Filter DSL** — собственный, in-memory (буфер уже в памяти, SQL не нужен):
  ```text
  OkHttp                              # bareword: substring в tag или message
  tag:OkHttp,Retrofit                 # позитивы через запятую — OR
  tag:!CatalogParser,!TrafficStats    # негативы через ! — все должны НЕ совпасть
  tag:!Spam,!Noise,SSH                # смешанно: (не Spam И не Noise) И содержит SSH
  level:E                             # только error
  level:W..F                          # диапазон, warn и выше
  pid:1234                            # конкретный PID
  app:com.foo,!com.foo.helper         # pid процессов com.foo минус pid com.foo.helper
  ~^(?!.*Connection)                  # regex (через ~), matches tag или message
  !tag:OkHttp                         # внешнее ! — инвертирует весь токен
  tag:OkHttp !msg:keep-alive          # AND между токенами
  ```
  Запятая в значении объединяет альтернативы: позитивы — OR, негативы (`!value`) — все должны НЕ совпасть, две группы соединяются по AND. Внешнее `!key:foo` инвертирует весь токен.
- **Export** — сохранение текущей отфильтрованной выборки в `.log` файл в формате `threadtime` (открывается в Android Studio / любой grep-цепочке).
- **⌘F** — фокус в поле фильтра.

Логика безопасности:
- При закрытии окна `adb logcat`-процесс убивается (`WindowEvent::Destroyed` → shutdown channel + `kill_on_drop`).
- На EOF/обрыве потока (USB-reseat, рестарт adb-server) — авто-реконнект с backoff 0.5s → 10s, до 5 попыток.
- Записи приходят на фронт **батчами** (50 строк или 100 мс, что раньше) чтобы 1000+ строк/сек firehose не клинил Solid-reactor.

## Дальше

- [Подмена ответов](/docs/rules/) — Stub / Patch правила.
- [Релизный процесс](/docs/reference/releases/) — для мейнтейнеров.
