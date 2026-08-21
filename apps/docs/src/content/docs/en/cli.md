---
title: The pane CLI
description: Drive Pane from the terminal — captures, rules, collections, devices and logcat without touching the GUI.
---

`pane` is a separate binary that does everything the window does: read captures,
create and toggle rules, pair devices, pull logcat. It exists for the places a
GUI cannot go — scripts, CI, automated UI tests, and sessions where an agent is
at the keyboard (see [Agents and MCP](/docs/en/agents/)).

If the desktop app is running, commands go to it over its local control socket
and the open window reflects them immediately. If it is not, the CLI opens the
same data directory directly.

## Install

The CLI is **not** part of the app bundle — it ships as its own file. Grab it
from a release:

| Platform | Release asset |
| --- | --- |
| macOS Apple Silicon | `pane-cli-darwin-aarch64` |
| Linux x86_64 | `pane-cli-linux-x86_64` |

```sh
curl -fsSL -o pane https://github.com/thothlab/pane-app/releases/latest/download/pane-cli-darwin-aarch64
chmod +x pane && mv pane /usr/local/bin/
pane --version
```

Or build it from source, which guarantees it matches your app:

```sh
make cli-install          # = cargo build --release -p pane-cli + install
# or by hand:
cargo build --release -p pane-cli
./target/release/pane-cli install       # symlinks onto PATH as `pane`
```

`install` links into `/usr/local/bin`, falling back to `~/.local/bin` when that
is not writable. Somewhere else: `install --dir <DIR>`.

:::caution[Windows]
There is no Windows build of the CLI. It talks to the app over a Unix socket,
which has no Windows implementation, so every command there would report "no
running instance". The Windows build of the desktop app itself is complete.
:::

:::note[Keep the two in step]
The CLI refuses to open a data directory whose schema is newer or older than it
expects, rather than migrating it:

```text
pane: opening the Pane data directory
  → this database is at schema v11 and this build expects v12. Migrating it
    would stop the installed Pane app from launching, so it is left alone.
```

Migration is one-way, and an app that meets an unknown migration version will
not launch at all. Update whichever half is behind — or point the CLI at a
scratch directory with `--data-dir`. **Never delete the database to get past
it**: that is your whole capture history and rule library.
:::

## The two commands everything starts with

```console
$ pane doctor
instance   attached to a running Pane
proxy      running   127.0.0.1:8888      38 captures
adb        /Users/you/Library/Android/sdk/platform-tools/adb
ca         7c4db809d727457e…
devices    2 paired · 1 attached
```

One call answers: is there an instance? is the proxy up? was adb found? is the
CA valid? are devices paired? Every "why is nothing working" starts here.

```sh
pane schema      # the whole command tree, both filter grammars and exit codes, as JSON
```

`schema` is the machine-readable version of this page, and it is the more
authoritative one: it is generated from the CLI itself, so when it and the docs
disagree, it wins.

## Output format

Human-readable columns by default. For scripts:

```sh
export PANE_FORMAT=json     # once per session
pane --json doctor          # or per call
```

The rule is simple: **stdout is a clean pipe for `| jq`**, and everything else —
warnings, explanations, errors — goes to stderr. `captures body` writes raw body
bytes to stdout and its truncation note to stderr, so redirecting to a file
never corrupts it.

## Captures: the drill-down ladder

Each rung costs roughly ten times the previous one. Start at the top, especially
when context is tight.

```console
$ pane captures count --filter 'host:api.example.com status:500..599'
2
```
```console
$ pane captures list --filter 'status:500..599' --limit 5
SID       METHOD  STATUS HOST              PATH              MS  BYTES  RULE
c97cc13d  POST    500    api.example.com   /v2/orders       412    218
4cb21e58  GET     503    api.example.com   /v2/orders/8821 30011    244
```
```console
$ pane captures get c97cc13d              # the full row plus request/response headers
$ pane captures body c97cc13d --res --max-bytes 4096 | jq .error
```

- `--filter` takes **the same string** as the GUI search bar — see
  [Filtering captures](/docs/en/filtering/).
- `--fields id,status,url_path` keeps only what you need; `--full` emits the
  whole DTO instead of the summary projection.
