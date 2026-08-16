BEGIN;

-- Remember whether a provider key passed its last check, so the Settings page does not report
-- every stored key as unchecked after a restart.
--
-- The timestamp travels with the result on purpose: "verified" with no date would keep asserting a
-- check that might be weeks old and since revoked. Callers show the age alongside the verdict.
ALTER TABLE providers ADD COLUMN last_validation_status TEXT;
ALTER TABLE providers ADD COLUMN last_validated_at TEXT;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (16);

COMMIT;
