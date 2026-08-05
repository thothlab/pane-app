---
name: pane-debug
description: Inspect live HTTPS/MITM traffic from the Pane desktop debugger via the `pane` CLI — start the proxy, pair an Android device or emulator, find failing or slow requests, read request/response bodies without flooding context, and correlate them with logcat. Use when asked to debug a network call, see what an app or emulator is sending, find why an API request 500s / times out / returns the wrong payload, inspect headers or JSON bodies, export a request as curl or HAR, or watch traffic live while a UI scenario runs.
---

# Inspecting traffic with Pane

Pane is a local HTTPS/MITM proxy for debugging your own apps. Drive it with the
CLI — never the GUI.

## Preflight

```bash
command -v pane || echo "pane CLI not installed"   # if absent, say so and stop
export PANE_FORMAT=json                            # once per session
pane doctor
```

```json
{"attached_to_instance":false,
 "proxy":{"running":false,"listen":null,"captures_count":0},
 "android_tooling":{"ok":true,"adb_path":"/Users/you/Library/Android/sdk/platform-tools/adb"},
 "devices_paired":[],
 "devices_attached":[{"platform":"android","serial":"emulator-5554",
                      "name":"Google sdk_gphone64_arm64 · Android 14 · r-5554"}],
 "ca":{"sha256_fp":"4394bdce96a126c4…","valid_to":"2029-08-04T12:53:15Z"}}
```

`attached_to_instance` decides what you can do:

- **`true`** — a Pane instance is running; everything works.
- **`false`** — the data directory is read directly. Queries and rule edits
  work; `captures tail`, `proxy stop` and `devices add` fail with exit 3.

No GUI? Start your own instance — it hosts the same control socket, so every
command behaves identically:

```bash
pane proxy run --port 8888 &     # first stdout line is {"event":"ready",...}
pane doctor                      # attached_to_instance is now true
```

It stops on Ctrl-C or SIGTERM, clearing device proxy settings on the way out.
Use `--data-dir` / `PANE_DATA_DIR` to keep a scratch run off your real captures.

Then pair a device:

```bash
pane devices attached            # exit 4 = nothing plugged in
pane devices add emulator-5554   # needs a running proxy
```

## The drill-down ladder — never skip a rung

Each rung costs roughly ten times the previous one. Start narrow.

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
$ pane captures get c97cc13d          # full row plus request/response headers
```
```console
$ pane captures body c97cc13d --res --max-bytes 4096 | jq .error
{"code":"ORDER_INDEX_TIMEOUT","message":"upstream did not respond within 1000ms"}
```

Bodies truncate to 8 KiB by default; the note goes to stderr so stdout stays a
clean pipe. For anything large use `--out FILE` — it writes the whole body and
costs no context:

```bash
pane captures body c97cc13d --res --out /tmp/resp.json && jq '.items | length' /tmp/resp.json
```

`--fields` narrows further: `pane captures list --fields id,status,url_path`.

**Use the DTO's field names.** A capture row has `url_path`, `server_host`,
`duration_ms`, `total_bytes`, `state` and `matched_rule_name` — there is no
`path`, `duration`, `size` or `rule` key. `pane schema` is authoritative.

## Watch traffic live

```console
$ pane captures tail --filter 'host:api.example.com' --count 1 --timeout 30
{"event":"ready","filter":"host:api.example.com","count":1,"timeout":30}
{"event":"capture","id":"c97cc13d-7134-4074-b90b-15bbb76b7bf8","method":"POST",
 "url_path":"/v2/orders","status":500,"state":"completed","duration_ms":412,
 "server_host":"api.example.com","matched_rule_name":null}
{"event":"end","reason":"count","captures":1,"elapsed_ms":1014}
```

**The first line is always `ready`.** Read it, *then* trigger the app — otherwise
you race the first request and end up adding a `sleep`, which is where flaky
scripts come from. `--count` is the assertion, `--timeout` the deadline; falling
short exits **7**.

## Correlate with logcat

```bash
pane logcat attach --serial emulator-5554
pane logcat query --serial emulator-5554 --filter 'app:dev.shop.app level:E' --limit 20
```

`app:` is resolved to PIDs by the CLI. Filter keys: `tag: msg: level: pid: app:`,
plus `~regex`; a bare word matches tag or message. `level` takes `V D I W E F S`
and ranges like `W..F`.

## Hand off a request

```bash
pane captures export c97cc13d --format curl        # reproducible one-liner
pane captures export c97cc13d --format har --out req.har
pane replay c97cc13d --header 'X-Debug: 1'         # re-send, optionally modified
```

## Rule collections

Mocks are grouped into collections, one per scenario. `pane collections ls` shows
them; `pane collections only <name>` switches scenario in a single call. See the
**pane-mock** skill for creating rules and for the assertion pattern that proves
which one served a response.

## Two things that will bite you

**The CLI and the Pane app must be the same build.** The CLI refuses a data
directory whose schema differs:

```
pane: opening the Pane data directory
  → this database is at schema v11 and this build expects v12. Migrating it
    would stop the installed Pane app from launching, so it is left alone.
```

Deliberate: migrating is one-way, and an app meeting an unknown migration
version aborts on launch. Update the app, or point the CLI at a scratch
directory. **Never delete the database to get past it.**

**A Pane build predating the control endpoint cannot be attached to.** `doctor`
says `none running` with the app plainly open. Use `pane proxy run &`.

## Reference

Captures filter DSL — the same string the GUI search bar takes:

| key | matches |
|---|---|
| `host:` `path:` | substring, `*` glob |
| `method:` | exact, uppercased |
| `status:` `size:` `duration:` | `N` or `N..M` |
| `mime:` | response Content-Type substring |
| `error:` | error kind |
| `device:` | device name or serial; `__host__` = this Mac |
| `state:` | `completed` \| `stubbed` \| `patched` \| `error` |
| `rule:` | rule that served a mocked response |

Bare word = host OR path · `!term` negates · `a,b` = OR within a key ·
`"quoted"` keeps spaces.

Selectors accept a full id, a unique id prefix, or a name substring; ambiguity
is an error listing candidates. Destructive commands need `--yes`. Errors are
JSON on stderr; stdout stays clean for `| jq`.

Exit codes: 0 ok · 2 usage · 3 no running instance · 4 no device · 5 not found ·
6 bad filter · 7 timeout / assertion failed · 8 conflict.

`pane schema` emits the whole command tree, both filter grammars and the exit
codes as JSON — authoritative when this file and the CLI disagree.
