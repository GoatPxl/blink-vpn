BEGIN;

-- Generic OpenStack as a production provider.
--
-- Not a vendor but software many operators run, so this one row serves every OpenStack cloud —
-- including Rackspace's OpenStack Flex, which is the reference target. Which cloud is decided by
-- the Keystone auth URL in the credential, not by the provider row.
--
-- `token_ref` stays NULL until the user saves a credential; the row exists so the provider is
-- listed and can be configured. `ensure_builtin_providers` re-creates it at runtime if it is ever
-- removed, so this seed and that helper must agree on the id, name, and slug.
INSERT OR IGNORE INTO providers (id, name, slug, token_ref)
VALUES ('provider-openstack', 'OpenStack', 'openstack', NULL);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (24);

COMMIT;
