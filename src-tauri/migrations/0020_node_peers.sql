BEGIN;

-- A server's [Peer] list is fixed when the node is created.
--
-- AGENTS.md §2 makes provisioning write-only, so there is no channel to register a peer after the
-- node exists. Every node therefore bakes DEFAULT_PEER_SLOTS peers into its cloud-init payload up
-- front and hands them out locally as devices are added.
--
-- This replaces vpn_nodes.client_public_key / client_private_key_ref, which between them described
-- exactly one peer. Slot 0 is the Windows desktop those columns used to mean.
CREATE TABLE IF NOT EXISTS node_peers (
  id                     TEXT PRIMARY KEY,
  node_id                TEXT NOT NULL REFERENCES vpn_nodes(id) ON DELETE CASCADE,
  slot_index             INTEGER NOT NULL,
  -- NULL until a device claims the slot. The label is what the user typed, e.g. "Pixel".
  label                  TEXT,
  client_public_key      TEXT NOT NULL,
  client_private_key_ref TEXT NOT NULL,
  client_address_cidr    TEXT NOT NULL,
  claimed_at             TEXT,
  created_at             TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  UNIQUE (node_id, slot_index),
  UNIQUE (node_id, client_address_cidr)
);

CREATE INDEX IF NOT EXISTS idx_node_peers_node_id ON node_peers(node_id);

-- A tunnel names the peer it uses instead of carrying its own copy of the key reference and
-- address. Two rows describing one peer is exactly the drift AGENTS.md §5 exists to prevent.
ALTER TABLE tunnels ADD COLUMN peer_id TEXT REFERENCES node_peers(id);

-- The superseded single-peer columns, dropped rather than left in place.
ALTER TABLE vpn_nodes DROP COLUMN client_public_key;
ALTER TABLE vpn_nodes DROP COLUMN client_private_key_ref;
ALTER TABLE tunnels DROP COLUMN client_private_key_ref;
ALTER TABLE tunnels DROP COLUMN client_address_cidr;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (20);

COMMIT;
