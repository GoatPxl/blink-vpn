BEGIN;

CREATE INDEX IF NOT EXISTS idx_providers_slug ON providers(slug);
CREATE INDEX IF NOT EXISTS idx_vpn_nodes_provider_id ON vpn_nodes(provider_id);
CREATE INDEX IF NOT EXISTS idx_vpn_nodes_status ON vpn_nodes(lifecycle_status);
CREATE INDEX IF NOT EXISTS idx_tunnels_node_id ON tunnels(node_id);
CREATE INDEX IF NOT EXISTS idx_tunnels_status ON tunnels(status);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (2);

COMMIT;
