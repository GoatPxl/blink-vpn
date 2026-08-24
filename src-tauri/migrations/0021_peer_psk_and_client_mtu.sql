BEGIN;

-- Per-peer WireGuard preshared key, baked into the server config at create time.
--
-- A PSK is symmetric: the server and exactly one client hold it, so it lives with the peer slot.
-- Generated on the Windows host alongside the peer keypair (AGENTS.md §2 leaves no later channel),
-- and included in exported client configs — WireGuard key material is not a secret here (§6).
-- NULL marks a peer created before PSK support; its server was baked without one, and because the
-- server cannot be reconfigured after create, such a peer connects without a PSK forever.
ALTER TABLE node_peers ADD COLUMN preshared_key TEXT;

-- Client tunnel MTU this node's provider calls for, captured at provision time.
--
-- Some providers run instance NICs below 1500 (Google Compute Engine: 1460), which makes the
-- WireGuard default of 1420 fragment or blackhole. The owning adapter states the hint once, at
-- create; NULL means the provider needs no MTU line in client configs.
ALTER TABLE vpn_nodes ADD COLUMN client_mtu INTEGER;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (21);

COMMIT;
