---
title: Agents and MCP
description: "`pane mcp` — Pane as tools for an LLM agent: wiring it up, the tool list, and how to prove a response came from a mock."
---

`pane mcp` runs an MCP server over stdio that exposes Pane's operations as tools.
An agent — Claude Code, Cursor, your own SDK script — can then see what actually
went over the wire, stand up a mock, run a scenario and check the result, without
touching the GUI or the app's source.

The same operations are available straight from the terminal — see
[the `pane` CLI](/docs/en/cli/). What MCP adds is schema-described tools, so the
agent is not parsing text output.

## Wiring it up

The server is one command, `pane mcp`, with no flags. Any MCP client is
configured the same way:

```json
{
  "mcpServers": {
    "pane": {
      "command": "pane",
      "args": ["mcp"]
    }
  }
}
```

Claude Code does it in one line:

```sh
claude mcp add pane -- pane mcp
```

For a different data directory — a scratch one for CI, say — use
`"args": ["mcp", "--data-dir", "/tmp/scratch-pane"]`, or set `PANE_DATA_DIR` in
the client's environment.

The server works both with the desktop app open (it attaches, and edits show up
in the window immediately) and without it — see
[Headless Pane](/docs/en/headless/).

## The tools

| Tool | What it does |
| --- | --- |
| `pane_doctor` | proxy state, devices, adb, CA fingerprint. Run this first |
| `pane_proxy_status` | is the proxy running, on what address, with how many captures |
| `pane_captures_list` | recent captures with `state` and `matched_rule_name`, filterable |
| `pane_captures_count` | how many captures match a filter — the cheapest assertion |
| `pane_capture_get` | one capture with request and response headers |
| `pane_capture_body` | request or response body, truncated to `max_bytes` (8192 default) |
| `pane_captures_wait` | block until `count` captures match, or `timeout_sec` elapses |
| `pane_captures_clear` | delete every capture — use between scenarios |
| `pane_rules_list` | all rules with their enabled state, matchers and response status |
| `pane_rule_mock` | create a stub rule |
| `pane_rule_set_enabled` | enable/disable a rule by name substring or id |
| `pane_rules_set_enabled_bulk` | in bulk: the whole library, one collection, or everything ungrouped |
| `pane_collections_list` | collections and how many rules each holds |
| `pane_collection_set_enabled` | enable/disable every rule in a collection |
| `pane_collection_only` | switch to exactly one scenario |
| `pane_collection_delete` | delete a collection (its rules move to Ungrouped) |
| `pane_devices_list` | paired devices and whether the CA is installed |
| `pane_devices_attached` | what is plugged in over USB right now |
| `pane_device_add` | pair a device; requires a running proxy |
| `pane_logcat_query` | Android log lines, filtered with `tag: msg: level: pid: app:` |

The tools are shaped for the task rather than mapped 1:1 onto CLI commands. The
main difference is `pane_captures_wait`: MCP cannot stream, so `captures tail`
becomes a blocking call that returns whatever it saw within the deadline.

## Context hygiene

Response bodies and long capture lists burn context faster than anything else, so
work down the ladder instead of reaching straight for a body:

1. `pane_captures_count` with a filter — one number, and you already know whether
   there is anything to discuss;
2. `pane_captures_list` with a narrow filter and a small `limit`;
3. `pane_capture_get` for a specific id;
4. `pane_capture_body` — and always with `max_bytes`.

From the terminal, large bodies have a better route:
`pane captures body <id> --res --out FILE` writes the whole thing to disk and
costs no context at all.

## Prove the mock actually served the request

`state:stubbed` says *a* mock answered. With a large rule library that is much
weaker than knowing *which* one: a run that matched the wrong rule looks just as
green. Assert both sides.

```sh
pane captures count --filter 'state:stubbed rule:orders-500'          # must be ≥ 1
pane captures count --filter 'host:api.example.com state:completed'   # must be 0
```

The first says "my rule answered". The second says "nothing reached the live
backend". Only together do they mean what you want.

With scenarios split across devices, `device:` belongs on both sides — otherwise
the other phone's traffic answers for this one:

```sh
pane captures count --filter 'device:pixel state:stubbed rule:checkout-ok'  # >= 1
pane captures count --filter 'device:pixel rule:orders-500'                 # 0
```

Which is also why scenarios switch wholesale:

```sh
pane rules disable --all
pane rules enable --collection orders-error
# or, shorter:
pane collections only orders-error
```

Leaving the previous collection enabled is the classic way to get a green run
that proves nothing: two rules match the same host and path, the higher-priority
one answers, and `state:stubbed` still matches.

:::tip
When a scenario mixes stub and patch rules, assert `state:stubbed,patched` —
checking only `stubbed` fails on a response a patch rule produced. And
`state:tunneled` means the client refused our CA and the connection was spliced
through undecrypted: that is a certificate-trust problem, not a rule-matching
one.
:::

## A scripted run

`pane captures tail` emits NDJSON, and **the first line is always `ready`** —
read it, then trigger the app. Skipping that races the first request, which needs
a `sleep` to paper over, and that is where flaky runs come from.

```bash
#!/usr/bin/env bash
set -euo pipefail
export PANE_FORMAT=json

pane collections only orders-error
pane captures clear --yes

exec 3< <(pane captures tail --filter 'host:api.example.com' --count 1 --timeout 45)
read -r -u 3 _ready                      # blocks until {"event":"ready",…}

maestro test flows/orders_error.yaml     # now trigger the app

read -r -u 3 hit || true
printf '%s' "$hit" | jq -r 'select(.event=="capture") | "\(.status) \(.matched_rule_name)"'

test "$(pane captures count --filter 'state:stubbed rule:orders-500')" -ge 1
test "$(pane captures count --filter 'host:api.example.com state:completed')" -eq 0
```

The exit code carries the verdict: **7 means `--count` was not reached before
`--timeout`** — a failed assertion, not a broken tool. The last line of the
stream is always `{"event":"end","reason":"count"|"timeout"|"signal"}`.

## Skills in the repository

`.claude/skills/` holds three ready-made skills. Copy them into your own project,
or read them as a template for instructing any agent:

| Skill | About |
| --- | --- |
| `pane-debug` | watching live traffic: the drill-down ladder, tail, correlating with logcat |
| `pane-mock` | mocks and assertions: collections, `only`, proving the mock answered |
| `pane-run` | starting and stopping an instance, getting out of a directory conflict |

Next to them, in the repository root, `AGENTS.md` is a one-page cheat sheet for an
agent entering the project for the first time.

When any documentation disagrees with reality, there is one source of truth:

```sh
pane schema      # the command tree, both filter grammars and exit codes, as JSON
```

## Next

- [The `pane` CLI](/docs/en/cli/) — every command and flag.
- [Headless Pane](/docs/en/headless/) — an instance for CI and agents.
- [Response stubs](/docs/en/rules/) — what the rules an agent toggles can actually do.
