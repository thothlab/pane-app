-- Schema v3: rule collections.
--
-- Rules can be grouped into named collections. A collection has its own
-- enabled toggle that cascades — a rule fires only when both rule.enabled
-- AND (collection.enabled OR collection_id IS NULL). Rules without a
-- collection ("Ungrouped") behave exactly as before.

CREATE TABLE IF NOT EXISTS rule_collection (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    priority    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_collection_enabled_priority ON rule_collection(enabled, priority);

ALTER TABLE rule ADD COLUMN collection_id TEXT REFERENCES rule_collection(id);
CREATE INDEX IF NOT EXISTS idx_rule_collection ON rule(collection_id);
