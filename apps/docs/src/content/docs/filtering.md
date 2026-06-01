---
title: Filtering captures
description: The small DSL on the capture search bar — keys, ranges, globs, negation, barewords.
---

The capture list's search bar accepts a small filter DSL. It evaluates
left to right, all space-separated terms must match (AND).

## Keys

| Key | Matches | Example |
| --- | --- | --- |
| `host:` | `server_host` substring (SQL `LIKE`, `*` works) | `host:api.example.com`, `host:*.dev` |
| `method:` | exact method, case-insensitive | `method:POST` |
| `status:` | single status or range | `status:200`, `status:500..599`, `status:5..` |
| `mime:` | response `Content-Type` substring | `mime:json`, `mime:image/` |
| `path:` | URL path substring (`*` works) | `path:/v1/*`, `path:auth` |
| `size:` | response total bytes, single or range | `size:0`, `size:1000..` |
| `duration:` | request duration in ms | `duration:1000..` (slow), `duration:..50` (fast) |
| `error:` | exact `error_kind` value | `error:tls_handshake`, `error:pinning` |

Keys are **case-insensitive** — `host:`, `Host:` and `HOST:` all
resolve to the same clause. Useful when iOS autocapitalises the first
letter.

## Negation

Prefix any term with `!` to exclude matches:

```text
!error:tls_handshake          # drop everything that failed TLS
!host:*.cdn.example.com       # ignore CDN noise
!path:/healthz                # hide health-check pings
```

## Barewords

A term without a colon is treated as a substring search across **host
or path** simultaneously:

```text
google                        # any capture touching google.com or /google
docs                          # matches host:docs.example.com OR path:/docs
```

Quote phrases that contain spaces or special characters: `"some phrase"`.

## Saving filters

The ☆ button on the right of the search bar saves the current filter to
the sidebar. Pinned filters live above non-pinned and survive restarts.

## Syntax highlighting

Tokens light up as you type:

| Colour | Meaning |
| --- | --- |
| accent (blue) | Known key (`host`, `method`, …) |
| red, dotted underline | Unknown key — backend will reject this term |
| red | The `!` negation prefix |
| muted | The `:` separator |
| default | Values and barewords |

Unknown keys are flagged immediately, before sending to the backend.

## What's not (yet)

- No `OR` between terms. Workaround: save two filters and switch.
- No regex (deliberate — DSL is for skimming, not full grep).
- Nothing matches inside the body. Use the body viewer's Tree mode to
  navigate captured bodies.
