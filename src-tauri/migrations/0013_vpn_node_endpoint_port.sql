BEGIN;

-- The port the client dials to reach this node. Adapters report their own endpoint rather than
-- the orchestrator assuming 51820: DigitalOcean publishes the server's listen port directly,
-- while the Docker dev provider maps an ephemeral host port per node so several can run at once.
ALTER TABLE vpn_nodes ADD COLUMN endpoint_port INTEGER;

INSERT OR IGNORE INTO schema_migrations(version) VALUES (13);

COMMIT;
