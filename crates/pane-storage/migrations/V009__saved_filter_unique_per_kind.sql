-- saved_filter.name was UNIQUE globally (V001), but V005 introduced `kind`
-- to scope filters to "captures" vs "logcat". A global name-unique means the
-- two scopes can't share a name and a same-name save hits a UNIQUE error
-- instead of updating. Rebuild the table with UNIQUE(name, kind) so each
-- scope is independent and save-by-name can dedup per kind.
CREATE TABLE saved_filter_new (
    id      TEXT PRIMARY KEY,
    name    TEXT NOT NULL,
    query   TEXT NOT NULL,
    color   TEXT NOT NULL,
    pinned  INTEGER NOT NULL DEFAULT 0,
    kind    TEXT NOT NULL DEFAULT 'captures',
    UNIQUE(name, kind)
);

INSERT INTO saved_filter_new (id, name, query, color, pinned, kind)
    SELECT id, name, query, color, pinned, kind FROM saved_filter;

DROP TABLE saved_filter;
ALTER TABLE saved_filter_new RENAME TO saved_filter;

CREATE INDEX IF NOT EXISTS idx_saved_filter_kind ON saved_filter(kind);
