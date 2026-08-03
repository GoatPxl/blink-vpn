BEGIN;

ALTER TABLE tunnels ADD COLUMN client_private_key_ref TEXT;
ALTER TABLE tunnels ADD COLUMN client_address_cidr TEXT;
ALTER TABLE tunnels ADD COLUMN dns_server TEXT;
ALTER TABLE tunnels ADD COLUMN allowed_ips TEXT;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (10);

COMMIT;
