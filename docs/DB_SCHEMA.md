# Database Schema

## Overview

SQLite stores local metadata, lifecycle state, and runtime snapshots. Secret values are stored in secure storage, while SQLite persists secret references.

## Tables

### providers

Columns:

- `id` TEXT PRIMARY KEY
- `name` TEXT NOT NULL
- `slug` TEXT NOT NULL UNIQUE
- `status` TEXT NOT NULL
- `token_ref` TEXT NULL
- `last_validation_status` TEXT NULL
- `last_validated_at` TEXT NULL
- `created_at` TEXT NOT NULL
- `updated_at` TEXT NOT NULL

`last_validation_status` is `ok` or `failed` from the last key check, with `last_validated_at`
recording when it ran. Persisted so the verdict survives a restart instead of every stored key
reporting "not checked" on each launch — and the timestamp travels with it, because a verdict with
no age would keep asserting a check that may be weeks old and since revoked.

Both are **cleared whenever the key changes or is removed**: a result describes the key that was
checked, not the provider, so carrying it onto a replacement would mark an unchecked credential as
verified.

Indexes:

- `idx_providers_slug`
- `idx_providers_token_ref`

### vpn_nodes

Columns:

- `id` TEXT PRIMARY KEY
- `provider_id` TEXT NOT NULL REFERENCES providers(id)
- `name` TEXT NOT NULL
- `region` TEXT NOT NULL
- `lifecycle_status` TEXT NOT NULL
- `external_ref` TEXT NULL — the provider's own identifier for the node, opaque above the adapter.
  A DigitalOcean droplet id (`42`), an EC2 `<region>/<instance-id>` (`us-east-1/i-0abc123def4567890`),
  or a Docker container id. Nothing outside the owning adapter parses it; it was briefly read back as
  a `u64`, and the parse failure branch skipped the remote delete while reporting success.
- `public_ipv4` TEXT NULL
- `server_public_key` TEXT NULL
- `error_reason` TEXT NULL
- `created_at` TEXT NOT NULL
- `updated_at` TEXT NOT NULL

Indexes:

- `idx_vpn_nodes_provider_id`
- `idx_vpn_nodes_status`

### tunnels

Columns:

- `id` TEXT PRIMARY KEY
- `node_id` TEXT NOT NULL REFERENCES vpn_nodes(id) ON DELETE CASCADE
- `name` TEXT NOT NULL
- `status` TEXT NOT NULL
- `interface_name` TEXT NULL
- `assigned_ipv4` TEXT NULL
- `peer_id` TEXT NULL REFERENCES node_peers(id)
- `dns_server` TEXT NULL
- `allowed_ips` TEXT NULL
- `last_error` TEXT NULL
- `last_handshake_age_seconds` INTEGER NULL
- `rx_bytes` INTEGER NULL
- `tx_bytes` INTEGER NULL
- `stats_updated_at` TEXT NULL
- `connected_at` TEXT NULL
- `created_at` TEXT NOT NULL
- `updated_at` TEXT NOT NULL

`peer_id` names the `node_peers` row supplying this tunnel’s key and address. It replaced
`client_private_key_ref` and `client_address_cidr` (migration 0020): a tunnel carrying its own copy
of a peer’s identity was a second source of truth for the same fact, which is the drift AGENTS.md
§5 exists to prevent.

`dns_server` and `allowed_ips` record what the last connect used. `dns_server` in particular is a
**record, not an input**: the value written into a new config comes from `app_settings`, so there
is one source of truth for what the next connect will do.

`connected_at` is the start of the current session, stamped on a successful connect and cleared on
every disconnect path. NULL therefore means either "not connected" or "connected, but the start is
unknown" — a tunnel already running when the app started, which reconciliation deliberately does
**not** backfill, because the real start time is unknowable at that point. The dashboard renders
the unknown case as unknown rather than counting from an arbitrary moment.

Indexes:

- `idx_tunnels_node_id`
- `idx_tunnels_status`

### node_peers

Columns:

- `id` TEXT PRIMARY KEY
- `node_id` TEXT NOT NULL REFERENCES vpn_nodes(id) ON DELETE CASCADE
- `slot_index` INTEGER NOT NULL
- `label` TEXT NULL
- `client_public_key` TEXT NOT NULL
- `client_private_key_ref` TEXT NOT NULL
- `client_address_cidr` TEXT NOT NULL
- `claimed_at` TEXT NULL
- `created_at` TEXT NOT NULL

One row per `[Peer]` block baked into a server. Every row a node will ever have is written when it
is provisioned, because AGENTS.md §2 makes provisioning write-only — there is no channel to
register a peer on a running server, so a slot that was not baked can never exist. Slot 0 is the
Windows desktop and arrives with a label; the rest are unclaimed until a device takes one.

`label IS NULL` means the slot is free. Claiming one only sets `label` and `claimed_at`; nothing is
sent anywhere. `client_private_key_ref` points at Windows Credential Manager and is never
serialized to the UI.

