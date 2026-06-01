# Release process

1. Cut a release branch: `git checkout -b release/v0.X.Y`.
2. Bump versions in `Cargo.toml` (workspace), `package.json`, `src-tauri/tauri.conf.json`.
   All three must agree.
3. Update `CHANGELOG.md` (to be added on first release).
4. Open PR, wait for green CI on all three OSes.
5. Merge into `main`, then tag: `git tag -a v0.X.Y -m "v0.X.Y"`.
6. Push tag: `git push --tags`. This triggers `.github/workflows/release.yml`,
   which builds bundles for macOS Apple Silicon, Linux x86_64, and Windows
   x86_64 in parallel via `tauri-apps/tauri-action`, then creates a **draft**
   GitHub Release with the installers attached.
7. Review the draft on the Releases page, then publish it.

Artefacts produced:

| Platform | Files |
| --- | --- |
| macOS (aarch64) | `Pane_<ver>_aarch64.dmg`, `Pane.app.tar.gz` |
| Linux (x86_64)  | `pane_<ver>_amd64.AppImage`, `pane_<ver>_amd64.deb`, `pane-<ver>-1.x86_64.rpm` |
| Windows (x86_64) | `Pane_<ver>_x64_en-US.msi`, `Pane_<ver>_x64-setup.exe` |

When `TAURI_SIGNING_PRIVATE_KEY` is set, each bundle also ships with a
matching `.sig` minisign signature for the Tauri updater chain.

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

Hosted JSON manifest at `releases.pane.tech/<channel>/<platform>/latest.json`.
Schema:

```jsonc
{
  "version": "0.1.1",
  "pub_date": "2026-06-10T12:00:00Z",
  "url": "https://github.com/thothlab/pane-app/releases/download/v0.1.1/Pane_0.1.1_x64.dmg",
  "signature": "<tauri updater signature>",
  "notes": "## What's new\n..."
}
```
