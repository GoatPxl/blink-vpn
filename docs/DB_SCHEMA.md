# Database Schema

## Overview

SQLite stores local metadata and runtime snapshots only. Secrets stay out of SQLite; only secret references are persisted.

## Tables

### `providers`

Columns:

- `id` TEXT primary key
- `name` TEXT not null
- `slug` TEXT not null unique
- `token_ref` TEXT nullable, unique via index when present
- `created_at` TEXT not null
- `updated_at` TEXT not null

Indexes:

- `idx_providers_slug`
- `idx_providers_token_ref`

### `vpn_nodes`

Columns:

- `id` TEXT primary key
- `provider_id` TEXT not null, foreign key to `providers(id)`
- `name` TEXT not null
- `region` TEXT not null
- `lifecycle_status` TEXT not null
- `external_ref` TEXT nullable
- `public_ipv4` TEXT nullable
- `server_public_key` TEXT nullable
- `error_reason` TEXT nullable
- `created_at` TEXT not null
- `updated_at` TEXT not null

Indexes:

- `idx_vpn_nodes_provider_id`
- `idx_vpn_nodes_status`

### `tunnels`

Columns:

- `id` TEXT primary key
- `node_id` TEXT not null, foreign key to `vpn_nodes(id)` with `ON DELETE CASCADE`
- `name` TEXT not null
- `status` TEXT not null
- `interface_name` TEXT nullable
- `assigned_ipv4` TEXT nullable
- `last_error` TEXT nullable
- `last_handshake_age_seconds` INTEGER nullable
- `rx_bytes` INTEGER nullable
- `tx_bytes` INTEGER nullable
- `stats_updated_at` TEXT nullable
- `created_at` TEXT not null
- `updated_at` TEXT not null

Indexes:

- `idx_tunnels_node_id`
- `idx_tunnels_status`

## Status Enums

### `NodeLifecycleStatus`

- `new`
- `provisioning`
- `bootstrapping`
- `ready`
- `connected`
- `error`
- `deleting`

Valid transitions are enforced in code via `can_transition_to`.

### `TunnelStatus`

- `new`
- `ready`
- `connected`
- `disconnected`
- `error`
- `deleting`

## Transition Rules

### Node lifecycle

- `new -> provisioning`
- `provisioning -> bootstrapping | error | deleting`
- `bootstrapping -> ready | error | deleting`
- `ready -> connected | error | deleting`
- `connected -> ready | error | deleting`
- `error -> deleting`

### Tunnel lifecycle

- `new -> ready`
- `ready -> connected | error | deleting`
- `connected -> disconnected | error | deleting`
- `disconnected -> connected | error | deleting`
- `error -> deleting`

## Migration Guidance

Current migration versions:

1. initial schema for providers, vpn_nodes, tunnels
2. indexes for provider slug, node provider/status, tunnel node/status
3. add `providers.token_ref`
4. add `vpn_nodes.server_public_key`
5. add tunnel runtime snapshot columns

Guidance for future schema changes:

- Add new columns with additive migrations.
- Keep secret values out of SQLite.
- Preserve backward compatibility for existing rows wherever possible.
- When adding status values, update both the Rust enums and the SQLite CHECK constraints.

## Assumptions

- TODO: project-specific fill if a later phase adds new runtime tables or audit tables.