Constraints: `UNIQUE (node_id, slot_index)` and `UNIQUE (node_id, client_address_cidr)` — two
devices answering to the same tunnel address would be a silent routing failure.

This table replaced `vpn_nodes.client_public_key` / `client_private_key_ref`, which between them
could describe exactly one peer. Both were dropped in migration 0020.

Indexes:

- `idx_node_peers_node_id`

### activity_events

Columns:

- `id` INTEGER PRIMARY KEY AUTOINCREMENT
- `node_id` TEXT NULL
- `provider_slug` TEXT NULL
- `category` TEXT NOT NULL — `provision` · `connect` · `disconnect` · `destroy` · `cleanup` · `app`
- `stage` TEXT NOT NULL
- `outcome` TEXT NOT NULL — `info` · `success` · `error`
- `correlation_id` TEXT NULL
- `detail` TEXT NOT NULL
- `created_at` TEXT NOT NULL

What the app did, one recorded step at a time. It is the app's memory of its own actions: without
it, closing the window forgets every connect, disconnect and teardown.

`correlation_id` groups the stages of one run, so a failed provision reads as a single entry with
its steps rather than eight unrelated rows. `node_id` is **nullable** because a startup reconcile
belongs to the app, not to any one node.

`detail` is redacted at the **write** boundary, in `ActivityRecorder::record` — nothing redacts on
read. An `error` outcome records `describe_error_chain`, the flattened `#[source]` chain, rather
than the outermost message: a droplet rejected for a bad name used to record only "Node creation or
bootstrap setup failed", with the provider's actual complaint attached to the error and printed
nowhere.

Bounded to the newest 1000 rows by `prune_activity_events`, called once from `reconcile_startup`.
Its predecessor was never pruned, which was survivable only because a single flow wrote to it.

Replaces `init_run_logs` (migration 0009, dropped by 0017), which was written **only** by
`create_vpn_node_to_ready`. Its `node_id TEXT NOT NULL` could not hold an app-level event, and its
`init_kind` column held `cloud-init` / `docker-init`, selected by comparing the provider slug inside
the orchestrator — a provider branch outside an adapter, which AGENTS.md §3 forbids. `provider_slug`
carries that distinction already, so the column was dropped rather than migrated.

Indexes:

- `idx_activity_events_created`
- `idx_activity_events_node`

### app_settings

Columns:

- `key` TEXT PRIMARY KEY
- `value` TEXT NOT NULL
- `updated_at` TEXT NOT NULL

Keys:

- `wireguard_dns_server` — DNS server written into generated tunnel configs. Defaults to `1.1.1.1`
  when unset. An explicitly stored empty string means "emit no `DNS` line", which leaves the system
  resolver in place — distinct from unset. A stored value that no longer parses as an IP is treated
  as unset rather than propagated, so a bad value cannot render a config that fails every connect.
- `wireguard_keepalive_seconds` — `PersistentKeepalive` interval. Defaults to `25`; `0` disables.
- `current_node_id` — the node the app connects to by default. Written on a successful connect and
  when the user chooses a server explicitly; cleared when that node is destroyed. Reads validate the
  id against the node list, so a dangling pointer resolves to "unset" rather than aiming connect at a
  server that no longer exists. Absent when the user has several nodes and has not chosen between
  them; a single node needs no key, since it is unambiguously the current one.

The table is seeded empty. It previously held `wireguard_connection_mode`, deleted by migration 0014
along with the connection-mode toggle.

Accessed through `Database::get_setting` / `set_setting`, and reached from the frontend only via
typed command pairs — `get_current_node_id` / `set_current_node_id` and `get_tunnel_settings` /
`set_tunnel_settings` — never directly. Each setting validates in the orchestrator before it is
stored, so an unusable value cannot reach config generation.

## Lifecycle Enums

Node status values:

- `new`
- `provisioning`
- `bootstrapping`
- `ready`
- `connected`
- `error`
- `deleting`

Tunnel status values:

- `new`
- `ready`
- `connected`
- `disconnected`
- `error`
- `deleting`

## Migration Model

Current migration sequence in [src-tauri/migrations](src-tauri/migrations):

1. Base tables
2. Performance indexes
3. Provider token reference
4. Node server key metadata
5. Tunnel stats snapshots
6. Provider pricing snapshots
7. Docker provider seed
8. Provider status metadata
9. Init run logs *(dropped by 17)*
10. Tunnel config metadata
11. App settings
12. Node client key
13. Node endpoint port
14. Drop `wireguard_connection_mode`
15. Tunnel `connected_at`
16. Provider validation result
17. Activity events *(replaces init run logs)*
18. Backfill legacy correlation ids
19. Amazon EC2 provider seed

Migration rules:

- Use additive changes when possible.
- Keep backward compatibility for existing rows.
- Update enum constraints and Rust enum mapping together.
- Do not introduce plaintext secret storage.
