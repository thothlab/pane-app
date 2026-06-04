#!/usr/bin/env python3
"""Merge per-platform Tauri updater manifests into one `latest.json`.

Tauri-action emits a separate `*.json` file alongside each bundle,
each containing a `platforms` map with a single entry. We need one
combined manifest the way `argos-web`'s `/api/update/...` route reads
from disk.

Usage:
    merge-tauri-manifests.py <dir-with-*.json> > latest.json

Doesn't depend on PyYAML or anything fancy — just stdlib so it runs
on a vanilla `ubuntu-latest` runner.
"""

import json
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    src = Path(sys.argv[1])
    if not src.is_dir():
        print(f"not a directory: {src}", file=sys.stderr)
        return 1

    combined: dict = {
        "version": "",
        "notes": "",
        "pub_date": "",
        "platforms": {},
    }
    for f in sorted(src.glob("*.json")):
        try:
            doc = json.loads(f.read_text())
        except json.JSONDecodeError as e:
            print(f"skip {f.name}: {e}", file=sys.stderr)
            continue
        for k in ("version", "notes", "pub_date"):
            if doc.get(k) and not combined[k]:
                combined[k] = doc[k]
        for platform, entry in (doc.get("platforms") or {}).items():
            combined["platforms"][platform] = entry

    if not combined["platforms"]:
        print("no platforms found — was tauri-action successful?", file=sys.stderr)
        return 1
    json.dump(combined, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
