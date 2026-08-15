BEGIN;

-- Remove the admin/user WireGuard connection mode setting.
--
-- The toggle promised two tunnel backends but had one: Admin Mode silently and permanently
-- degraded to the service path on the first call in a non-elevated process, and User Mode
-- prompted for elevation anyway. There is now a single connection path, so the stored preference
-- is meaningless and would only mislead anyone reading the settings table.
--
-- The app_settings table itself is kept for future settings.
DELETE FROM app_settings WHERE key = 'wireguard_connection_mode';

INSERT OR IGNORE INTO schema_migrations(version) VALUES (14);

COMMIT;
