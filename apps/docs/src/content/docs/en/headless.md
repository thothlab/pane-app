---
title: Headless Pane
description: "`pane proxy run` — an instance with no window for CI, scripts and agents, plus control.json and how to get out of a data-directory conflict."
---

Pane is one process that owns a data directory and hosts a control socket on it.
Everything else — the [CLI](/docs/en/cli/), the [MCP server](/docs/en/agents/), a
second terminal — attaches to it. How you start it decides exactly one thing:
whether there is a window.

| What you want | Start | Window |
| --- | --- | --- |
| Normal use | the Pane app | yes |
| CI, scripts, agents | `pane proxy run` | no |
| Working on Pane itself | `make tauri-dev` | yes |

## Starting

```console
$ pane proxy run
pane: proxy listening on 127.0.0.1:8888
pane: control socket at /Users/you/Library/Application Support/tech.thothlab.pane/control.sock
pane: ready — Ctrl-C to stop
{"data_dir":"/Users/you/Library/Application Support/tech.thothlab.pane","event":"ready","kind":"headless","proxy":true}
```

The prose goes to stderr and the machine-readable line to stdout, so
`pane proxy run | head -1` in a script is not drowned out by logs.

The process runs in the foreground until Ctrl-C. Flags:

```sh
pane proxy run --port 9999                    # default is 127.0.0.1:8888
pane proxy run --host 0.0.0.0                 # listen beyond loopback
pane proxy run --no-proxy                     # bring the instance up without starting the proxy
pane proxy run --data-dir /tmp/scratch-pane   # separate database, separate lock
```

**The first stdout line is always `ready`.** Block on it in a script rather than
polling for the socket or sleeping a fixed number of seconds.

It hosts the same control socket the desktop app does, so `pane captures tail`,
`pane devices add` and everything else in another terminal behave exactly as they
would with a window open.

## Stopping

Ctrl-C and `kill` (SIGTERM) do the same thing: shut the proxy down and **clear
the proxy settings from paired devices**. Skipping that teardown strands a phone
pointing at a dead `127.0.0.1:8888` — that is, with no internet.

:::caution
`kill -9` skips it. If an instance was killed hard, pair the device again after
the next start — or clear it by hand with
`adb shell settings delete global http_proxy`.
:::

`pane proxy stop` is a different thing: it stops the **proxy inside** a running
instance without exiting the instance. With nothing running it exits 3.

## Who owns the directory right now

On startup an instance writes `control.json` (mode 0600) next to the database:

```console
$ cat ~/Library/Application\ Support/tech.thothlab.pane/control.json
{
  "protocol": 1,
  "pid": 14055,
  "app_version": "0.2.12",
  "kind": "headless",
  "endpoint": "/Users/you/Library/Application Support/tech.thothlab.pane/control.sock",
  "data_dir": "/Users/you/Library/Application Support/tech.thothlab.pane",
  "started_at": "2026-08-11 14:07:48.148151 +00:00:00"
}
```

- `kind` — `gui` (the desktop app) or `headless` (`pane proxy run`);
- `pid` — the process to look at when something is stuck;
- `data_dir` — which database this process owns.

The quick "is anything running at all" check is `attached_to_instance` in
`pane doctor`.

## Directory conflicts

Exactly one process owns a directory at a time. A second one gets **exit 8** and
a message naming the directory — that is the guard working, not a bug:

```console
$ pane proxy run
pane: another Pane instance already owns /Users/you/Library/Application Support/tech.thothlab.pane — use it instead of starting a second one
$ echo $?
8
```

Find the culprit:

```sh
pgrep -fl "Pane.app/Contents/MacOS/pane"   # the GUI
pgrep -fl "pane proxy run"                 # a headless run
```

Then either quit it, or send the new instance somewhere else:

```sh
pane proxy run --data-dir /tmp/scratch-pane
```

The opposite symptom — "everything exits 3 but Pane is plainly open" — usually
means the app is running against a **different** data directory: compare
`data_dir` in `control.json` against what `pane doctor --data-dir …` reports.

## Scratch directories

`--data-dir` (or `PANE_DATA_DIR`) is the supported way to experiment alongside
your real Pane: own database, own lock, no conflict.

```sh
export PANE_DATA_DIR=/tmp/scratch-pane
pane proxy run --port 9999 &
# … run the scenario …
pane captures count --filter 'status:500..599'
```

Such a directory starts with no rules; load a mock set into it from a file with
`pane rules import fixtures/all-rules.json` (see [the CLI](/docs/en/cli/#rules)).

## A CI step

```yaml
- name: Bring Pane up and run the scenario
  run: |
    set -euo pipefail
    export PANE_FORMAT=json
    export PANE_DATA_DIR="$RUNNER_TEMP/pane"

    pane proxy run --port 8888 &
    PANE_PID=$!
    trap 'kill -TERM $PANE_PID' EXIT          # SIGTERM, not -9: teardown matters

    until pane doctor | jq -e '.attached_to_instance' >/dev/null; do sleep 0.2; done

    pane rules import fixtures/all-rules.json
    pane collections only orders-error
    pane captures clear --yes

    ./gradlew connectedAndroidTest

    test "$(pane captures count --filter 'state:stubbed rule:orders-500')" -ge 1
    test "$(pane captures count --filter 'host:api.example.com state:completed')" -eq 0
```

There is no Windows build of the CLI (see
[the caveat](/docs/en/cli/#install)), so a headless step belongs on macOS and
Linux runners.

## Next

- [The `pane` CLI](/docs/en/cli/) — every command, filter and exit code.
- [Agents and MCP](/docs/en/agents/) — the same operations for an LLM agent, plus
  the "prove the mock answered" pattern.
