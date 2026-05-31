-- Schema v1: sessions, CA, devices, captures, headers, bodies, replay, filters,
-- pinning incidents. Mirrors PRD-01 §3.

CREATE TABLE IF NOT EXISTS ca_certificate (
    id          TEXT PRIMARY KEY,
    serial      TEXT NOT NULL,
    sha256_fp   TEXT NOT NULL,
    subject     TEXT NOT NULL,
    valid_from  INTEGER NOT NULL,
    valid_to    INTEGER NOT NULL,
    pem         TEXT NOT NULL,
    revoked_at  INTEGER
);

CREATE TABLE IF NOT EXISTS session (
    id          TEXT PRIMARY KEY,
    started_at  INTEGER NOT NULL,
    stopped_at  INTEGER,
    listen      TEXT NOT NULL,
    ca_id       TEXT NOT NULL REFERENCES ca_certificate(id),
    status      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS device (
    id                  TEXT PRIMARY KEY,
    session_id          TEXT REFERENCES session(id),
    platform            TEXT NOT NULL,
    connection          TEXT NOT NULL,
    serial              TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    state               TEXT NOT NULL,
    ca_installed_at     INTEGER,
    capabilities_json   TEXT,
    last_error          TEXT,
    created_at          INTEGER NOT NULL,
    UNIQUE (platform, serial)
);

CREATE TABLE IF NOT EXISTS capture_body (
    id           TEXT PRIMARY KEY,
    sha256       TEXT NOT NULL,
    encoding     TEXT NOT NULL,
    mime         TEXT,
    size_bytes   INTEGER NOT NULL,
    storage      TEXT NOT NULL,
    inline_blob  BLOB,
    file_path    TEXT,
    created_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_body_sha ON capture_body(sha256);

CREATE TABLE IF NOT EXISTS capture (
    id            TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL REFERENCES session(id),
    started_at    INTEGER NOT NULL,
    ended_at      INTEGER,
    client_addr   TEXT NOT NULL,
    server_host   TEXT NOT NULL,
    server_port   INTEGER NOT NULL,
    scheme        TEXT NOT NULL,
    http_version  TEXT NOT NULL,
    method        TEXT NOT NULL,
    url_path      TEXT NOT NULL,
    status        INTEGER,
    req_body_id   TEXT REFERENCES capture_body(id),
    res_body_id   TEXT REFERENCES capture_body(id),
    tls_info_id   TEXT,
    total_bytes   INTEGER NOT NULL DEFAULT 0,
    duration_ms   INTEGER,
    state         TEXT NOT NULL,
    error_kind    TEXT,
    is_replay     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_capture_session_started ON capture(session_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_capture_host ON capture(server_host);
CREATE INDEX IF NOT EXISTS idx_capture_status ON capture(status);
CREATE INDEX IF NOT EXISTS idx_capture_method ON capture(method);

CREATE TABLE IF NOT EXISTS header (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    capture_id  TEXT NOT NULL REFERENCES capture(id) ON DELETE CASCADE,
    direction   TEXT NOT NULL,
    name        TEXT NOT NULL,
    value       TEXT NOT NULL,
    order_idx   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_header_capture ON header(capture_id);

CREATE TABLE IF NOT EXISTS tls_info (
    id                  TEXT PRIMARY KEY,
    capture_id          TEXT NOT NULL REFERENCES capture(id) ON DELETE CASCADE,
    sni                 TEXT,
    alpn                TEXT,
    cipher              TEXT,
    version             TEXT,
    cert_chain_fps      TEXT,
    pinning_detected    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS replay_record (
    id                  TEXT PRIMARY KEY,
    source_capture_id   TEXT REFERENCES capture(id),
    result_capture_id   TEXT REFERENCES capture(id),
    created_at          INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS saved_filter (
    id      TEXT PRIMARY KEY,
    name    TEXT NOT NULL UNIQUE,
    query   TEXT NOT NULL,
    color   TEXT NOT NULL,
    pinned  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS pinning_incident (
    id           TEXT PRIMARY KEY,
    capture_id   TEXT REFERENCES capture(id) ON DELETE CASCADE,
    host         TEXT NOT NULL,
    alpn         TEXT,
    hint_kind    TEXT NOT NULL,
    occurred_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pinning_host ON pinning_incident(host);
