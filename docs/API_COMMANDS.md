# Tauri Command Contracts

## Overview

The frontend talks to Rust through a narrow command surface. All commands are Phase 1 scoped and assume Windows + DigitalOcean + WireGuard.

## Command List

| Command | Request | Response | Notes |
| --- | --- | --- | --- |
| `hello` | `name?: string` | `string` | Sanity check only |
| `register_digitalocean_provider_token` | `provider_id`, `provider_name`, `api_token` | `string` | Stores token in secure storage and updates provider row |
| `validate_provider_token` | `provider_id` | `string` | Validates token through DigitalOcean account endpoint |
| `create_vpn_node_to_ready` | `node_id`, `provider_id`, `node_name`, `region` | `string` | Provisions droplet and waits for readiness |
| `create_and_activate_client_tunnel` | `tunnel_id`, `node_id`, `tunnel_name`, `client_address_cidr`, `dns_server?`, `allowed_ips` | `string` | Generates keypair, imports tunnel, activates it |
| `activate_client_tunnel` | `tunnel_id` | `void` | Local WireGuard activation |
| `deactivate_client_tunnel` | `tunnel_id` | `void` | Local WireGuard deactivation |
| `connect_tunnel` | `node_id`, `tunnel_id` | `void` | Marks tunnel connected and updates node status |
| `disconnect_tunnel` | `node_id`, `tunnel_id` | `void` | Marks tunnel disconnected and updates node status |
| `destroy_vpn_node` | `node_id` | `void` | Conservative destroy path |
| `force_cleanup_vpn_node` | `node_id` | structured cleanup result | Best-effort cleanup for interrupted provisioning |
| `list_vpn_nodes` | none | `VpnNodeRecord[]` | Reads local node rows |
| `list_tunnels_for_node` | `node_id` | `TunnelRecord[]` | Reads tunnels for one node |
| `read_client_tunnel_status` | `tunnel_id` | `string` | Human-readable status summary |
| `get_health_panel_data` | `node_id`, `tunnel_id?` | `HealthPanelData` | UI health card data |
| `get_diagnostics_bundle` | none | `DiagnosticsBundleResponse` | Redacted bundle for support |
| `export_diagnostics_bundle` | none | `string` | Returns local file path to sanitized JSON export |
| `delete_provider_droplet` | `provider_id`, `droplet_id` | `void` | Direct remote cleanup helper |
| `provision_provider_droplet` | `provider_id`, `droplet_name` | `string` | Lower-level provider helper |

## Request / Response Examples

### Save a provider token

Request:

```json
{
  "provider_id": "provider-1",
  "provider_name": "DigitalOcean",
  "api_token": "dop_v1_..."
}
```

Response:

```text
Provider registered with token_ref: do-token:provider-1:...
```

### Create a node

Request:

```json
{
  "node_id": "node-1",
  "provider_id": "provider-1",
  "node_name": "Primary Node",
  "region": "nyc1"
}
```

Response:

```text
Create node complete: node_id=node-1, droplet_id=42, endpoint_ip=203.0.113.10, listen_port=51820, correlation_id=..., idempotent_reuse=false
```

### Force cleanup

Request:

```json
{
  "node_id": "node-1"
}
```

Response shape:

```json
{
  "node_id": "node-1",
  "correlation_id": "...",
  "local_tunnels_removed": 1,
  "remote_droplet_delete_attempted": true,
  "remote_droplet_delete_succeeded": true,
  "node_record_removed": true
}
```

## Error Taxonomy and UI Mapping

Backend errors are converted to user-facing text by the command layer and then mapped in the dashboard into broad recovery classes.

| Error class | Typical source | UI hint |
| --- | --- | --- |
| Provider | token validation, droplet create/delete, bootstrap wait | Validate token or retry after checking DigitalOcean state |
| WireGuard | local service missing, command failure, tunnel not found | Repair WireGuard and use force cleanup for stale artifacts |
| Database / local state | missing rows, invalid transitions | Refresh dashboard or force cleanup |
| Invalid state | lifecycle mismatch, interrupted workflow | Force cleanup, refresh, then retry |
| Unknown | anything else | Retry with caution and inspect diagnostics |

## Assumptions

- TODO: project-specific fill if the command surface changes in Phase 2.
