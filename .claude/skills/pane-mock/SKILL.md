---
name: pane-mock
description: Stub and verify backend responses with the Pane debugger's `pane` CLI — create mock rules, switch whole scenarios by toggling rule collections, import a rule bundle, and prove which rule actually served a response (`state:stubbed rule:<name>`). Use when running automated UI scenarios (Maestro, Espresso, emulator flows) that need a fixed backend, when asked to mock/stub/fake an API response, force a 500 or empty list, simulate an error or offline case, enable or disable a mock rule or scenario, or assert that a request was served by a mock instead of the live backend.
---

# Mocking and verifying responses with Pane

Pane is a local HTTPS/MITM proxy. Rules replace matching responses; collections
group the rules for one scenario. Everything here is CLI — never drive the GUI.

## Preflight

```bash
command -v pane || echo "pane CLI not installed"   # if absent, say so and stop
export PANE_FORMAT=json                            # once per session
pane doctor
```

`doctor` reports `attached_to_instance`. `true` means a Pane instance is running
and everything works. `false` means reads and rule edits still work, but
`captures tail` and `devices add` do not — start one with `pane proxy run &`.

## Switch scenarios by collection, not by rule

A collection is one scenario. Toggling it switches all its rules at once.

```console
$ pane collections ls
ID        STATE    RULES  NAME
fed42761  on           7  base — база 500 ₽
4c12117f  on           7  noamount — QR без суммы
8390cbd8  on           7  allchips — вся лента чипсин
```

```console
$ pane collections only noamount
pane: enabled 0, disabled 2 — only `noamount — QR без суммы` is live
```

**Use `only` between scenarios.** Leaving the previous collection enabled is the
classic way to get a green run that proves nothing: two collections matching the
same host and path means the lower-priority one answers, and the assertion still
sees `state:stubbed`. `only` makes that impossible.

Single rules toggle the same way — `pane rules enable|disable <sel>` — but reach
for the collection first.

## Prove the mock actually served the request

`state:stubbed` says *a* mock answered. With a large library that is much weaker
than knowing *which* one: a run that matched the wrong rule still looks green.
Assert both sides.

```console
$ pane captures count --filter 'state:stubbed rule:orders-500'
1
$ pane captures count --filter 'host:api.example.com state:completed'
0
```

The first must be ≥ 1 (your rule answered). The second must be 0 (nothing
reached the live backend). Only together do they mean what you want.

```console
$ pane captures list --filter 'state:stubbed rule:orders-500'
SID       METHOD  STATUS HOST              PATH          MS  BYTES  RULE
c97cc13d  POST    500    api.example.com   /v2/orders     1     27  orders-500
```

Use `state:stubbed,patched` when a scenario mixes stub and patch rules —
asserting only `stubbed` fails on a response a patch rule produced.

## Scripted assertion loop

`captures tail` emits NDJSON. The **first line is always `ready`** — read it,
then trigger the app. Skipping that races the first request and needs a `sleep`
to paper over, which is where flakiness comes from.

```bash
#!/usr/bin/env bash
set -euo pipefail
export PANE_FORMAT=json

pane collections only orders-error
pane captures clear --yes

exec 3< <(pane captures tail --filter 'host:api.example.com' --count 1 --timeout 45)
read -r -u 3 _ready                      # blocks until {"event":"ready",...}

maestro test flows/orders_error.yaml     # now trigger the app

read -r -u 3 hit || true                 # the capture line, or EOF on timeout
printf '%s' "$hit" | jq -r 'select(.event=="capture") | "\(.status) \(.matched_rule_name)"'

test "$(pane captures count --filter 'state:stubbed rule:orders-500')" -ge 1
test "$(pane captures count --filter 'host:api.example.com state:completed')" -eq 0
```

Exit codes carry the verdict: **7 means `--count` was not reached before
`--timeout`** — a failed assertion, not an error. The last line is always
`{"event":"end","reason":"count"|"timeout"|"signal"}`.

## Creating rules

```console
$ pane rules mock --host api.example.com --path '/v2/orders*' --method POST \
    --status 500 --body '{"error":"internal"}' --name orders-500
```
```json
{"id":"38bd7c6d-6d46-496c-8a66-a336b7677744","name":"orders-500","enabled":true,
 "mode":"stub","match_host_glob":"api.example.com","match_method":"POST",
 "match_path_glob":"/v2/orders*","res_status":500,"res_body_size":20,"res_delay_ms":0}
```

**Always pass `--name`** — it is the selector for `enable|disable|rm` and the
value for `rule:` in filters. Without it the name defaults to host+path, which
is awkward to assert on.

Large bodies come from a file. `from-capture` derives a rule from real traffic,
reusing the capture's host, path and method:

```bash
pane rules mock --host api.example.com --path /v2/orders --status 200 \
  --body-file fixtures/orders_empty.json --name orders-empty
pane rules from-capture c97cc13d --status 500 --name orders-500
```

## Bundles

Export and import use the same `pane-rules` format the GUI reads, bodies inline,
so a bundle round-trips between them.

```bash
pane rules export --out fixtures/all-rules.json
pane rules import fixtures/all-rules.json --dry-run    # inspect first
pane rules import fixtures/all-rules.json
```

Import always creates new entities; it never overwrites by name.

## Three things that will bite you

**The CLI and the Pane app must be the same build.** The CLI refuses a data
directory whose schema differs:

```
pane: opening the Pane data directory
  → this database is at schema v11 and this build expects v12. Migrating it
    would stop the installed Pane app from launching, so it is left alone.
```

Deliberate: migrating is one-way, and an app meeting an unknown migration
version aborts on launch. Update the app, or use `--data-dir` / `PANE_DATA_DIR`
for a scratch directory. **Never delete the database to get past it** — that
discards the whole capture history and rule library.

**Captures recorded before `matched_rule_name` existed report no rule.** On a
database with history, `state:stubbed` matches while `rule:<name>` matches
nothing. Correct, but it looks like a broken filter. For old runs assert on
`state:` alone; for new runs assert on `rule:`.

**A Pane build predating the control endpoint cannot be attached to.** `doctor`
says `none running` with the app plainly open, and `tail` fails with exit 3.
Run your own instance instead of rebuilding the app: `pane proxy run &`.

## Reference

Selectors: full id, unique id prefix, or name substring. Ambiguity is an error
listing candidates — it never picks for you. Destructive commands need `--yes`.

Filter keys: `host: path: method: status: mime: size: duration: error: device:
state: rule:` · bare word = host OR path · `!` negates · `a,b` = OR · `N..M` =
range. `state` is `completed|stubbed|patched|tunneled|error`; `rule` matches the
rule that served a mocked response. `tunneled` means the client refused our CA
and the connection was spliced through undecrypted — a scenario that expected
`stubbed` but got `tunneled` has a CA-trust problem, not a rule-matching one.

Exit codes: 0 ok · 2 usage · 3 no running instance · 4 no device · 5 not found ·
6 bad filter · 7 timeout / assertion failed · 8 conflict.

`pane schema` emits the whole command tree, both filter grammars and the exit
codes as JSON — authoritative when this file and the CLI disagree.
