# Pane — agent guide

Pane is a local HTTPS/MITM debugger. Drive it with the `pane` CLI; never the GUI.

    export PANE_FORMAT=json        # once per session

Not installed? `cargo build --release -p pane-cli && ./target/release/pane-cli install`

## Start here

    pane doctor        # proxy? devices? adb? CA?
    pane schema        # full command tree, both filter grammars, exit codes, as JSON

## The commands that matter

    pane captures list --filter 'host:api.example.com status:500..599' --limit 20
    pane captures get <id>
    pane captures body <id> --res --max-bytes 4096      # raw bytes on stdout
    pane captures tail --count 1 --timeout 30s --filter '…'   # NDJSON
    pane rules mock --host api.example.com --status 500 --body '{"e":1}' --name x
    pane rules enable <sel> | disable <sel>             # by name substring or id
    pane rules disable --all                            # reset to a known state
    pane rules enable --collection <sel>                # one scenario's rules
    pane collections ls | only <sel>                    # switch a whole scenario at once
    pane collections rm <sel> --yes                     # rules survive, move to Ungrouped

## Proving a response came from a mock

`state` alone says *a* mock answered. With a large rule library that is much weaker
than knowing *which* one — a run that matched the wrong rule still looks green. Assert
both sides:

    pane captures count --filter 'state:stubbed rule:orders-500'   # must be >= 1
    pane captures count --filter 'host:api.example.com state:completed'  # must be 0

## Rules

- A rule fires on its own `enabled` flag alone. There is no second switch on the
  collection — a collection is grouping and ordering only, so a ticked rule is
  always live and there is never a second place to look.
- `collections enable|disable|only <sel>` therefore tick and untick the rules
  themselves. `only` = disable everything, then enable that collection.
- The reliable way into a known state is `pane rules disable --all` followed by
  `pane rules enable --collection <sel>`, rather than trusting whatever the last
  run left behind.
- Bodies truncate to 8 KiB. For large payloads use `--out FILE`, not `--max-bytes 0`.
- Every `<id>`/`<sel>` takes a unique prefix, or a rule/device name substring.
  Ambiguity is an error listing candidates — it never picks for you.
- Nothing prompts. Destructive commands need `--yes`.
- Errors are JSON on stderr; stdout stays clean for `| jq`.
- `pane captures tail` prints `{"event":"ready",…}` first. Read that line, *then*
  trigger the app under test — otherwise you race the first request.
  The count is the assertion, the timeout is the deadline.
- Works with the desktop app open (shares its state) or closed. `pane proxy run`
  starts a headless instance; `tail` needs one either way.

## Exit codes

    0 ok · 2 usage · 3 no running instance · 4 no device · 5 not found
    6 bad filter · 7 timeout / --count not reached (assertion failed) · 8 conflict

## Captures filter DSL — the same string the GUI search bar takes

    host: path: method: status: mime: size: duration: error: device: state: rule:
    bare word = substring over host OR path   ·   !term negates
    a,b = OR within one key   ·   N..M = range   ·   "quoted" keeps spaces
    state: completed | stubbed | patched | tunneled | error
    tunneled = client refused our CA, spliced through undecrypted — no body
    rule:  the rule that served a mocked response — name substring or exact id

## Logcat filter DSL

    tag: msg: level: pid: app:   ·   ~regex   ·   bare word = tag OR message
    level takes V D I W E F S and ranges like W..F
