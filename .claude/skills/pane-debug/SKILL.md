---
name: pane-debug
description: Inspect live HTTPS/MITM traffic from the Pane desktop debugger via the `pane` CLI — start the proxy, pair an Android device or emulator, find failing or slow requests, read request/response bodies without flooding context, and correlate them with logcat. Use when asked to debug a network call, see what an app or emulator is sending, find why an API request 500s / times out / returns the wrong payload, inspect headers or JSON bodies, export a request as curl or HAR, or watch traffic live while a UI scenario runs.
---

# pane-debug

Drive the Pane network debugger from the terminal instead of its GUI.

## Preflight (once per session)

```bash
command -v pane || echo "pane CLI not installed"
```

If it is absent, **say so and stop** — do not guess at another tool.

```bash
export PANE_FORMAT=json     # every command emits machine-readable JSON
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

`attached_to_instance` tells you which mode you are in:

- **`true`** — the desktop app (or `pane proxy run`) is up and every command goes
  to it. Required for `captures tail`, `proxy stop` and `devices add`.
- **`false`** — nothing is running, so the data directory is opened directly.
  Reads and rule edits still work; the commands above do not.

Fix what `doctor` reports before anything else:

```bash
pane proxy start                       # exit 3 from any command = proxy not running
pane devices attached                  # exit 4 = no device
pane devices add emulator-5554         # pair a device seen by adb; needs a running proxy
```

## No GUI? Run a headless instance

Without the desktop app there is nothing to attach to, so `tail` and device
pairing fail with exit 3. Start one yourself — it hosts the same control socket,
so every command behaves identically:

```bash
pane proxy run --port 8888 &           # foreground process; background it yourself
# first stdout line is {"event":"ready","kind":"headless",...} — wait for it
pane doctor                            # attached_to_instance is now true
```

It shuts down on Ctrl-C or SIGTERM, clearing device proxy settings on the way
out. Use `--data-dir` (or `PANE_DATA_DIR`) to keep a scratch run away from your
real captures.

> **Use the DTO's field names.** A capture row has `url_path`, `server_host`,
> `duration_ms`, `total_bytes`, `state` and `matched_rule_name` — there is no
> `path`, `duration`, `size` or `rule` key. `pane schema` is authoritative.

## The drill-down ladder — never skip a rung

Each rung costs ~10x the context of the one above. Start at the top, narrow with
`--filter`, and only descend when the current rung cannot answer the question.

| Rung | Command | Cost |
|---|---|---|
| 1 | `pane captures count --filter '…'` | one integer |
| 2 | `pane captures list --filter '…' --limit 20 --fields id,status,method,host,path` | ~20 lines |
| 3 | `pane captures get <id>` | one full record + headers |
| 4 | `pane captures body <id> --res --max-bytes 2048` | bytes |

```bash
pane captures count --filter 'host:api.example.com status:500..599'
```
```
7
```
```bash
pane captures list --filter 'host:api.example.com status:500..599' --limit 5 \
  --fields id,status,method,path,duration
```
```json
[{"id":"c97cc13d-7134-4074-b90b-15bbb76b7bf8","status":500,"method":"POST",
  "url_path":"/v2/orders","duration_ms":412,"server_host":"api.example.com",
  "state":"completed","matched_rule_name":null},
 {"id":"4cb21e58-16ba-4f48-b342-439c7aac9b6e","status":503,"method":"GET",
  "url_path":"/v2/orders/8821","duration_ms":30011,"server_host":"api.example.com",
  "state":"completed","matched_rule_name":null}]
```
```bash
pane captures get c97cc13d
```
```json
{"id":"c97cc13d","state":"completed","method":"POST","host":"api.example.com",
 "url_path":"/v2/orders","status":500,"total_bytes":218,"duration_ms":412,
 "device_id":"e41b…","matched_rule_name":null,
 "req_headers":{"content-type":"application/json","authorization":"Bearer ey…"},
 "res_headers":{"content-type":"application/json","x-request-id":"7f1c2a"}}
