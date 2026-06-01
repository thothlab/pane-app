-- Schema v4: rule modes (stub vs patch) and the patches list.
--
-- mode='stub'  (default) — engine short-circuits upstream and returns the
--                          rule's prepared status/headers/body verbatim.
--                          Existing behavior; existing rows keep working.
-- mode='patch'           — engine forwards the request upstream, then applies
--                          the patches list to the real response before
--                          returning it. Useful when the client depends on
--                          fresh server-generated fields (tokens, timestamps)
--                          and you only want to mutate a few specific
--                          values.
--
-- patches is a JSON array of { "op": "set"|"delete"|"append", "path": "...",
-- "value": <any> }. `path` uses dot-notation against a virtual response
-- tree: `status`, `headers.<Name>`, `body.<dot-path>`. `value` is parsed
-- as JSON, with a string fallback for plain text.

ALTER TABLE rule ADD COLUMN mode TEXT NOT NULL DEFAULT 'stub';
ALTER TABLE rule ADD COLUMN patches TEXT NOT NULL DEFAULT '[]';
