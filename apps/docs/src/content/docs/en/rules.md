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
- `res_status`, `res_headers`, `res_body` are taken straight from the captured response.

The Rules tab is also pre-aimed at this new rule's editor — switch tabs and you can tweak name, body, headers and hit Save.

## Editor state is preserved

Rules-tab state now survives both tab switches and full app restarts:

- which collections are collapsed,
- which rule is currently being edited,
- a per-editor draft of every field, **including** the response-body textarea.

This covers the common "open rule → switch to Captures → copy something → switch back → paste → save" flow — before this, the editor would be closed on return and you had to re-expand every time. The draft is dropped only on Save, Cancel, or the "collapse without saving" arrow (↑ in the editor header).

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
