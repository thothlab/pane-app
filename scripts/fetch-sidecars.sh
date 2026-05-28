#!/usr/bin/env bash
# Fetches sidecar binaries (libimobiledevice CLIs + adb) per target triple.
# Real bundling is platform-specific; for MVP this script documents intent
# and bails informatively when run on a platform without the matching artefact.

set -euo pipefail

target_triple() {
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)  echo "aarch64-apple-darwin" ;;
    Darwin-x86_64) echo "x86_64-apple-darwin" ;;
    Linux-x86_64)  echo "x86_64-unknown-linux-gnu" ;;
    *) echo "unknown" ;;
  esac
}

TRIPLE="$(target_triple)"
DEST="src-tauri/binaries/${TRIPLE}"
mkdir -p "${DEST}"

cat <<MSG
Sidecar destination: ${DEST}

Place the following binaries here before \`pnpm tauri build\`:
  - adb
  - idevice_id
  - ideviceinfo
  - idevicepair
  - ideviceinstaller
  - idevicesyslog
  - iproxy

On macOS: \`brew install libimobiledevice android-platform-tools\` then copy
the universal binaries from \$(brew --prefix). On Linux: apt-get equivalents.
On Windows: imobiledevice-net + platform-tools-latest-windows.

This script intentionally does not auto-download — sidecar provenance is a
security-sensitive decision and is best controlled by the maintainer.
MSG
