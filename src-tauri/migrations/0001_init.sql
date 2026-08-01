BEGIN;

CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS providers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  slug TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS vpn_nodes (
  id TEXT PRIMARY KEY,
  provider_id TEXT NOT NULL,
  name TEXT NOT NULL,
  region TEXT NOT NULL,
  lifecycle_status TEXT NOT NULL CHECK (lifecycle_status IN (
    'new',
    'provisioning',
    'bootstrapping',
    'ready',
    'connected',
    'error',
    'deleting'
  )),
  external_ref TEXT,
  public_ipv4 TEXT,
  error_reason TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  FOREIGN KEY(provider_id) REFERENCES providers(id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS tunnels (
  id TEXT PRIMARY KEY,
  node_id TEXT NOT NULL,
  name TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN (
    'new',
    'ready',
    'connected',
    'disconnected',
    'error',
    'deleting'
  )),
  interface_name TEXT,
  assigned_ipv4 TEXT,
  last_error TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  FOREIGN KEY(node_id) REFERENCES vpn_nodes(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);

COMMIT;
