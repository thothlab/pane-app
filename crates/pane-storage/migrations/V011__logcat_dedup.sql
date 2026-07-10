-- Deduplicate logcat_entry and keep it deduplicated.
--
-- Before this, `adb logcat` re-dumped the entire device ring buffer on every
-- window (re)open and on every reconnect, and each replayed line was INSERTed
-- again — so the table accumulated exact duplicates of historical lines. A
-- logcat line's identity is its serial plus everything adb emits for it
-- (device_ts, pid, tid, level, tag, message); two distinct emissions differ in
-- at least one field, a re-dumped line matches all of them.
--
-- Order matters: CREATE UNIQUE INDEX fails on a table that still holds
-- duplicates, and real databases already contain them. So collapse existing
-- duplicates FIRST (keep the earliest-ingested row of each identical group —
-- lowest id), THEN add the uniqueness constraint. Going forward the ingest
-- path uses INSERT OR IGNORE, so a replayed line is silently dropped.
DELETE FROM logcat_entry
WHERE id NOT IN (
    SELECT MIN(id) FROM logcat_entry
    GROUP BY serial, device_ts, pid, tid, level, tag, message
);

-- Wide key (includes message) but no separate hash column to backfill —
-- SQLite derives the index from existing rows. Insert cost is acceptable: the
-- firehose peaks ~10k lines/s, an order of magnitude under SQLite's
-- INSERT-OR-IGNORE throughput, and the table is retention-capped.
CREATE UNIQUE INDEX IF NOT EXISTS logcat_dedup
    ON logcat_entry(serial, device_ts, pid, tid, level, tag, message);
