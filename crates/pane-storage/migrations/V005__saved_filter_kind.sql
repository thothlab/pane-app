-- Saved filters are now scoped by kind so the Logcat window can have its
-- own saved-filter list without colliding with captures filters (different
-- DSL grammar; would be invalid if applied to the wrong view). Existing
-- rows default to 'captures' — that's where they were created from before
-- this migration.
ALTER TABLE saved_filter ADD COLUMN kind TEXT NOT NULL DEFAULT 'captures';
CREATE INDEX IF NOT EXISTS idx_saved_filter_kind ON saved_filter(kind);
