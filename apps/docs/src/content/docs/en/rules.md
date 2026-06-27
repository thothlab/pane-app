---
title: Response stubs
description: Replace or patch responses for testing without touching the real server.
---

Pane supports two modes for substituting responses on requests passing through the proxy:

- **Stub** — upstream is not called; a fully prepared response is returned directly.
- **Patch** — the real request is forwarded to the server, the response is intercepted, and a list of patches is applied on top of it.

Patch mode is convenient in scenarios where the client depends on server-generated fields (tokens, timestamps, ids) — they stay real, while you swap only the specific values needed for testing.

## Quick path: from Captures

Right-clicking a row in **Captures** opens an "Add to rules" picker. It lists existing collections, an "Ungrouped" slot, and a "New collection…" option (creates a collection with the default name `From captures` — you can rename it later).

When you pick one, Pane creates a stub rule pre-filled from the captured request:

- `method`, `host_glob`, `path_glob` (the query string is stripped — `match_params` stays empty so the mock matches regardless of query),
- **Request body (JSON)** — if the request had a JSON body, it's pre-filled here in full (pretty-printed); see the section below,
- `res_status`, `res_headers`, `res_body` are taken straight from the captured response.

The Rules tab is also pre-aimed at this new rule's editor — switch tabs and you can tweak name, body, headers and hit Save.

## Matching on the request body (JSON subset)

The **Request body (JSON)** field in the Match section holds a JSON value the engine checks for **containment** in the request body (deep subset). This matches **nested** fields — unlike `match_params`, which only sees top-level scalars of the query/body.

> ⚠️ **Params are exact-match, no operators.** A param value is compared literally (string for string). `=10000` will NOT match a body with `amount: 10000` — the leading `=` is taken as part of the value. For `=`/`>`/`≥`/contains use the **Conditions** block (below) and put a plain value (`10000`) in Params. The editor flags a Params value that starts with an operator.

`template ⊆ actual` semantics:

- **objects** — every key in the template must be present in the body and match recursively; extra fields in the request are allowed;
- **arrays** — positional prefix: the body is at least as long, and `template[i]` is contained in `actual[i]`;
- **scalars** — exact equality;
- `{}` matches any JSON object; an empty field / NULL means "don't match on the body";
- a body that isn't JSON, or doesn't contain the template, **fails the match** (fail-closed).

```jsonc
// Request body (JSON):
{ "hidePaid": true, "period": { "from": "2026-01" } }
// matches any request that has hidePaid:true AND period.from:"2026-01",
// whatever else the body carries
```

> ⚠️ **Volatile fields.** A captured body contains per-request values (`requestId` — a UUID, etc.). The match is exact, so leaving `requestId` in makes the rule silently never fire on future requests. The editor flags UUIDs in the body with a warning — **delete** those fields from the JSON. (Numbers: `0` and `0.0` compare as unequal — normally harmless, since the client serializes them consistently.)

## Conditions on body fields (comparisons)

`match_req_body` only checks **equality** (template subset). To match on a **comparison** — "`amount` between 1000 and 1500", "`status` contains active" — use the **Conditions** block in the Match section.

Each condition is `path · operator · value`:

- **path** — dot/bracket notation into the JSON body: `amount`, `payment.details.sum`, `items[0].price` (same syntax as patch paths; a leading `body.` is optional);
- **operator** — `=`, `≠`, `>`, `≥`, `<`, `≤`, `contains`;
- **value** — the right-hand side.

Conditions are **AND-ed** (with each other and the other matchers), so a **range is two rows**:

```
amount   ≥   1000
amount   ≤   1500
```

Matches a request where `amount` is in `[1000, 1500]`, and won't fire on `999` or `1501`.

**Operator semantics:**

- numeric (`>`, `≥`, `<`, `≤`) — both sides coerce to a number (`f64`). Money arrives as `1200` or as the string `"1200.50"` — both parse. A non-number fails closed (no match);
- `=` / `≠` — numeric compare when both sides look numeric, else exact string equality. Exact float equality is fragile (`0` vs `0.0`) — use `≥`/`≤` for ranges;
- `contains` — **case-insensitive** substring (`ACTIVE_NOW` contains `active`); arrays/objects don't match.

> A field missing at the path, a non-JSON body, or a type that can't satisfy the operator → the condition fails and the rule doesn't match (fail-closed) — consistent with the rest of the matcher.

## Response body editor

