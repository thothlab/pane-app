---
name: pane-mock
description: Stub and verify backend responses with the Pane debugger's `pane` CLI — create mock rules, toggle them on/off per scenario, import a rule bundle, and prove which rule actually served a response (`state:stubbed rule:<name>`). Use when running automated UI scenarios (Maestro, Espresso, emulator flows) that need a fixed backend, when asked to mock/stub/fake an API response, force a 500 or empty list, simulate an error or offline case, enable or disable a mock rule, or assert that a request was served by a mock instead of the live backend.
---

# pane-mock

Stub responses for automated runs — and **prove** which stub answered.

## Preflight (once per session)

```bash
command -v pane || echo "pane CLI not installed"     # if absent: say so, do not improvise
export PANE_FORMAT=json
pane doctor && pane proxy start
```

## The five commands that matter

```bash
pane rules import scenarios/orders.json            # load a bundle
pane rules enable orders-500                       # arm one rule
pane rules disable orders-500                      # disarm it
pane captures count --filter 'state:stubbed rule:orders-500'   # PROOF it fired
pane captures list  --filter 'host:api.example.com state:completed'  # LEAK check
```

## Verify the mock — the pattern that matters

A run where "a mock answered" is not a passing run. `state:stubbed` alone is green
even when the **wrong** rule matched, and a rule whose host/path glob missed
silently lets the request through to the live backend. Assert on **both** sides:

```bash
# 1. the intended rule served the request
pane captures count --filter 'state:stubbed rule:orders-500'
```
```
1
```
```bash
# 2. nothing on that host reached the real backend
pane captures count --filter 'host:api.example.com state:completed'
```
```
0
```
```bash
# 3. eyeball the correlation when a count disagrees
pane captures list --filter 'host:api.example.com' --fields id,path,status,state,rule
```
```json
[{"id":"c97cc13d-7134-4074-b90b-15bbb76b7bf8","url_path":"/v2/orders","status":500,
  "state":"stubbed","matched_rule_name":"orders-500","server_host":"api.example.com"},
 {"id":"4cb21e58-16ba-4f48-b342-439c7aac9b6e","url_path":"/v2/cart","status":200,
  "state":"completed","matched_rule_name":null,"server_host":"api.example.com"}]
```

`c97cc13d` is a leak: no rule covered `/v2/cart`, so the live backend answered.

`state:` values: `completed` (real backend) · `stubbed` (rule produced the whole
response) · `patched` (rule modified a real response) · `error`. If a scenario mixes
both mock kinds, assert `state:stubbed,patched`.

## Scripted assertion loop

`captures tail` blocks until N matching captures arrive. **The first NDJSON line is
always `{"event":"ready",…}` — read it, then trigger the app**, otherwise the request
can land before the stream is listening and the assertion hangs to timeout.

```bash
cat > /tmp/verify.sh <<'EOF'
set -u
export PANE_FORMAT=json
OUT=$(mktemp)

pane captures tail --filter 'state:stubbed rule:orders-500' --count 1 --timeout 30s > "$OUT" &
TAIL=$!
until grep -q '"event":"ready"' "$OUT" 2>/dev/null; do sleep 0.1; done   # do NOT trigger before this

maestro test flows/orders_error.yaml                                    # trigger the app

wait "$TAIL"; RC=$?
[ "$RC" -eq 7 ] && { echo "FAIL: rule orders-500 never served a response"; cat "$OUT"; exit 1; }
[ "$RC" -ne 0 ] && { echo "FAIL: pane exit $RC"; exit 1; }
grep '"event":"capture"' "$OUT" | jq -r '"OK \(.id) <- \(.rule) [\(.status)]"'
EOF
bash /tmp/verify.sh
```
```
{"event":"ready","filter":"state:stubbed rule:orders-500","timeout":"30s"}
OK c97cc13d <- orders-500 [500]
```

Exit **7** = the `--count` was not met inside `--timeout`, i.e. the assertion failed.
Run the script in the background if your harness blocks foreground `sleep`.

> **Field names are the DTO's, not shorthand.** A capture row has `url_path`,
> `server_host`, `state` and `matched_rule_name` — there is no `path` or `rule`
> key. A rule has `match_host_glob` / `match_path_glob` / `res_status`. Run
> `pane schema` if in doubt.

## Creating rules

```bash
pane rules mock --host api.example.com --path '/v2/orders*' --method POST \
  --status 500 --body '{"error":"internal"}' --name orders-500
```
```json
{"id":"38bd7c6d-6d46-496c-8a66-a336b7677744","name":"orders-500","enabled":true,
 "mode":"stub","match_host_glob":"api.example.com","match_method":"POST",
 "match_path_glob":"/v2/orders*","res_status":500,"res_body_size":20,"res_delay_ms":0}
```

**Always pass `--name`.** It is the `<sel>` for `enable|disable|get|rm` and the value
for `rule:` in capture filters, so a name you chose is far easier to assert on than
the auto-generated `host+path` default. Ids are UUIDs; any unique prefix works as a
selector too. Big bodies come from a file; `from-capture`
derives a rule from real traffic, keeping its host/path/method:

```bash
pane rules mock --host api.example.com --path '/v2/orders' --status 200 \
  --body-file fixtures/orders_empty.json
pane rules from-capture c97cc13d --status 500 --body '{"error":"internal"}'
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

## Bundles and per-scenario toggling

```bash
pane rules export --out scenarios/all-rules.json     # snapshot current 374 rules
pane rules import scenarios/all-rules.json           # restore on another machine/CI
pane rules ls --filter 'orders' --limit 20
```
```json
[{"id":"38bd7c6d-6d46-496c-8a66-a336b7677744","name":"orders-500","enabled":false,
  "match_path_glob":"/v2/orders*","res_status":500},
 {"id":"7471aea8-2c19-4f8a-9d33-b0f1c2e4a5d7","name":"orders-empty","enabled":false,
  "match_path_glob":"/v2/orders*","res_status":200}]
```

Two rules on one glob means order decides the winner — exactly what `rule:`
disambiguates. Arm exactly one per scenario:

```bash
for s in orders-500 orders-empty; do pane rules disable "$s"; done   # reset
pane captures clear --yes                                           # clean evidence
pane rules enable orders-500                                        # confirm: rules get
# … tail → {"event":"ready"} → run flow → wait (0 = served, 7 = FAIL) …
pane captures count --filter 'host:api.example.com state:completed'  # leak check, must be 0
pane rules disable orders-500
```

Repeat per scenario. When step 3 exits 7 or the leak check is non-zero,
`pane captures list --filter 'host:<h>' --fields id,path,state,rule` tells you whether
the wrong rule matched or nothing matched at all.

## Reference

Rules: `ls | get <sel> | enable <sel> | disable <sel> | rm <sel> --yes | mock … |
from-capture <id> … | import <file.json> | export [--out F]`. `<sel>` = id or name substring.

Filter keys: `host: path: method: status: mime: size: duration: error: device: state:
rule:` — bare word = host OR path substring, `a,b` = OR, `N..M` = range, `!` negates,
quotes keep spaces, terms AND together.

Exit codes: `0` ok · `2` usage · `3` proxy not running · `4` no device · `5` not found ·
`6` bad filter · `7` timeout/assertion failed · `8` conflict. Errors are JSON on stderr.

For reading bodies, live tailing and logcat correlation, see the **pane-debug** skill.
