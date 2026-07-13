---
title: Logcat window
description: Device log window — filter DSL (tag/level/pid/app/regex), search, tail, export, SQLite-backed retention.
---

A dedicated **Logcat** window per device shows the live `adb logcat` stream.
Lines are written to SQLite (roughly the last 24h per device), so history
survives reopening the window, and filtering and pagination run off an index
rather than memory.

Open it from a device card (**Devices** tab) — the **Logcat** button. Each
device opens in its own window.

## Toolbar

| Control | What it does |
| --- | --- |
| **Tail** | Follow mode: the view sticks to the end of the stream. Scrolling up turns Tail off; scrolling back to the bottom turns it on. |
| **Pause** | Freezes the view (the backend keeps writing to the DB). While frozen, a “+N” badge counts new matching lines. |
| **Clear** | Deletes this device’s rows from the DB. |
| **Export** | Writes the **entire** filtered history (not just the loaded window) to a `.log` file in `threadtime` format — opens in any log viewer. |

## Filter

The row with the **▽** icon is the filter DSL (focus with `⌘⇧F`).
Space-separated terms are AND-ed together; the same grammar compiles to the
SQL applied to the DB.

### Keys

| Key | Matches | Example |
| --- | --- | --- |
| `tag:` | substring on tag (case-insensitive, Unicode) | `tag:OkHttp`, `tag:*Worker` |
| `msg:` / `message:` | substring on the message text | `msg:timeout`, `message:"connection reset"` |
| `level:` | a level or a range (see below) | `level:E`, `level:W..F` |
| `pid:` | exact PID | `pid:1234` |
| `app:` | the app’s process by package name (see below) | `app:com.example.app`, `app:example` |

Keys are **case-insensitive**. Quote values with spaces or special
characters: `tag:"Pane Helper"`.

### Levels

`level:` takes a single level or a range. Tokens: `v`/`verbose`,
`d`/`debug`, `i`/`info`, `w`/`warn`, `e`/`error`, `f`/`fatal`, `s`/`silent`.

```text
level:E          # error only
level:W..F       # warn and above (warn, error, fatal)
level:..E        # error and below
level:W..        # warn and above (no upper bound)
```

### App filter — `app:`

`app:` matches a process by **substring** of its package name. Names come
from a periodic `ps -A` snapshot (~every 10s), so `app:example` catches
`com.example.app`. The value is resolved to PIDs and rows are filtered by
those — the log itself carries only a PID, not a process name.

```text
app:com.example.app                      # only this app’s process
app:com.example.app,!com.example.helper  # the app, minus its helper process
```

Important behavior (since 0.2.6): if `app:X` **matches no live process**
(the app isn’t running, or hasn’t landed in the `ps` snapshot yet), the view
is **empty** — it does not fall back to the whole device log. An empty result
here means “no lines from this app right now”, not “the filter didn’t apply”.

:::note
`app:` attribution is **live-PID only**. Lines from a process that already
exited (or logged before the window opened) won’t match `app:X` — their PID
isn’t in the snapshot. For that case, use `pid:` directly.
:::

### Negation

A `!` before a term excludes matches; a `!` before a value inside a list
excludes that value.

```text
!tag:LeakCanary            # drop all LeakCanary
tag:!Spam,!Noise           # tag with neither Spam nor Noise
level:E !msg:keepalive     # errors, but not about keepalive
```

### Comma means OR

Within one key, a comma lists alternatives (positives OR together, negatives
mean “none of”):

```text
tag:OkHttp,Retrofit        # tag contains OkHttp OR Retrofit
pid:1234,5678              # either PID
```

### Barewords and regex

A term with no colon is a substring across **tag or message**. A term
prefixed with `~` is a regular expression (Rust syntax) matching **tag or
message**:

```text
timeout                    # tag OR message contains "timeout"
~^Worker.*failed$          # regex over tag or message
!~keepalive                # exclude everything the regex matches
```

Unlike the captures filter, this one **has regex** — logs are often skimmed
by exact patterns.

## Search (`⌘F`)

Separate from the filter, there’s a substring search (`⌘F`). It does **not**
narrow the table — it highlights matches and jumps between them (`Enter`
next, `Ctrl+Enter` previous) across all columns. The filter (`⌘⇧F`) shapes
the stream; search (`⌘F`) finds within what’s already filtered.

## Filter-row icons

- **×** — clear the whole filter.
- **☆** — save the current filter to the sidebar (left list). Saved filters
  survive restarts; saving under an existing name **updates** it. The
  captures and logcat lists are separate.

## Columns

Time · App · PID · Tag · Level · Message. Time/PID/App/Tag widths drag from
the header’s right edge (double-click to reset). Right-click the header to
show/hide columns. Choices persist across restarts. The **App** column is the
process name for a PID (from the same `ps -A` snapshot); blank when the PID
doesn’t resolve.

## Selection and copy

Click selects a row, `Shift`-click a range, `⌘`/`Ctrl`-click toggles one.
`⌘C` copies the selection as full `threadtime` lines, `⇧⌘C` messages only.
Right-click → **View message** opens an overlay with the full message and a
JSON-format button.

## Syntax highlighting

Filter tokens are highlighted as you type: known keys in accent (blue), an
unknown key in red with a dotted underline (the backend drops the term), `!`
in red, `:` muted.
