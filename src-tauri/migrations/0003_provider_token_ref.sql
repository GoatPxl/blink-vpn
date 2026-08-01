BEGIN;

ALTER TABLE providers ADD COLUMN token_ref TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_providers_token_ref ON providers(token_ref);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (3);

COMMIT;