```
```bash
pane captures body c97cc13d --res --max-bytes 2048
```
```json
{"error":"internal","trace":"7f1c2a","message":"order service unavailable"}
```

**Large payloads never go through context.** Write them to a file and query the file
(`--req` reads the request body instead; default is `--res`):

```bash
pane captures body c97cc13d --res --out /tmp/resp.json
jq '.items | length' /tmp/resp.json
```

## Watch traffic live

`captures tail` streams NDJSON. **The first line is always `{"event":"ready",…}` —
read it before triggering the app**, or you will miss the request you are hunting.
The last line is `{"event":"end","reason":…}`.

```bash
pane captures tail --filter 'host:api.example.com !status:200' --count 3 --timeout 60s
```
```
{"event":"ready","filter":"host:api.example.com !status:200","timeout":"60s"}
{"event":"capture","id":"c97cc13d-7134-4074-b90b-15bbb76b7bf8","method":"POST","url_path":"/v2/orders","status":500,"state":"completed","matched_rule_name":null}
{"event":"capture","id":"4cb21e58-16ba-4f48-b342-439c7aac9b6e","method":"GET","url_path":"/v2/cart","status":401,"state":"completed","matched_rule_name":null}
{"event":"end","reason":"timeout"}
```

Exit 7 = the `--count` was not reached before `--timeout`.

## Correlate with logcat

Take the timestamp/ids from a capture, then look at what the app logged:

```bash
pane logcat query --serial emulator-5554 --filter 'level:E,W app:com.example.app' --limit 20
pane logcat query --serial emulator-5554 --filter '~7f1c2a'     # regex on the x-request-id
```
```json
[{"ts":"2026-08-05T11:20:14.881Z","level":"E","tag":"OrderRepo","pid":8123,
  "msg":"POST /v2/orders failed http=500 trace=7f1c2a"}]
```

`pane logcat tail --serial emulator-5554 --filter '…'` follows it live; `pane logcat clear --yes` resets the buffer before a run.

## Hand off / reproduce

```bash
pane captures export c97cc13d --format curl        # paste-able reproduction
pane captures export c97cc13d --format har
pane replay c97cc13d --header 'Authorization: Bearer NEW' --method POST
pane captures clear --yes                          # clean slate before a run
```

## Two things that will bite you

**Captures recorded before the `matched_rule_name` column exists show no rule.**
Migration V012 added it, and pre-existing rows are NULL — so a database with
history will report `state:stubbed` on old captures while `rule:<name>` matches
nothing:

```bash
pane captures count --filter 'state:stubbed'            # 638
pane captures count --filter 'state:stubbed rule:orders'   # 0
```

That is correct, not a bug. Only traffic captured after the upgrade carries the
name. When auditing an old run, assert on `state:` alone; for a fresh run,
assert on `rule:`.

**The CLI and the Pane app must be the same build.** The CLI refuses to touch a
data directory whose schema does not match:

```
pane: opening the Pane data directory
  → this database is at schema v11 and this build expects v12. Migrating it
    would stop the installed Pane app from launching, so it is left alone.
```

That refusal is deliberate, not a bug: migrating is one-way, and an app that
meets an unknown migration version aborts on launch. Either update the app to
match the CLI, or point the CLI at a scratch directory with `--data-dir` /
`PANE_DATA_DIR`. Never work around it by deleting the database.

**A Pane desktop build older than the control endpoint cannot be attached to.**
`pane doctor` then reports `none running` even with the app open, and `tail`,
`proxy stop` and `devices add` fail with exit 3. Reads and rule edits still work
against the database directly. Start your own instance instead of rebuilding the
app:

```bash
pane proxy run --port 8888 &
pane doctor        # attached_to_instance: true
```

## Captures filter DSL

Identical string to the GUI search bar. Keys: `host: path: method: status: mime:
size: duration: error: device: state: rule:`.

| Form | Meaning |
|---|---|
| `orders` | bare word — substring of host OR path |
| `status:500,503` | OR within one key |
| `status:400..499` | range (also `size:`, `duration:`) |
| `!status:200` | negate |
| `path:"/v2/my orders"` | quoted value keeps spaces |
| `state:completed\|stubbed\|patched\|error` | how the response was produced |
| `rule:orders-500` | which mock rule served it (name substring or id) |

Terms are ANDed: `host:api.example.com state:error duration:5000..`.

`state:` and `rule:` are how you tell a real backend response from a mocked one —
see the **pane-mock** skill for asserting on them.

## Exit codes

`0` ok · `2` usage · `3` proxy not running · `4` no device · `5` not found ·
`6` bad filter · `7` timeout/assertion failed · `8` conflict.
Errors print JSON on stderr. Add `--yes` to destructive commands (`clear`, `rm`).
