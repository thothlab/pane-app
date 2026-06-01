#!/usr/bin/env bash
# Pane installer for macOS.
#
# Downloads the latest .dmg, copies Pane.app to /Applications, and
# strips the com.apple.quarantine attribute so macOS doesn't refuse
# to open the unsigned build with a misleading "damaged" message.
#
# Usage:
#   curl -fsSL https://pane.thothlab.tech/install-macos.sh | bash
set -euo pipefail

PANE_HOST="${PANE_HOST:-https://pane.thothlab.tech}"
APP_NAME="Pane.app"
DEST_DIR="/Applications"

if [[ "$(uname)" != "Darwin" ]]; then
  echo "error: this installer is for macOS only." >&2
  exit 1
fi

case "$(uname -m)" in
  arm64)  target="macos-aarch64" ;;
  x86_64) target="macos-x64" ;;
  *)      echo "error: unsupported arch $(uname -m)" >&2; exit 1 ;;
esac

tmp="$(mktemp -d -t pane)"
mount_point=""
cleanup() {
  if [[ -n "$mount_point" && -d "$mount_point" ]]; then
    hdiutil detach -quiet "$mount_point" >/dev/null 2>&1 || true
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

dmg="$tmp/Pane.dmg"
echo "==> Downloading Pane ($target)"
curl -fL --progress-bar -o "$dmg" "$PANE_HOST/download/$target"

echo "==> Mounting installer"
attach_out="$(hdiutil attach -nobrowse -readonly -noverify "$dmg")"
mount_point="$(printf '%s\n' "$attach_out" | awk -F'\t' '$NF ~ /^\/Volumes\//{print $NF}' | tail -n1)"

if [[ -z "$mount_point" || ! -d "$mount_point/$APP_NAME" ]]; then
  echo "error: could not find $APP_NAME inside the dmg" >&2
  exit 1
fi

dest="$DEST_DIR/$APP_NAME"
if [[ -d "$dest" ]]; then
  echo "==> Removing existing $dest"
  rm -rf "$dest" 2>/dev/null || sudo rm -rf "$dest"
fi

echo "==> Copying to $DEST_DIR"
if ! cp -R "$mount_point/$APP_NAME" "$DEST_DIR/" 2>/dev/null; then
  echo "    (need elevated permissions for $DEST_DIR)"
  sudo cp -R "$mount_point/$APP_NAME" "$DEST_DIR/"
fi

echo "==> Stripping quarantine attribute"
xattr -dr com.apple.quarantine "$dest" 2>/dev/null || sudo xattr -dr com.apple.quarantine "$dest" || true

echo "==> Launching Pane"
open "$dest"

echo "OK"
