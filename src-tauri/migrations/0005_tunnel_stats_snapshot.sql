BEGIN;

ALTER TABLE tunnels ADD COLUMN last_handshake_age_seconds INTEGER;
ALTER TABLE tunnels ADD COLUMN rx_bytes INTEGER;
ALTER TABLE tunnels ADD COLUMN tx_bytes INTEGER;
ALTER TABLE tunnels ADD COLUMN stats_updated_at TEXT;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (5);

COMMIT;
