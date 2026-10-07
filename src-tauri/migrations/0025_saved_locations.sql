BEGIN;

-- A server the user can turn off to stop billing and turn back on later.
--
-- Servers are meant to be short-lived, and destroying one deletes its vpn_nodes row, which used to
-- take everything needed to bring it back with it. A saved location is that "everything": which
-- provider, region and plan, and what it cost last time.
--
-- A separate table rather than a stopped status on vpn_nodes: every query, reconcile pass and
-- status table treats a vpn_nodes row as a server that exists, and an "off" row would need special
-- handling in each of them.
--
-- No REFERENCES clauses: this database does not enable PRAGMA foreign_keys, so they would be
-- documentation pretending to be enforcement. Database::delete_vpn_node clears node_id itself.
CREATE TABLE IF NOT EXISTS saved_locations (
  id                     TEXT PRIMARY KEY,
  provider_id            TEXT NOT NULL,
  name                   TEXT NOT NULL,
  region                 TEXT NOT NULL,
  plan_key               TEXT NOT NULL,
  price_monthly_usd      REAL,
  price_hourly_usd       REAL,
  price_source_timestamp TEXT,
  -- The running server for this location, or NULL while it is off.
  node_id                TEXT,
  last_turned_on_at      TEXT,
  created_at             TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at             TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  -- Saving is automatic on every create, so the same server shape must land on one entry.
  UNIQUE (provider_id, region, plan_key)
);

-- Servers that already exist become saved locations, linked to themselves. Ordered so that when
-- two share a shape, the newest wins the link.
INSERT OR IGNORE INTO saved_locations (
  id, provider_id, name, region, plan_key,
  price_monthly_usd, price_hourly_usd, price_source_timestamp, node_id
)
SELECT
  'loc-' || id, provider_id, name, region, plan_key,
  price_monthly_usd, price_hourly_usd, price_source_timestamp, id
FROM vpn_nodes
WHERE plan_key IS NOT NULL
ORDER BY created_at DESC;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (25);

COMMIT;
