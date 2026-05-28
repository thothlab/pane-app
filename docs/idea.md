# Идея: «Charles, но без боли в настройке»

> *Рабочее имя проекта — `my-charles`. Финальный нейминг — отдельный шаг (см. §10).*

---

## 1. Vision

**Network debugger, в котором подключить iPhone или Android-устройство занимает 30 секунд, а не 15 минут с гуглением «how to install Charles certificate on iOS».**

Сегодня все Charles-подобные инструменты (Charles, Proxyman, Reqable, mitmproxy) делят одну и ту же проблему: **первые 5-15 минут пользователь воюет с системой**, прежде чем впервые видит трафик. Установка CA в двух местах настроек iOS, network security config на Android, ручная настройка proxy в Wi-Fi, разбор «почему не работает» при cert pinning. Этот барьер отсекает большую часть пользователей, которым «надо просто посмотреть запросы».

Идея проекта — **взять core-функционал Charles (MITM proxy + capture + replay + rewrites)** и **переделать слой setup'а так, чтобы новый девайс цеплялся за один QR-код или одну USB-команду**.

---

## 2. Target user

**Primary:**
- Mobile-разработчик, отлаживающий свой Android/iOS app в дев-сборке. Хочет видеть, что его SDK или бэкенд возвращают.
- Backend / full-stack dev, проверяющий, какие запросы шлёт фронт.
- QA, повторяющий баг с подменой ответа.

**Secondary:**
- Reverse-engineer / security researcher, смотрящий чужие приложения (где это легально и разрешено).

**Не target:**
- Production-network monitoring (это другая ниша — Wireshark / packet capture).
- Обход cert pinning в чужих apps без согласия (этический и юридический фронт — explicitly out of scope, см. §6).

---

## 3. Главная ставка — «one-command device setup»

Это сердце продукта. Всё остальное — следствие.

### Сценарий «как должно быть»

**iOS:**
1. Подключаешь iPhone USB → жмёшь «Add iOS device».
2. Приложение через `libimobiledevice` ставит CA в keychain устройства и включает proxy через usbmux-туннель.
3. Готово. Трафик летит. Без Settings, без Safari, без двойного trust-тапа.

*Fallback (без USB)*: показывается QR с одной страницей setup'а — открываешь камерой → автоматический deeplink в Safari → install profile + инструкции с гифками на двух экранах.

**Android:**
1. Подключаешь телефон USB с включённой USB debugging → жмёшь «Add Android device».
2. Приложение через `adb`:
   - Делает `adb reverse tcp:<port>` чтобы трафик шёл через localhost (без правки Wi-Fi).
   - Если устройство рутовано — ставит CA в system store.
   - Если нет — генерит для разработчика `network_security_config.xml` snippet для копипасты в его dev-сборку.
3. Готово.

*Fallback*: QR с Wi-Fi proxy конфигом + Android-инструкции.

### Что это даёт пользователю

- **30 секунд от запуска до первого перехваченного запроса** против 5-15 минут в Charles.
- **Один источник истины**: подключённые устройства живут в списке внутри приложения. Удаляешь устройство — снимаются и CA и proxy.
- **Меньше «не работает» тикетов**: если pinning заблокировал handshake — explicit toast с объяснением «вот этот хост использует cert pinning, инспекция невозможна без Frida», а не молчаливый таймаут как у Charles.

---

## 4. Core feature set (MVP)

Минимум, при котором продукт уже полезен:

1. **HTTPS MITM proxy** на локальном порту.
2. **Auto-generated root CA**, экспорт через QR / file / direct push на USB-устройство.
3. **Capture list** — request/response, URL, method, status, timing, размер.
4. **Detail panes** — headers, body (JSON pretty-print, image preview, hex для бинарного), timing waterfall.
5. **Filter / search** — по URL, methods, status, hostname.
6. **Replay** — кликнул запрос → правишь → отправляешь повторно. Стандартный API client flow.
7. **One-command device setup** для iOS USB / Android USB (§3).
8. **Cert pinning detection** — при handshake failure показываем pinned host и объясняем почему.

Это **~6-8 недель** на solo (без `mitmproxy-rs` как dependency) или **~4-5 недель** с ним.

---

## 5. Beyond MVP (по мере спроса)

Не строим всё сразу. Очередь будущих эпиков:

