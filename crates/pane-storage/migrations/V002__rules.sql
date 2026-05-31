-- Schema v2: response stubbing rules.
--
-- A rule fires when an incoming request matches the matcher fields. Instead of
-- proxying upstream, the engine writes the prepared response straight back to
-- the client and persists the capture as `state='stubbed'`.

CREATE TABLE IF NOT EXISTS rule (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,
    priority        INTEGER NOT NULL DEFAULT 0,

    -- Matcher (NULL = wildcard for that dimension)
    match_host_glob TEXT,
    match_method    TEXT,
    match_path_glob TEXT,
    -- JSON array of {name, value} pairs that must all be present in the
    -- request query string (subset match; extras allowed).
    match_query     TEXT,

    -- Response
    res_status      INTEGER NOT NULL,
    -- JSON array of {name, value}. Hop-by-hop and length headers are stripped
    -- and recomputed by the engine before writing.
    res_headers     TEXT NOT NULL,
    -- Body reuses the existing capture_body store (so "stub from this capture"
    -- can reference the same row without duplicating bytes).
    res_body_id     TEXT REFERENCES capture_body(id),
    res_delay_ms    INTEGER NOT NULL DEFAULT 0,

    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rule_enabled_priority ON rule(enabled, priority);
