-- Per-device rule enablement.
--
-- V014 is deliberately skipped: `capture_started_index` already holds that
-- number in shipped 0.2.14 databases, and reusing it aborts startup with
-- "applied migration is different than filesystem one". The
-- migrate_real_db test exists to catch exactly this and did.
--
-- Until now a rule was live on its own `enabled` flag alone, which meant every
-- connected device saw the same set of mocks — two phones could not run two
-- different scenarios at once. The flag itself now carries a scope:
--
--   enabled=0                     -> off everywhere
--   enabled=1, scope='all'        -> on everywhere, including devices paired later
--   enabled=1, scope='set'        -> on exactly for the devices listed in rule_device
--
-- This is deliberately NOT a second switch layered on top of the checkbox: the
-- rejected collection cascade (see the doc comment on `list_active_rules`) put
-- the reason a ticked rule stayed silent somewhere other than the rule. Here
-- everything that decides liveness still lives on the rule itself.
--
-- Traffic we cannot attribute (device_id NULL: iOS, port 8888 without host
-- capture, an exhausted port pool) sees only scope='all' rules — a request whose
-- origin is unknown must not pick up a scenario aimed at a specific device.
--
-- 'all' is the default, so every existing rule keeps its current behaviour.
ALTER TABLE rule ADD COLUMN enabled_scope TEXT NOT NULL DEFAULT 'all';

-- device_id is TEXT, not a FK: it holds either a device-row id or the
-- '__host__' sentinel used for this Mac's own traffic, and unpairing a device
-- must not cascade into the rule library. Orphans are cleaned up explicitly on
-- devices.remove. The FK to rule() does cascade — foreign_keys=ON is set on
-- every connection — so deleting a rule takes its scope rows with it.
CREATE TABLE IF NOT EXISTS rule_device (
    rule_id   TEXT NOT NULL REFERENCES rule(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL,
    PRIMARY KEY (rule_id, device_id)
);
CREATE INDEX IF NOT EXISTS idx_rule_device_device ON rule_device(device_id, rule_id);
