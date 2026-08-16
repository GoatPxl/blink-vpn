BEGIN;

-- Session start time for an active tunnel, so the dashboard can show a real elapsed timer.
-- NULL means "not connected", or "connected, but the start time is unknown" (a tunnel that was
-- already running before this column existed, or one started outside the app). Callers must
-- render the unknown case honestly rather than substituting zero.
ALTER TABLE tunnels ADD COLUMN connected_at TEXT;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (15);

COMMIT;