- **Map Local** — подменить ответ файлом с диска.
- **Map Remote** — перенаправить запросы с одного хоста на другой.
- **Rewrite rules** — regex/JS-функция трансформации requests/responses в полёте.
- **Breakpoints** — pause → edit → resume.
- **Throttling / bandwidth simulation** (3G/4G/slow Wi-Fi).
- **WebSocket / SSE / gRPC inspector**.
- **HAR import / export**.
- **Wi-Fi proxy mode (без USB)** — для устройств не на той же сети что хост.
- **WireGuard mode** (à la mitmproxy 8) — устройство ставит WG-конфиг и весь его трафик идёт через хост без proxy-настроек на девайсе.
- **Git-native rule sharing** — rewrites/breakpoints в YAML, шарятся через git (та же история что у Argos с workspace-форматом).

---

## 6. Что НЕ делаем

Чёткие границы важны, чтобы продукт не утонул в feature creep.

- **Не пытаемся обходить cert pinning.** Это пользовательская ответственность через Frida/Magisk/jailbreak. Мы детектим pinning и честно говорим «не получится».
- **Не делаем production monitoring** — это Wireshark / Datadog / Splunk territory.
- **Не делаем VPN / packet-level capture.** Только HTTP(S)/WebSocket/gRPC на application layer.
- **Не делаем traffic generation / load testing** (это k6 / JMeter).
- **Не делаем «hack чужие apps»** — продукт для своих сборок и для legitimate security work. В UI и доках это explicit.

---

## 7. Дифференциация от существующих

| | Charles | Proxyman | Reqable | mitmproxy | **my-charles** |
|---|---|---|---|---|---|
| Цена | $50 | $69/yr | freemium | free | **free / OSS** |
| OS | Win/Mac/Linux | Mac only | Win/Mac/Linux/iOS/Android | Win/Mac/Linux | **Win/Mac/Linux** (Tauri) |
| UI | dated | modern | modern | TUI + web | **modern, focused** |
| Setup mobile | ручной | ручной | ручной | ручной (+WG) | **one-command** ← ставка |
| HTTP/2 | да | да | да | да | да |
| HTTP/3 | β | нет | нет | β | план |
| Rewrites | да | да | да | да (Python) | план |
| Git-friendly config | нет | нет | нет | нет | **да** ← Argos-DNA |
| Cert pinning detect | нет (молча) | нет | частично | manual | **да + объяснение** |

**Ключевые отличия** (то, что делает продукт «не очередным Charles»):

1. **Setup в один шаг через USB.** Никто из конкурентов не делает auto-install CA через usbmux/adb. Это технически достижимо и даёт огромное UX-преимущество.
2. **Honest cert pinning UX.** Все молча показывают handshake error. Мы — объясняем что произошло и куда идти дальше.
3. **Git-native rules.** Rewrites хранятся в проектной папке (как Argos workspace), шарятся между разработчиками через PR. У Charles `.xml` файлы локальные, у Proxyman ситуация чуть лучше но не git-first.
4. **OSS + free.** Closing the gap с mitmproxy по доступности, но с user-friendly UI которого у mitmproxy нет.

---

## 8. Возможный tech stack

*Конкретные выборы — задача technical notes / dev-spec; здесь — гипотеза для оценки риска.*

- **Shell**: Tauri 2 (как Argos — тот же подход).
- **UI**: SolidJS + Tailwind (как Argos).
- **Proxy engine**: open вопрос — embed `mitmproxy-rs` как dependency, либо писать свой на `hyper` + `rustls` + `rcgen`. **Рекомендую начать с embed `mitmproxy-rs`** — экономит ~2 месяца на TLS-нюансах.
- **Device control**:
  - **iOS**: `libimobiledevice` через FFI или standalone CLI как sidecar.
  - **Android**: `adb` бинарь как sidecar (bundled).
- **Storage**: SQLite для capture log (потенциально много гигов — нужна индексация по host/time/status).
- **Cert generation**: `rcgen`.

### Можем ли переиспользовать что-то из Argos?

Частично, но **отдельный проект и отдельный репо**:

- UI компоненты (CodeEditor, ResponsePane, Splitter, Toaster) — можно extract в shared package если оба проекта станут серьёзными.
- HTTP типы (HttpRequest, HttpResponse) — concept похож, но capture-domain имеет больше полей (client_ip, server_ip, TLS info).
- **Не пытаемся** делать монорепо «всё под Argos» — это два разных продукта, отдельные циклы релизов и аудиторий.

