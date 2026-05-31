# Sidecar binaries

Pane ships two families of CLI sidecars:

- `adb` — Android Platform Tools, used by `pane-android`.
- `idevice_id`, `ideviceinfo`, `idevicepair`, `ideviceinstaller`, `idevicesyslog`, `iproxy` — libimobiledevice, used by `pane-ios`.

These live under `src-tauri/binaries/<target-triple>/` and are bundled into
the Tauri build per platform.

## Adding sidecars

```bash
./scripts/fetch-sidecars.sh
```

The script prints platform-specific instructions and does **not** auto-download
to keep provenance under the maintainer's control. Verify checksums against
the upstream release before checking new artefacts into the repo (or, better,
into a release-asset bucket referenced by `scripts/fetch-sidecars.sh`).

## Running without sidecars (dev mode)

If a sidecar is missing the matching device flow surfaces a `tooling_missing`
error and the UI shows actionable copy ("install Android Platform Tools / put
`adb` on PATH"). The rest of the app stays functional.
