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
   - **Android (без root)** — пушит файл `/sdcard/Pane/pane-ca.pem`
     на устройство, в строке Devices показывает пошаговую инструкцию
     «Settings → Install certificate → выбрать pane-ca.pem» (см. ниже);
   - проложит `adb reverse tcp:8888 tcp:8888` + `tcp:8889 tcp:8889`
     (для PAC) — трафик с устройства идёт через USB, **Wi-Fi не
     нужен и не настраивается**;
   - выставит на устройстве `http_proxy_pac` на локальный PAC-сервер.
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
- пушит `pane-ca.pem` в собственную папку `/sdcard/Pane/`, которая не
  подвержена авточистке Samsung Smart Manager (Downloads чистится);
- в UI строки Devices показывает раскрывающийся блок с точной
  последовательностью кликов под твою прошивку, копируемым путём к
  файлу и напоминанием про lock-screen PIN.

### PAC-based proxy

Pane выставляет на устройстве `http_proxy_pac`, а не direct `http_proxy`.
Когда USB подключён — PAC-сервер доступен через `adb reverse`, и
трафик идёт через Pane. Когда USB отключают (или Pane закрывается) —
PAC URL становится недоступен, Android прозрачно делает fallback на
DIRECT, и устройство **сохраняет интернет**. Прежний прямой
http_proxy в этом сценарии «закрывал» интернет до Remove device.

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

## Язык интерфейса

Pane доступен на **English** и **русском**. По умолчанию — English.
Переключить: **Settings → Appearance → Language**. Выбор сохраняется
в `localStorage` и применяется реактивно, без перезапуска приложения.

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
