BEGIN;

ALTER TABLE vpn_nodes ADD COLUMN plan_key TEXT;
ALTER TABLE vpn_nodes ADD COLUMN size_slug TEXT;
ALTER TABLE vpn_nodes ADD COLUMN price_monthly_usd REAL;
ALTER TABLE vpn_nodes ADD COLUMN price_hourly_usd REAL;
ALTER TABLE vpn_nodes ADD COLUMN price_source_timestamp TEXT;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (6);

COMMIT;
