-- Durable logcat storage. Replaces the frontend's in-memory ring buffer so
-- filtered views (e.g. tag:AnalyticsEvent) survive the unfiltered firehose
-- instead of being evicted by unrelated noise. One row per parsed logcat line,
-- tagged by device serial. Retention is enforced in code (rolling 24h by
-- created_at + a per-device row cap), not by the schema.
CREATE TABLE IF NOT EXISTS logcat_entry (
    id          INTEGER PRIMARY KEY,   -- rowid; monotonic ingest cursor (NOT AUTOINCREMENT)
    serial      TEXT    NOT NULL,      -- raw adb serial → per-device separation
    created_at  INTEGER NOT NULL,      -- ingest unix-millis → retention + tiebreak
    device_ts   TEXT    NOT NULL,      -- raw "MM-DD HH:MM:SS.mmm" (display only; no year)
    pid         INTEGER NOT NULL,
    tid         INTEGER NOT NULL,
    level       INTEGER NOT NULL,      -- LogLevel discriminant 0..6 → cheap range filter
    tag         TEXT    NOT NULL,
    message     TEXT    NOT NULL
);
-- Hot path: WHERE serial=? [AND filter] ORDER BY id DESC LIMIT ? (reverse walk,
-- early stop), the "+N new" badge count, and per-device cap trimming.
CREATE INDEX IF NOT EXISTS logcat_serial_id  ON logcat_entry(serial, id);
-- Serves the retention prune (DELETE WHERE created_at < cutoff).
CREATE INDEX IF NOT EXISTS logcat_created_at ON logcat_entry(created_at);