The response body lives in a JSON-aware textarea: object keys, strings, numbers, `true`/`false`/`null` are colour-coded inline against the Pane theme. Highlighting is done by a transparent textarea sitting on top of a `<pre>` with a small regex tokenizer — no external editor libraries, no install hit.

Two buttons sit next to the field label:

- **Format** (`{ }`) — runs `JSON.parse → JSON.stringify(..., null, 2)` (2-space indent). Invalid JSON flashes an inline "Invalid JSON: …" message for 2.5 s without touching the content. Empty body shows "Body is empty". Format counts as an edit — Save will turn red.
- **Expand / Collapse** — toggles the textarea height between compact (~7 rem) and tall (70vh, 400 px minimum). State is persisted to `localStorage` — open a big JSON once, every editor afterwards opens already expanded.
- **Load file** — loads a JSON body from a `.json` file in one click (for heavy dumps, instead of copy-paste).
- **Clear** (red) — empties the field contents. Disabled when the field is already empty.

## Unsaved-changes indicator

The **Save** button turns red as soon as the user edits any field (name, host glob, method, status, headers, body, patches, …) and goes back to blue after a successful save. On a save error it stays red so you can see the work hasn't shipped. The flag is local to the current editor instance: rehydrating an in-progress draft from `localStorage` does NOT light Save red, because nothing was typed in this session yet.

## Checkbox toggles

Each rule row has a small checkbox on the left (enabled/disabled, replacing the previous pill switch). The collection header has the same checkbox next to its chevron:

- every rule enabled → check, clicking disables everything;
- every rule disabled → empty box, clicking enables everything;
- mixed → minus (`−`) inside the box, clicking enables everything.

A single click only flips the rules whose current state differs from the target — the rest are left alone.

The **enabled** checkbox inside an open rule editor behaves the same way — for an already-saved rule the toggle commits immediately and instantly syncs the collapsed-row switch and the collection-header tri-state, without waiting for Save.

## Drag to reorder

Rule and collection order is also **match precedence**: the engine takes the first matching rule (by collection priority first, then rule priority). So you can reorder by dragging:

- **A rule within its collection** — drag the rule row and drop it above/below another. A thin accent line shows where it will land.
- **A whole collection** — drag the grip handle (`⠿`) on the left of the collection header and drop it above/below another collection.

Dragging a rule into a **different** collection (dropping onto its area) still moves the rule into that collection, as before.

## Duplicating a rule

Each rule row has a **Copy** button (between Edit and Export). It creates a duplicate next to the original, with the same priority and all fields (matching, body, headers), appending a ` - copy` suffix to the name. The response body is reused by reference (storage dedupes identical bodies) — no re-upload.

## Editor state is preserved

Rules-tab state now survives both tab switches and full app restarts:

- which collections are collapsed,
- which rule is currently being edited,
- a per-editor draft of every field, **including** the response-body textarea.

This covers the common "open rule → switch to Captures → copy something → switch back → paste → save" flow — before this, the editor would be closed on return and you had to re-expand every time. The draft is dropped only on Save, Cancel, or the "collapse without saving" arrow (↑ in the editor header).

## Sharing rules: import and export

A curated set of mocks is worth keeping around — and worth handing to a teammate. Rules can be exported to and imported from `.json` files at three granularities.

**Export**:

- **One rule** — Download icon in the rule row (between Copy and Delete). Saves a `<name>.pane-rule.json` containing that rule plus its response body inline.
- **One collection** — Download icon in the collection header. Saves `<name>.pane-collection.json` with the collection metadata and every rule inside it.
- **Everything** — *Export all* button in the page header. Saves `pane-rules.json` covering every collection (including Ungrouped) and every rule.

Bodies travel inline as base64, so the file is self-contained — no companion blobs.

**Import**:

The *Import* button in the page header opens a file picker. The file's `kind` field decides what gets created: a single rule, a collection plus its rules, or the full library. The action is additive — existing rules and collections aren't touched.

**Conflict policy**: every imported entity gets a fresh UUID. If a collection or rule with the same name already exists, the import lands beside it as a duplicate (you decide whether to rename or delete after). There's no merge, overwrite, or skip prompt — the same name twice is a deliberate non-event.

**Format**: `{ format: "pane-rules", version: 1, kind, exported_at, collections, rules }`. Hand-editing the JSON is supported but unprotected — invalid fields surface as backend errors at import time, not warnings.

## Where to configure

Sidebar → **Rules** → collection → rule in `Patch — forward, then mutate` mode.

