---
title: Release process
description: Cut a versioned Pane release via tags and the GitHub Actions matrix.
---


1. Cut a release branch: `git checkout -b release/v0.X.Y`.
2. Bump versions in `Cargo.toml` (workspace), `package.json`, `src-tauri/tauri.conf.json`.
   All three must agree.
3. Update [`CHANGELOG.md`](https://github.com/thothlab/pane-app/blob/main/CHANGELOG.md).
   That version's section is then copied by hand into the GitHub Release body
   (`gh release edit v0.X.Y --notes-file …`), because the workflow fills in a
   generic template. Do it **after the whole workflow has finished**: every
   platform build rewrites the body with the template again, so an earlier
   edit gets overwritten.
4. Open PR, wait for green CI on all three OSes.
5. Merge into `main`, then tag: `git tag -a v0.X.Y -m "v0.X.Y"`.
6. Push tag: `git push --tags`. This triggers `.github/workflows/release.yml`,
   which builds bundles for macOS Apple Silicon, Linux x86_64, and Windows
   x86_64 in parallel via `tauri-apps/tauri-action`, then creates a **draft**
   GitHub Release with the installers and the macOS/Linux `pane-cli`
   attached. Once all three builds pass, the `publish` job merges the
   per-platform manifests into `latest.json`
   (`.github/scripts/merge-tauri-manifests.py`), uploads it, and
   **publishes the release itself**.
7. Nothing to publish by hand. If any build fails, `publish` doesn't run
   and the release stays a draft — invisible to users and to the in-app
   updater.

Artefacts produced:

| Platform | Files |
| --- | --- |
| macOS (aarch64) | `Pane_<ver>_aarch64.dmg`, `Pane_aarch64.app.tar.gz` (for the updater), `pane-cli-darwin-aarch64` |
| Linux (x86_64)  | `Pane_<ver>_amd64.AppImage`, `Pane_<ver>_amd64.deb`, `Pane-<ver>-1.x86_64.rpm`, `pane-cli-linux-x86_64` |
| Windows (x86_64) | `Pane_<ver>_x64_en-US.msi`, `Pane_<ver>_x64-setup.exe` |

Plus one combined `latest.json` — the in-app updater manifest.

When `TAURI_SIGNING_PRIVATE_KEY` is set, every bundle except the `.dmg`
ships with a matching `.sig` minisign signature for the Tauri updater chain.

## GitHub secrets

All secrets are **optional** for v0.x — without them the workflow still
produces unsigned bundles. Each one unlocks an extra capability.

| Secret | Purpose |
| ------ | ------- |
| `TAURI_SIGNING_PRIVATE_KEY` | Minisign private key. Drives the in-app updater. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Passphrase of the above (empty for an unencrypted key). |
| `APPLE_CERTIFICATE` | Base64 of Developer ID p12 — Gatekeeper-friendly macOS builds. |
| `APPLE_CERTIFICATE_PASSWORD` | p12 password. |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: ACME (TEAMID)`. |
| `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` | Notarisation. |
| `WIN_CERT_P12` / `WIN_CERT_PASSWORD` | Authenticode signing (no SmartScreen warning). |
| `GPG_PRIVATE_KEY` / `GPG_PASSPHRASE` / `GPG_KEY_ID` | Detached `.asc` signatures for the Linux artefacts. |

`tauri-action` reads the Apple/Tauri secrets straight from the
environment — see the [`environmentVariables` section of its
README](https://github.com/tauri-apps/tauri-action) for the full list.
Generate the Tauri updater key with `pnpm tauri signer generate`.

## Updater endpoint

The in-app updater reads
`https://github.com/thothlab/pane-app/releases/latest/download/latest.json`
(`plugins.updater.endpoints` in `src-tauri/tauri.conf.json`). `/releases/latest/`
resolves only to a published release — never a draft or a prerelease — which
is why the workflow keeps the release a draft until `latest.json` is uploaded.

Schema (abridged):

```jsonc
{
  "version": "0.3.6",
  "pub_date": "2026-10-10T06:51:23.281Z",
  "notes": "…",
  "platforms": {
    "darwin-aarch64": { "url": ".../v0.3.6/Pane_aarch64.app.tar.gz", "signature": "…" },
    "linux-x86_64":   { "url": ".../v0.3.6/Pane_0.3.6_amd64.AppImage", "signature": "…" },
    "windows-x86_64": { "url": ".../v0.3.6/Pane_0.3.6_x64_en-US.msi", "signature": "…" }
    // plus per-installer keys: linux-x86_64-deb, windows-x86_64-nsis, etc.
  }
}
```

`notes` lands in the manifest from the release body at build time — the
generic template, not the CHANGELOG. The app doesn't display it.
