# Release process

1. Cut a release branch: `git checkout -b release/v0.X.Y`.
2. Bump versions in `Cargo.toml` (workspace), `package.json`, `src-tauri/tauri.conf.json`.
3. Update `CHANGELOG.md` (to be added on first release).
4. Open PR, wait for green CI on all three OSes.
5. Tag: `git tag -a v0.X.Y -m "v0.X.Y"`.
6. Push tag: `git push --tags`. This triggers `.github/workflows/release.yml`,
   which builds + signs + uploads artefacts to GitHub Releases.

## Required GitHub secrets

| Secret | Purpose |
| ------ | ------- |
| `APPLE_CERTIFICATE` | Base64 of Developer ID p12. |
| `APPLE_CERTIFICATE_PASSWORD` | p12 password. |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: ACME (TEAMID)`. |
| `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` | Notarisation. |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater signing key. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for the above. |
| `WIN_CERT_P12` / `WIN_CERT_PASSWORD` | Authenticode signing (optional initially). |
| `GPG_KEY` | Linux AppImage/.deb signing (optional initially). |

## Updater endpoint

Hosted JSON manifest at `releases.my-charles.tech/<channel>/<platform>/latest.json`.
Schema:

```jsonc
{
  "version": "0.1.1",
  "pub_date": "2026-06-10T12:00:00Z",
  "url": "https://github.com/thothlab/my-charles/releases/download/v0.1.1/my-charles_0.1.1_x64.dmg",
  "signature": "<tauri updater signature>",
  "notes": "## What's new\n..."
}
```
