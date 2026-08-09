BEGIN;

CREATE TABLE IF NOT EXISTS app_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

INSERT INTO app_settings (key, value)
VALUES ('wireguard_connection_mode', 'admin')
ON CONFLICT(key) DO NOTHING;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (11);

COMMIT;