BEGIN;

ALTER TABLE vpn_nodes ADD COLUMN server_public_key TEXT;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (4);

COMMIT;
