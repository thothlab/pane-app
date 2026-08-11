-- Why a capture failed, not just that it did.
--
-- `error_kind` is a bucket ('tls_handshake', 'tunneled', 'upstream'); the
-- actual message — the TLS alert the client sent, the I/O error, the reason a
-- host is being tunnelled — was passed to mark_error and dropped on the floor.
-- That made "why did this machine stop decrypting" unanswerable after the fact,
-- which is exactly the question the SSL-passthrough work created.
ALTER TABLE capture ADD COLUMN error_detail TEXT;
