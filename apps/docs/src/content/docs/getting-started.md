---
title: Getting started
description: Install Pane, add a device, see your first capture.
---

This page walks you from "haven't downloaded Pane yet" to "every request my app
makes shows up in the capture list".

## Install

Grab the latest build for your OS from the [download page](https://pane.thothlab.tech/#download).

| Platform | File |
| --- | --- |
| macOS Apple Silicon | `.dmg` |
| Linux x86_64 | `.AppImage` (portable) or `.deb` / `.rpm` |
| Windows x86_64 | `.msi` (recommended) or `.exe` NSIS installer |

### macOS first launch

Closed-alpha builds aren't notarised yet, so the first launch is awkward.
One-liner that downloads, copies to `/Applications` and strips the
quarantine bit:

```sh
curl -fsSL https://pane.thothlab.tech/install-macos.sh | bash
```

…or after dragging the app from the dmg:

```sh
xattr -dr com.apple.quarantine /Applications/Pane.app
```

### Linux

```sh
chmod +x Pane_*_amd64.AppImage && ./Pane_*_amd64.AppImage
```

### Windows

The NSIS installer isn't signed with an EV cert yet, so SmartScreen will
show a warning — click **More info → Run anyway**.

## First device

1. Open Pane. The sidebar shows **Captures**, **Devices**, **Rules**,
   **Settings**.
2. Connect your phone via USB. On Android, enable **USB debugging** under
   Developer options. On iOS, trust the laptop the first time you plug in.
3. Go to **Devices → Add device**. Pane discovers attached phones via
   `adb` / `libimobiledevice` and shows them in a list.
4. Pick your phone → **Install CA + set proxy**. Pane:
   - generates a per-device leaf-cert chain rooted at Pane's local CA,
   - pushes the root CA to the device's trust store,
   - configures the Wi-Fi proxy to point at Pane's local listener
     (`127.0.0.1:8888` by default).
5. Hit **Start proxy** in the sidebar.

The next request your phone's app makes is a capture in the list. Click
a row to see method / URL / status, headers, body and timing.

## Reading captures

The capture list supports a small filter DSL on the search bar:

```text
host:api.example.com          # only requests to this host
status:5..                    # any 5xx
!error:tls_handshake          # exclude pinning + handshake failures
status:200..299 host:*.dev    # ranges + globs
google                        # bareword: substring of host or path
```

Save the current filter with the ☆ icon to pin it to the sidebar.

Right pane shows **Overview / Request / Response / Timing / TLS**. The
body viewer auto-detects JSON / XML / text:

- **Tree** — collapsible nodes, copy by path or by value.
- **Pretty** — formatted, syntax-highlighted text.
- **Raw** — bytes as they came off the wire.

## Next

- [Response stubs](/docs/rules/) — replace or patch responses for testing.
- [Release process](/docs/reference/releases/) — cutting tags, for maintainers.
