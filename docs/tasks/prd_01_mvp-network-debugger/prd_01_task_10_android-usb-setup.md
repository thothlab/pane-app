# Task 10 — Android USB one-command setup (★ ключевая ставка)

## Goal
По кнопке "Add Android device": `adb reverse` для proxy без правки Wi-Fi настроек, CA install в system store (если root) или генерация `network_security_config.xml` snippet для dev-сборки (если no-root). Первый capture за 30 секунд.

## Scope
**In:**
- `adb` discovery, authorization handshake.
- `adb reverse tcp:<port> tcp:<port>` для proxy redirect.
- CA install в system store через `adb root + adb remount + push`.
- Если no-root: генерация `network_security_config.xml` snippet + копирование в clipboard + wizard "вставить в свой проект".
- Detection: rooted? OEM-блокировка system CA?
- Cleanup на remove.

**Out:**
- Wi-Fi fallback (Task 11).
- Magisk-module автоустановка — beyond MVP (документация).

## Subtasks

### 10.1 adb authorization
- [ ] `adb devices -l` — если status `unauthorized` → UI: "Allow USB debugging on your device".
- [ ] Poll каждые 2 s до `device`-status.
- [ ] Timeout 60 s.

### 10.2 Capability probe
- [ ] `adb shell which su` → есть/нет root.
- [ ] `adb shell getprop ro.build.version.release` → Android version.
- [ ] `adb shell getprop ro.product.manufacturer` → OEM (Samsung/Xiaomi/Pixel/...).
- [ ] Persist в `Device.capabilities_json`.

### 10.3 Path A — rooted: system CA install
- [ ] `adb root` → restart adbd as root (если поддерживается).
- [ ] `adb remount` (или `mount -o rw,remount /system`).
- [ ] Подготовить CA в формате Android system store: `<hash>.0` (hash = OpenSSL subject_hash_old).
- [ ] `adb push <ca-file> /system/etc/security/cacerts/<hash>.0`.
- [ ] `adb shell chmod 644` + `chcon u:object_r:system_file:s0`.
- [ ] Для Android 14+: `/apex/com.android.conscrypt/cacerts/` (если applicable) — Magisk-style mount.
- [ ] Verify: `adb shell trust list | grep my-charles`.

### 10.4 Path B — no-root: network_security_config helper
- [ ] Генерим snippet:
  ```xml
  <network-security-config>
      <debug-overrides>
          <trust-anchors>
              <certificates src="@raw/my_charles_ca"/>
              <certificates src="system"/>
          </trust-anchors>
      </debug-overrides>
  </network-security-config>
  ```
- [ ] Copy в clipboard + сохранить PEM как файл, открыть Finder/Explorer на нём.
- [ ] Wizard в UI: "1. Add file to `res/raw/my_charles_ca.crt`, 2. Save XML to `res/xml/network_security_config.xml`, 3. Reference in AndroidManifest: `android:networkSecurityConfig=...`, 4. Rebuild and install debug build".
- [ ] Если у разработчика уже есть `network_security_config.xml` — даём вариант "merge" с подсветкой что нужно добавить.

### 10.5 adb reverse proxy redirect
- [ ] `adb reverse tcp:8888 tcp:8888` — устройство шлёт на `localhost:8888` → хост `localhost:8888`.
- [ ] Установка системного proxy для device: `adb shell settings put global http_proxy 127.0.0.1:8888`.
- [ ] Для Wi-Fi-only apps: некоторые apps игнорируют system proxy — в этом случае помогает только VPN-mode (out of scope MVP) или manual через app config.

### 10.6 OEM warning matrix
| OEM | Android | Issue | UX |
|---|---|---|---|
| Samsung One UI 6+ | 14+ | system store блокируется | Path B forced + warning banner |
| Xiaomi MIUI 14+ | 13+ | adb remount часто падает | Try / catch + clear error |
| Huawei | any | adb root not available | Path B forced |
| Pixel | any | works clean | Path A |

### 10.7 Cleanup на remove
- [ ] `adb reverse --remove tcp:8888`.
- [ ] `adb shell settings put global http_proxy :0` (сброс).
- [ ] Если CA installed в system: `adb shell rm /system/etc/security/cacerts/<hash>.0` (требует remount).

### 10.8 Error matrix → UX
| Error | Cause | UI |
|---|---|---|
| `adb_unauthorized` | RSA prompt не подтверждён | wizard "Allow USB debugging" |
| `root_unavailable` | non-rooted device | автоматически Path B |
| `remount_failed` | OEM lock | "Path A не сработал — переключаемся на Path B" |
| `chcon_unavailable` | старый SELinux | warning, CA может не trust'ся |

## Deliverables
- `crates/mycharles-android/`.
- `src/views/devices/android-wizard/`.
- Документ `docs/android-setup-matrix.md` (OEM × Android version × что работает).

## Definition of Done
- [ ] AC2 из PRD: cold start → first capture ≤ 30 s на Pixel 8 / Android 14.
- [ ] Path A работает на rooted Pixel.
- [ ] Path B генерит корректный snippet — собранный с ним dev-app trust'ит CA (manual verification на demo-app).
- [ ] OEM matrix: для каждого OEM в матрице — ясный UX путь (либо work, либо clear "switch to Path B").
- [ ] Cleanup на remove работает (CA удалён, proxy сброшен).

## Tests
- **Manual matrix:** Pixel 8 (root + non-root), Samsung S23, Xiaomi 13.
- **Unit:** Android `subject_hash_old` алгоритм — сравнить с `openssl x509 -in ca.pem -subject_hash_old -noout`.
- **Unit:** XML snippet generator — validate против schema.
- **Integration:** mock adb (stub `adb shell` команд) — sequence verification.

## Dependencies
- Task 08 (device manager).
- Task 02 (CA).
- Task 04 (IPC).