A rule is matched against the same criteria as in Stub mode: host glob, method, path glob, query/body parameters. If the rule fires, the engine applies the list of patches after receiving the server's response.

## Path syntax

Path is dot-notation that walks the "virtual response tree":

| Prefix | What it changes |
|---|---|
| `status` | HTTP status of the response |
| `headers.<Name>` | Response header (case-insensitive) |
| `body.<dot.path>` | Field inside the JSON body |
| `<dot.path>` | Also body — the `body.` prefix is optional, so `user.fio` ≡ `body.user.fio` |

Inside body paths:

- `a.b.c` — nested object.
- `a.b[0]` — array element by index.
- `a.b[-]` — append to the end of array (only in `set` / `append` ops).

## Operation kinds

| op | What it does |
|---|---|
| `set` | Sets value at path. Missing parent objects are created. |
| `delete` | Removes an object field or an array element by index. |
| `append` | Appends an element to an array (path points to the array). |

## Value

Parsed as JSON, with a string fallback:

- `qwerty` → string `"qwerty"`
- `777` → number
- `true` / `false` / `null` → bool / null
- `{"a":1}` → object
- `["x","y"]` → array

If you need to substitute a field with a string that looks like a number (`"123"`), write `"123"` with quotes.

---

## Example 1. Patch a single field

Server replies:

```json
{
  "user": { "uid": 2715, "fio": "TG GIS MT", ... },
  "token": { "id": "ed821640d251...", ... }
}
```

Goal: replace the `fio` field with a test value, keep the real `token`.

**Mode**: Patch
**Match**: POST `/api/auth`
**Patches**:

```
op    | path           | value
------+----------------+------------
set   | user.fio       | "Test User"
```

The real token stays valid, subsequent authorised requests continue to work.

---

## Example 2. Replace an array of objects

Server returns a list of ~130 objects:

```json
{ "objects": [ {...}, {...}, ... 130 items ], "_links": {...} }
```

Goal: keep **only one** object in the array (for testing UI pagination, dropdowns, etc.).

### Variant A — single rule (recommended)

Replace the whole array at once. The value is a JSON array with one element.

```
op    | path           | value
------+----------------+----------------------------------------------------------
set   | body.objects   | [{"uid":6,"id":6,"name":"Test object","priority":2}]
```

Note: the path is **`body.objects`** (no `[0]`!) and the value starts with `[` — it's a JSON array, not an object.

### Variant B — two patches in sequence

If "clear + add" is more natural:

```
op       | path           | value
---------+----------------+-------------------------
set      | body.objects   | []
append   | body.objects   | {"uid":6,"id":6,"name":"Test object","priority":2}
```

`set ... = []` clears the array, `append` adds one element. Patches apply in order.

### What NOT to do (common mistake)

```
op    | path              | value
------+-------------------+----------------
set   | body.objects[0]   | {...}             ← replaces only element zero
```

With this form a 130-item array doesn't change — only the first element is overwritten, the remaining 129 stay. The UI still sees the long list.

Rule of thumb: **`[0]` is the index of a specific element, while `body.objects` (no index) is the whole array.**

---

## Example 3. Override status and a header

```
op    | path                       | value
------+----------------------------+-------------
set   | status                     | 401
set   | headers.X-Pane-Stubbed     | "true"
set   | body.error                 | "unauthorized"
```

---

## Example 4. Delete a field and add a new one

```
op       | path                       | value
---------+----------------------------+---------
delete   | body.user.email            |
set      | body.user.role             | "admin"
```

---

## Delay

Both modes expose a **`delay (ms)`** field in the Response section. In Stub mode the delay is applied before sending the response. In Patch mode it kicks in after receiving the real response, before writing back to the client. Useful for simulating a slow server.

---

## What ends up in Captures

- Stub mode → `state='stubbed'`.
- Patch mode → `state='patched'`. The Response tab shows the already-patched body — that's what the client received.

## When a patch silently doesn't fire

- Body is not valid JSON → body patches are skipped (warning in the log), but status/header patches still apply.
- Response `Content-Type` is not json-ish → body isn't parsed, body patches are skipped.
- Path points at a non-existent array index in `delete` / `append` (indices don't make sense in `append` anyway).
- Invalid JSON in value → treated as a string, which sometimes yields an unexpected `"true"` instead of `true`.

If a rule isn't behaving as expected, open the actual capture for that request and double-check the path.
