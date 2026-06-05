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
   - на Android: установит маленький APK-хелпер
     (`tech.thothlab.pane.helper`, ~600 КБ, без иконки в drawer);
     хелпер вызывает системный `KeyChain.createInstallIntent()` и
     показывает один диалог «Установить Pane Root CA?»;
   - на iOS: пушит мобильный профиль через USB;
   - проложит `adb reverse tcp:8888 tcp:8888` (на iOS — usbmuxd
     tunnel) — трафик с устройства идёт через USB, **Wi-Fi не нужен
     и не настраивается**;
   - выставит системный HTTP-прокси на `127.0.0.1:8888`.
5. На телефоне подтверди диалог установки CA и введи PIN/паттерн.

Следующий запрос приложения окажется в списке captures. Клик по
строке открывает method / URL / status, заголовки, тело и timing.

### Зачем APK-хелпер на Android

На Android 11+ установка CA через `adb shell am start` бьётся в
системный picker файлов (scoped storage), а на Samsung One UI с
Android 16 — вообще блокируется с сообщением «приложение Оболочка
должно установить через Настройки». Хелпер запускает установку из
своего UID, не из shell — система пропускает. Снимок «один диалог
+ PIN, без файл-пикера». APK живёт на устройстве между сессиями,
переустанавливать не надо. Удаляется через **Настройки → Приложения**.

### Когда что-то идёт не так

- **`adb not found`** — Pane ищет adb в `ANDROID_HOME`, `~/Library/
  Android/sdk`, `/opt/homebrew/bin`, `/usr/local/bin`. Если в Pane
  на странице Devices жёлтый баннер «Android tooling not found»,
  поставь `platform-tools` от Android SDK или Android Studio.
- **`adb reverse failed`** — обычно после переключения USB-кабеля
  или перезапуска adb-сервера. Нажми **Re-sync** на устройстве.
- **На телефоне «нет интернета» после Stop proxy** — Pane c v0.1.12
  автоматически снимает прокси-настройку на всех paired Android при
  остановке. На более старых версиях: `adb shell settings put global
  http_proxy :0` руками, или **Remove device**.
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

## Обновления

Pane сам проверяет новые релизы:

- при запуске,
- каждый час пока окно открыто,
- при возврате окна в фокус.

Когда выходит новая версия — в sidebar под версией появляется кнопка
**Update to vX.Y.Z**. Жмёшь, Pane скачивает подписанный bundle (minisign)
и перезапускается. Принудительная проверка — иконка обновления рядом
с версией, или **About → Check for updates**.

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
