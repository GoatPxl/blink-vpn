BEGIN;

-- The client keypair is now generated on the host *before* the node is provisioned, because the
-- client public key ships inside the server's [Peer] block in the bootstrap payload. The private
-- key stays in Windows Credential Manager; only its reference is recorded here so the tunnel
-- created later reuses the same key instead of generating a second, unpaired one.
ALTER TABLE vpn_nodes ADD COLUMN client_private_key_ref TEXT;
ALTER TABLE vpn_nodes ADD COLUMN client_public_key TEXT;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (12);

COMMIT;
