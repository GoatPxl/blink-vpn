BEGIN;

-- Google Compute Engine as a production provider.
--
-- `token_ref` stays NULL until the user saves a credential; the row exists so the provider is
-- listed and can be configured. `ensure_builtin_providers` re-creates it at runtime if it is ever
-- removed, so this seed and that helper must agree on the id, name, and slug.
INSERT OR IGNORE INTO providers (id, name, slug, token_ref)
VALUES ('provider-gce', 'Google Compute Engine', 'gce', NULL);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (23);

COMMIT;
