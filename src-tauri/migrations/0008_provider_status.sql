BEGIN;

ALTER TABLE providers
ADD COLUMN status TEXT NOT NULL DEFAULT 'active'
CHECK (status IN ('active', 'suspended'));

UPDATE providers
SET status = 'active'
WHERE status IS NULL OR status = '';

INSERT OR IGNORE INTO schema_migrations(version) VALUES (8);

COMMIT;
