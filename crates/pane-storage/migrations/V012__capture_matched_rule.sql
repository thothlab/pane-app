-- Record which rule served a stubbed/patched response.
--
-- `capture.state` already distinguishes 'stubbed' and 'patched' from
-- 'completed', so a caller can tell a mocked response from a live one. What it
-- could not tell you is *which* rule did it — `serve_stub` and the patch path
-- both had the matched `ActiveRule` in hand and dropped it on the floor.
--
-- That gap matters for automated runs: with a large rule library, asserting
-- "this response came from a mock" is much weaker than "this response came
-- from rule X". Without the latter, a run that silently matched the wrong rule
-- still looks green.
--
-- Nullable and unconstrained on purpose:
--   * every pre-existing capture row has NULL here, and 'completed' rows
--     always will;
--   * no FK to `rule(id)` — deleting a rule must not cascade into rewriting
--     capture history, and a capture should still name the rule that served it
--     even after that rule is gone. The name is denormalized for the same
--     reason.
ALTER TABLE capture ADD COLUMN matched_rule_id   TEXT;
ALTER TABLE capture ADD COLUMN matched_rule_name TEXT;

CREATE INDEX IF NOT EXISTS idx_capture_matched_rule ON capture(matched_rule_id);
