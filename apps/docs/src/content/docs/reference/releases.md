---
title: Релизный процесс
description: Выпуск версионного релиза Pane через теги и GitHub Actions matrix.
---


1. Создать релиз-ветку: `git checkout -b release/v0.X.Y`.
2. Поднять версии в `Cargo.toml` (workspace), `package.json`,
   `src-tauri/tauri.conf.json`. Все три должны совпадать.
3. Обновить `CHANGELOG.md` (будет добавлен в первом релизе).
4. Открыть PR, дождаться зелёного CI на всех трёх OS.
5. Смерджить в `main`, поставить тег: `git tag -a v0.X.Y -m "v0.X.Y"`.
6. Запушить тег: `git push --tags`. Триггерится
   `.github/workflows/release.yml`, который собирает бандлы для
   macOS Apple Silicon, Linux x86_64 и Windows x86_64 параллельно через
   `tauri-apps/tauri-action`, и создаёт **draft** GitHub Release с
   прикреплёнными инсталлерами.
7. Проверить draft на странице Releases, опубликовать.

Артефакты:

| Платформа | Файлы |
| --- | --- |
| macOS (aarch64) | `Pane_<ver>_aarch64.dmg`, `Pane.app.tar.gz` |
| Linux (x86_64)  | `pane_<ver>_amd64.AppImage`, `pane_<ver>_amd64.deb`, `pane-<ver>-1.x86_64.rpm` |
| Windows (x86_64) | `Pane_<ver>_x64_en-US.msi`, `Pane_<ver>_x64-setup.exe` |

Когда задан `TAURI_SIGNING_PRIVATE_KEY`, каждый бандл сопровождается
`.sig` minisign-подписью для updater-цепочки Tauri.

## GitHub secrets

Все secrets **опциональны** для v0.x — без них workflow всё равно
производит unsigned-бандлы. Каждый открывает дополнительную возможность.

| Secret | Назначение |
| ------ | ------- |
| `TAURI_SIGNING_PRIVATE_KEY` | Приватный minisign-ключ. Драйвит in-app updater. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Пароль ключа (пусто для незашифрованного). |
| `APPLE_CERTIFICATE` | Base64 Developer ID p12 — macOS-сборки без Gatekeeper-варнингов. |
| `APPLE_CERTIFICATE_PASSWORD` | Пароль p12. |
| `APPLE_SIGNING_IDENTITY` | например `Developer ID Application: ACME (TEAMID)`. |
| `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` | Notarisation. |
| `WIN_CERT_P12` / `WIN_CERT_PASSWORD` | Authenticode-подпись (без SmartScreen-варнинга). |
| `GPG_PRIVATE_KEY` / `GPG_PASSPHRASE` / `GPG_KEY_ID` | Detached `.asc`-подписи для Linux-артефактов. |

`tauri-action` читает Apple/Tauri-secrets прямо из окружения — см.
[`environmentVariables` в его README](https://github.com/tauri-apps/tauri-action)
для полного списка. Tauri updater key генерируется через
`pnpm tauri signer generate`.

## Updater endpoint

JSON-манифест на `releases.pane.tech/<channel>/<platform>/latest.json`.
Схема:

```jsonc
{
  "version": "0.1.1",
  "pub_date": "2026-06-10T12:00:00Z",
  "url": "https://github.com/thothlab/pane-app/releases/download/v0.1.1/Pane_0.1.1_x64.dmg",
  "signature": "<tauri updater signature>",
  "notes": "## What's new\n..."
}
```
