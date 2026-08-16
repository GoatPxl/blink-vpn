-- Seed the built-in Amazon EC2 provider row.
--
-- INSERT OR IGNORE, like the earlier provider seeds: `ensure_builtin_providers` also re-creates
-- this row at runtime, and several commands race to do so on a cold dashboard.
--
-- token_ref stays NULL until the user saves a credential. The EC2 adapter reads a two-field
-- credential (access_key_id, secret_access_key) from the one reference this column holds.
INSERT OR IGNORE INTO providers (id, name, slug, token_ref)
VALUES ('provider-aws-ec2', 'Amazon EC2', 'aws-ec2', NULL);
