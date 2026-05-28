# Android setup matrix

How `device.add_android_usb` behaves per (OEM × Android version × root state).

| Path | Trigger | Outcome |
| ---- | ------- | ------- |
| A — system store install | `adb root` succeeds + `adb remount` works | CA trusted by every app on the device. |
| B — debug-build snippet  | Path A fails (non-rooted or OEM lock) | We generate a `network_security_config.xml` + copy CA PEM. User rebuilds their dev app with these resources. |

In both cases we run `adb reverse tcp:8888 tcp:8888` and set
`settings put global http_proxy 127.0.0.1:8888` so the device routes traffic
through the desktop without Wi-Fi changes.

## OEM-specific notes

| OEM         | Android | Notes |
| ----------- | ------- | ----- |
| Pixel       | 13/14   | Path A clean if rooted. |
| Samsung One UI | 14/15 | system store locked → Path B forced. |
| Xiaomi MIUI | 13/14   | `adb remount` flaky; retry then Path B. |
| Huawei      | any     | `adb root` unavailable; Path B only. |

## Subject hash

Android uses an OpenSSL `subject_hash_old` derived 8-hex value for filenames.
The MVP `subject_hash_old` helper in `mycharles-android` is a sha-based
approximation suitable for fresh installs; once the upstream OpenSSL algorithm
is reproduced exactly, rotating in the correct value is a one-line change in
`install_system_ca`.
