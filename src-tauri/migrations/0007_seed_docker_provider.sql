BEGIN;

INSERT OR IGNORE INTO providers (id, name, slug, token_ref)
VALUES ('provider-1', 'DigitalOcean', 'digitalocean', NULL);

INSERT OR IGNORE INTO providers (id, name, slug, token_ref)
VALUES ('provider-docker', 'Docker', 'docker', NULL);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (7);

COMMIT;