---

## 9. Главные риски

1. **`libimobiledevice` на iOS 17+ работает нестабильно**, особенно с usbmuxd2. План B — Wi-Fi proxy mode с auto-generated mobileconfig.
2. **adb on Android 14+** — некоторые OEM (Xiaomi, Huawei) блокируют CA install даже через root. План B — открытое признание что для не-AOSP устройств может не работать.
3. **TLS 1.3 + HTTP/3 (QUIC)** — растёт доля трафика, который не пройдёт через классический HTTP-MITM. QUIC требует UDP-proxy, который сложнее. Откладываем до спроса, в MVP — клиенты автоматически даунгрейдят до HTTP/2 если QUIC не работает (как делают все).
4. **Cert pinning у большинства production apps** — фундаментальный лимит. UX-mitigation описан в §6, технически не лечится.
5. **Юридический фронт** — продукт может использоваться для reverse engineering чужого софта. Lighter than Charles' positioning (никто не приходит за Charles в суд), но License + Terms должны быть аккуратные. Apache-2.0 + явный disclaimer в README.

---

## 10. Naming

**Решено (2026-05-28): финальный кандидат — Pane.** Pending domain availability + trademark check (USPTO / EUIPO кат. 9 и 42). Codebase остаётся `my-charles` до подтверждения; rebrand — отдельный задачник перед v0.1.0-beta.

Обоснование выбора (см. ниже список кандидатов):
- Гомофон «pain» работает на USP проекта: «no pain to setup».
- UI-native — пользователи уже «открывают pane».
- Чистое trademark-поле в dev-tools нише (в отличие от Scope, где плотно).
- Family с Linear / Raycast / Vercel — короткое, с подтекстом.

Перед rebrand'ом — провести валидацию (произнести вслух 5 коллегам; если ≥3 переспросят «Pain?», вернуться к Scope или Lens).

Текущее «my-charles» — рабочее. До первого публичного релиза нужно финальное имя. Критерии:

- 1-2 слога, легко произносится.
- Метафора прозрачности / просмотра трафика.
- Доступный домен (.com / .tech / .dev).
- Не конфликтует с trademarks (Charles, Proxyman, Reqable, mitmproxy, Burp).

Кандидаты для проработки:
- **Lens** — оптическая метафора, простое имя. Возможно занято.
- **Tap** — «прослушка / краник», техническое + короткое.
- **Glimpse** — «беглый взгляд», нейтрально.
- **Argus** (не путать с Argos!) — стоглазый страж в мифологии, но слишком близко к имени соседнего проекта.
- **Looking glass** — «зеркало», поэтично, но длинно.
- **Peek** — «быстрый взгляд», повторяет ценность «открыл → увидел». 1 слог, дружелюбный. Risk: `.com` занят (Peek travel), нужно проверять `.dev`/`.tech`.
- **Pane** — оконное стекло (прозрачное) + «detail pane» из UI как двойной смысл; гомофон с «pain» → мнемоника «no pain to setup». 1 слог, технически грамотный pun.
- **Scope** — оскиллоскоп / микроскоп; сильная инженерная коннотация, family с `curl`/`dig`/`nmap`. Очень общее слово, много trademark'ов с префиксом.
- **Trace** — `traceroute` heritage, «проследить путь запроса». 1 слог, networking-DNA. Termin широко используется в сетях — нужен сильный логотип.
- **Lume** — свет / *lumen*; «осветить трафик». Мягко-модное звучание (Linear, Raycast). Менее очевидная связь с сетью.

**Текущий топ для проработки:** Pane (двойной смысл + неожиданность) и Scope (professional dev-tool звучание).

Финальное имя выбираем после BRD, когда позиционирование зацементируется.

---

## 11. Следующие шаги

1. **BRD** — business requirements: who, what, why в деталях, success metrics, MVP scope frozen.
2. **Technical notes** — конкретный выбор: `mitmproxy-rs` embed vs свой engine, как именно USB-cert-push работает, схема storage.
3. **Developer specification** — модули, API, схемы данных.
4. **Implementation plan** — декомпозиция на эпики/задачи с эстимейтами.
5. **Naming + landing** — параллельно с BRD.

> Документы складываем в `docs/` (как в Argos), таски в `tasks/`. По мере публикации репо — внутренние материалы могут уехать в Obsidian Vault (как у Argos после v0.1.0).