- Bodies truncate to 8 KiB. For large payloads use `--out FILE` (writes
  everything, prints nothing), not `--max-bytes 0`.
- `captures body` defaults to the response; `--req` for the request body,
  `--base64` for binary.

Hand a request to someone else, or reproduce it outside Pane:

```sh
pane captures export c97cc13d --format curl          # a one-liner
pane captures export c97cc13d --format har --out req.har
```

`captures clear --yes` wipes everything — useful between scenarios so an
assertion cannot match a previous run.

### Watching live

```console
$ pane captures tail --filter 'host:api.example.com' --count 1 --timeout 30
{"event":"ready","filter":"host:api.example.com","count":1,"timeout":30}
{"event":"capture","id":"c97cc13d-…","method":"POST","url_path":"/v2/orders",
 "status":500,"state":"completed","duration_ms":412,"matched_rule_name":null}
{"event":"end","reason":"count","captures":1,"elapsed_ms":1014}
```

NDJSON, where **the first line is always `ready`** and the last is always `end`.
Wait for `ready`, *then* trigger the app: otherwise you race the first request,
which people usually paper over with a `sleep` — and that is where flaky scripts
come from. `--count` is the assertion, `--timeout` (whole seconds, no suffix) the deadline;
falling short is [exit 7](#exit-codes).

`tail` is one of the few commands that need a **running instance** — without one
it exits 3. So do `proxy stop`, `devices add`, `devices rm`, `logcat attach` and
`logcat detach`; everything else — reading captures, editing rules, collections —
works with the app closed.

## Rules

```sh
pane rules ls
pane rules mock --host api.example.com --path '/v2/orders*' --method POST \
  --status 500 --body '{"error":"internal"}' --name orders-500
pane rules from-capture c97cc13d --status 500 --name orders-500
```

`mock` creates a stub rule in one line (defaults: `--status 200`,
`--mime application/json`); `from-capture` reuses the host, path and method of a
real capture. Large bodies come from `--body-file fixtures/x.json`. `--disabled`
creates it switched off.

**Always pass `--name`.** It is the selector for `enable | disable | rm` and the
value for `rule:` in filters. Without it the name defaults to host+path, which is
awkward to assert on.

Toggling, one at a time or in bulk:

```sh
pane rules enable orders-500          # by name substring or id
pane rules disable --all              # reset to a known state
pane rules enable --collection base   # every rule in one collection
pane rules enable --ungrouped         # everything outside a collection
```

The bulk flags are not sugar: sweeping a real library one rule at a time is one
process launch per rule, and each one re-lists everything to resolve its
selector.

Sharing a set of mocks uses the same `pane-rules` format the GUI reads and
writes, bodies inline:

```sh
pane rules export --out fixtures/all-rules.json
pane rules import fixtures/all-rules.json --dry-run    # look first
pane rules import fixtures/all-rules.json
```

Import always creates new entities; it never overwrites by name.

## Collections

A collection is one scenario. Switching scenario wholesale:

```console
$ pane collections ls
ID        STATE    RULES  NAME
b2d5f2a0  off          8  base — 500 ₽ baseline
a647e8c1  off          8  noamount — QR with no amount

$ pane collections only noamount
pane: only `noamount — QR with no amount` is live — 8 rule(s) enabled, everything else off
```

`only` means "disable everything, then enable this collection". That is how to
move between scenarios: leaving the previous collection enabled means two rules
match the same host and path, the higher-priority one answers, and an assertion
on `state:stubbed` still goes green while proving nothing.

`collections rm <sel> --yes` deletes a collection and moves its rules to
Ungrouped; `--with-rules` deletes them too.

:::caution[The STATE column is historical]
`enable | disable | only` tick **the rules themselves** — a rule fires on its own
`enabled` flag (and the devices that flag covers, below), and the engine has no
second switch at collection level. The `STATE` column in `collections ls` shows
an old database field that nothing acts on; `pane rules ls` is where the real
state is.
:::

## Different scenarios on different devices

By default a rule covers every device, so `only` switches the scenario on all of
them at once — right on one phone, wrong on four. `--device <sel>` narrows the
change to a single device; the others keep running what they were running.

```console
$ pane devices ls
ID        STATE  PLATFORM  NAME
a91f3c02  ready  android   Google Pixel 7 · Android 14 · r-0XXH
77b1e4de  ready  android   Google sdk_gphone64 · Android 14 · r-5554

$ pane rules disable --all --device pixel
pane: disabled 14 of 21 rules for a91f3c02
pane: 14 rule(s) are now pinned to named devices — a device paired from here on
      will not get them (undo with `pane rules enable --all`)
$ pane collections only checkout --device pixel
$ pane collections only errors --device 77b1e4de
```

The rule library stays shared: the same rule is simply on for one device and off
for another, so there is no need to duplicate it per phone. The flag is
understood by `rules enable | disable | mock` and
`collections enable | disable | only`.

`pane rules ls --device <sel>` shows the list as that device sees it: a `LIVE`
column instead of `STATE`, plus a `SCOPE` column — `all` for shared rules, `N
dev` for pinned ones. Rows are never hidden: "why didn't my mock fire" has to be
answerable from one command, not from guessing what got filtered out.

The device selector is a name substring, serial or id — the same string
`--filter 'device:…'` takes. `__host__` is this Mac's own traffic.

:::caution[Pinning is one-way]
Switching a rule off for one device writes out the devices it stays on for, so a
device paired **afterwards** will not get it. The command says so when it
happens; `pane rules enable --all` without `--device` returns the whole library
to global.

iOS is outside this: its traffic goes through the shared proxy port and is not
attributed per device, so `--device <ios>` returns an error rather than silently
doing nothing. Such devices only ever see rules enabled for all devices.
:::

Verify with `device:` on both sides too, or the other phone's traffic answers for
this one:

```console
$ pane captures count --filter 'device:pixel state:stubbed rule:checkout-ok'
1
$ pane captures count --filter 'device:pixel rule:orders-500'
0
```

## Devices, logcat, CA, proxy

```sh
pane devices attached                  # what is plugged in right now (exit 4 = nothing)
pane devices add emulator-5554         # pair; needs a running proxy
pane devices rm <sel> --yes

pane logcat attach --serial emulator-5554
pane logcat query --serial emulator-5554 --filter 'app:dev.shop.app level:E' --limit 20
pane logcat pids --serial emulator-5554
pane logcat clear --serial emulator-5554 --yes

pane ca show                           # fingerprint and validity
pane ca export --format pem --out ca.pem   # also: der, qr, mobileconfig

pane proxy status
pane proxy start --port 8888
pane proxy stop                        # also reverts proxy settings on devices
```

The logcat filter grammar is `tag: msg: level: pid: app:` plus `~regex`; a bare
word matches tag or message, and `level` takes `V D I W E F S` and ranges like
`W..F`. `app:` is resolved to PIDs by the CLI. More in
[the Logcat window](/docs/en/logcat/).

## Selectors

Anywhere a command wants an `<id>` or `<sel>`, it accepts a full id, a **unique
id prefix**, or a **name substring** of a rule, collection or device. Ambiguity
is an error listing the candidates — the CLI never picks for you.

Nothing prompts. Destructive commands need `--yes`.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | ok |
| 1 | error |
| 2 | usage (argument parsing) |
| 3 | no running instance / proxy stopped |
| 4 | no device, or adb missing |
| 5 | not found |
| 6 | bad filter |
| 7 | timeout, or `--count` not reached — **a failed assertion** |
| 8 | conflict (port in use, precondition failed) |

7 is the interesting one: not a tool failure but a verdict, which makes it easy
to build `set -e` scripts on.

## The data directory

```sh
pane doctor --data-dir /tmp/scratch-pane
export PANE_DATA_DIR=/tmp/scratch-pane
```

The default is the platform location: `~/Library/Application Support/tech.thothlab.pane`
on macOS, `~/.local/share/pane` on Linux. A separate directory means a separate
database, a separate lock and a separate instance — which is exactly how to run
experiments without touching your real capture history.

## Next

- [Headless Pane](/docs/en/headless/) — `pane proxy run` for CI and scripts.
- [Agents and MCP](/docs/en/agents/) — the same operations as tools for an LLM agent.
- [Filtering captures](/docs/en/filtering/) — the `--filter` grammar.
