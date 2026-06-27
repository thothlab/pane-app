-- Per-device capture attribution. Each USB Android device routes through its
-- own Mac-side proxy port; the proxy resolves the device from that port and
-- stamps the persisted device-row id here. NULL for old captures, iOS, and
-- connections on unmapped ports.
ALTER TABLE capture ADD COLUMN device_id TEXT;
