---
name: pane-run
description: Start, stop and check a Pane instance — the desktop app, a headless `pane proxy run`, or a dev build from the repo — and get out of the "another instance already owns this directory" conflict. Use when asked how to launch or open Pane, when a `pane` CLI command exits 3 (no running instance), when the GUI will not start or is already running, when two instances fight over the data directory, or when a freshly built app or CLI needs installing.
---

# Running Pane

Pane is one process that owns a data directory and hosts a control socket.
Everything else — the `pane` CLI, the MCP server, a second terminal — attaches
to it. Which way you start it decides only whether there is a window.

## Which one to start

| You want | Start | Window |
|---|---|---|
| Normal use | `open -a Pane` | yes |
| Agents, CI, scripts | `pane proxy run` | no |
| Working on the code | `make tauri-dev` from the repo | yes |
| UI work with no backend | `pnpm dev` | browser |

Only one of these can own a data directory at a time. Starting a second gives
exit code 8 and a message naming the directory; that is the guard working, not
a bug. See *Conflicts* below.

## Check first

```bash
export PANE_FORMAT=json
pane doctor
```

`attached_to_instance` is the answer to "is Pane running":

- **`true`** — an instance is up. Everything works.
- **`false`** — no instance. The CLI reads the data directory directly, so
  queries and rule edits still work, but `captures tail`, `proxy stop` and
  `devices add` exit 3.

To see *which* instance, read the discovery file it writes:

```bash
cat ~/Library/Application\ Support/tech.thothlab.pane/control.json
```

`kind` is `gui` or `headless`; `pid` is the process to look at if something is
stuck.

## Desktop app

```bash
open -a Pane                       # or: open /Applications/Pane.app
```

There is no single-instance plugin — macOS just activates the running app
instead of launching a second process. Force one (`open -n`, or running the
binary directly) and it exits 1 with "Pane is already running", because two
GUIs would both try to bind 8888 and write the same SQLite file.

That message goes to stderr, which is invisible when launched from Finder: an
app that "does not open" and leaves no window is almost always this.

## Headless

```bash
pane proxy run                     # 127.0.0.1:8888, Ctrl-C to stop
pane proxy run --port 9999
pane proxy run --data-dir /tmp/scratch-pane      # keep off your real captures
```

The first stdout line is `{"event":"ready",…}` — block on that in a script
rather than polling for the socket. It handles SIGTERM as well as Ctrl-C, and
clears device proxy settings on the way out. Skipping that teardown is what
strands a paired phone pointing at a dead 127.0.0.1:8888, so let it exit
cleanly rather than `kill -9`.

`--data-dir` (or `PANE_DATA_DIR`) is the way to run a throwaway instance
alongside your real one: separate directory, separate lock, no conflict.

## From the repo

```bash
make tauri-dev          # Vite + the Rust shell, hot reload on the frontend
make dev                # frontend only, in a browser
```

`make dev` has no backend behind it — every IPC call fails. It is for CSS and
layout work, not for anything that touches captures. (`pane serve`, which puts a
real backend behind a browser, is in progress on `feat/pane-serve`.)

## Installing what you built

```bash
make tauri-build
rm -rf /Applications/Pane.app
cp -R target/release/bundle/macos/Pane.app /Applications/
```

`make tauri-build` exits non-zero at the very end on a missing
`TAURI_SIGNING_PRIVATE_KEY` — that is the updater signature, which only CI has.
Both bundles are already written by then; check for
`target/release/bundle/macos/Pane.app` rather than trusting the exit code.

The CLI is a separate binary and does **not** come with the app:

```bash
cargo build --release -p pane-cli
./target/release/pane-cli install      # symlinks onto PATH as `pane`
pane --version
```

Keep the two in step. The CLI refuses to open a data directory whose schema is
newer or older than it expects, rather than migrating it — a one-way migration
from a newer CLI would stop the installed app from launching at all. If
`pane doctor` reports a schema mismatch, rebuild whichever of the two is behind.

## Conflicts

**"another Pane instance already owns …"** (exit 8) — something already holds
the lock. Find it:

```bash
pgrep -fl "Pane.app/Contents/MacOS/pane"   # the GUI
pgrep -fl "pane proxy run"                 # a headless run
```

Quit that one, or point the new one somewhere else with `--data-dir`.

**Everything exits 3 but you are sure Pane is open** — the lock is held by a
process that died without cleaning up, or the app is running against a different
data directory. `control.json` names both the pid and the directory; compare it
against `pane doctor --data-dir …`.

**The GUI will not start at all** — it fails hard rather than running degraded
when it cannot take the lock or open the database, and from Finder there is
nowhere for it to say so. The log says why:

```bash
tail -50 ~/Library/Application\ Support/tech.thothlab.pane/pane.log*
```

## Stopping

- GUI: ⌘Q. Quitting reverts the system proxy and clears device settings.
- Headless: Ctrl-C, or `kill` (SIGTERM) — both run the same teardown.
- `pane proxy stop` stops the *proxy* inside a running instance without exiting
  it. Requires an instance, so it exits 3 when nothing is running.
