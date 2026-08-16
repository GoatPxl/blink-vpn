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
- `external_ref` TEXT NULL
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
- `client_private_key_ref` TEXT NULL
- `client_address_cidr` TEXT NULL
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

The four columns from migration 0010 (`client_private_key_ref`, `client_address_cidr`, `dns_server`,
`allowed_ips`) record what the last connect used. `dns_server` in particular is a **record, not an
input**: the value written into a new config comes from `app_settings`, so there is one source of
truth for what the next connect will do.

`connected_at` is the start of the current session, stamped on a successful connect and cleared on
every disconnect path. NULL therefore means either "not connected" or "connected, but the start is
unknown" — a tunnel already running when the app started, which reconciliation deliberately does
**not** backfill, because the real start time is unknowable at that point. The dashboard renders
the unknown case as unknown rather than counting from an arbitrary moment.

Indexes:

- `idx_tunnels_node_id`
- `idx_tunnels_status`

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
9. Init run logs
10. Tunnel config metadata
11. App settings

Migration rules:

- Use additive changes when possible.
- Keep backward compatibility for existing rows.
- Update enum constraints and Rust enum mapping together.
- Do not introduce plaintext secret storage.
