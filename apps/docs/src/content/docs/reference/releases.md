---
title: Релизный процесс
description: Выпуск версионного релиза Pane через теги и GitHub Actions matrix.
---


1. Создать релиз-ветку: `git checkout -b release/v0.X.Y`.
2. Поднять версии в `Cargo.toml` (workspace), `package.json`,
   `src-tauri/tauri.conf.json`. Все три должны совпадать.
3. Обновить [`CHANGELOG.md`](https://github.com/thothlab/pane-app/blob/main/CHANGELOG.md) —
   секцию этой версии потом руками кладут в тело GitHub Release
   (`gh release edit v0.X.Y --notes-file …`), потому что workflow подставляет
   общий шаблон. Делать это **после того, как workflow целиком завершился**:
   каждая платформенная сборка заново перезаписывает тело шаблоном, и
   правка, сделанная раньше, затрётся.
4. Открыть PR, дождаться зелёного CI на всех трёх OS.
5. Смерджить в `main`, поставить тег: `git tag -a v0.X.Y -m "v0.X.Y"`.
6. Запушить тег: `git push --tags`. Триггерится
   `.github/workflows/release.yml`, который собирает бандлы для
   macOS Apple Silicon, Linux x86_64 и Windows x86_64 параллельно через
   `tauri-apps/tauri-action`, и создаёт **draft** GitHub Release с
   прикреплёнными инсталлерами и `pane-cli` для macOS и Linux. Когда все
   три сборки прошли, job `publish` склеивает `latest.json` из
   платформенных манифестов (`.github/scripts/merge-tauri-manifests.py`),
   загружает его в релиз и **публикует релиз сам**.
7. Руками публиковать ничего не нужно. Если хотя бы одна сборка упала,
   `publish` не запускается и релиз остаётся черновиком — пользователи
   и встроенное обновление его не видят.

Артефакты:

| Платформа | Файлы |
| --- | --- |
| macOS (aarch64) | `Pane_<ver>_aarch64.dmg`, `Pane_aarch64.app.tar.gz` (для обновления), `pane-cli-darwin-aarch64` |
| Linux (x86_64)  | `Pane_<ver>_amd64.AppImage`, `Pane_<ver>_amd64.deb`, `Pane-<ver>-1.x86_64.rpm`, `pane-cli-linux-x86_64` |
| Windows (x86_64) | `Pane_<ver>_x64_en-US.msi`, `Pane_<ver>_x64-setup.exe` |

Плюс общий `latest.json` — манифест встроенного обновления.

Когда задан `TAURI_SIGNING_PRIVATE_KEY`, каждый бандл, кроме `.dmg`,
сопровождается `.sig` minisign-подписью для updater-цепочки Tauri.

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

Встроенное обновление читает
`https://github.com/thothlab/pane-app/releases/latest/download/latest.json`
(`plugins.updater.endpoints` в `src-tauri/tauri.conf.json`). `/releases/latest/`
указывает только на опубликованный релиз — не на черновик и не на
prerelease, — поэтому workflow держит релиз черновиком, пока `latest.json`
не загружен.

Схема (сокращено):

```jsonc
{
  "version": "0.3.6",
  "pub_date": "2026-10-10T06:51:23.281Z",
  "notes": "…",
  "platforms": {
    "darwin-aarch64": { "url": ".../v0.3.6/Pane_aarch64.app.tar.gz", "signature": "…" },
    "linux-x86_64":   { "url": ".../v0.3.6/Pane_0.3.6_amd64.AppImage", "signature": "…" },
    "windows-x86_64": { "url": ".../v0.3.6/Pane_0.3.6_x64_en-US.msi", "signature": "…" }
    // плюс ключи по типу установщика: linux-x86_64-deb, windows-x86_64-nsis и т. д.
  }
}
```

`notes` попадает в манифест из тела релиза в момент сборки, то есть из
общего шаблона, а не из CHANGELOG. Приложение это поле не показывает.
