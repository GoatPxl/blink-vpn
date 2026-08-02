# Tauri Command Contracts

## Overview

The frontend talks to Rust through a narrow command surface. Commands are Phase 1 scoped for Windows + WireGuard and are routed through provider adapters.

Provider model in this phase:

- Production cloud path: DigitalOcean adapter.
- Development path: Docker adapter with synthetic API-style responses and local container provisioning.

DigitalOcean API reference for provider payload and lifecycle semantics:

- https://docs.digitalocean.com/reference/api/reference/droplets/

## Generalized Node Management API (Logical Contract)

This is the provider-neutral operation set exposed by the orchestrator architecture.

| Logical operation | Intent | Phase 1 implementation status |
| --- | --- | --- |
| `create` | Provision node and produce endpoint metadata | Implemented (`create_vpn_node_to_ready`) |
| `edit` | Change mutable node metadata/settings | Partial; modeled, not fully exposed for all providers |
| `suspend` | Pause operations against provider or node | Provider-level implemented (`suspend_provider`) |
| `resume` | Re-enable suspended provider or node | Provider-level implemented (`resume_provider`) |
| `delete` | Deprovision remote node and local state | Implemented (`destroy_vpn_node`, `force_cleanup_vpn_node`) |
| `get_system_info` | Read runtime health/status information | Implemented (`get_health_panel_data`, `read_client_tunnel_status`, diagnostics) |
| `list` / `get` | Query local provider/node/tunnel metadata | Implemented (`list_providers`, `list_vpn_nodes`, `list_tunnels_for_node`) |

## Command List

| Command | Request | Response | Notes |
| --- | --- | --- | --- |
| `hello` | `name?: string` | `string` | Sanity check only |
| `register_digitalocean_provider_token` | `provider_id`, `provider_name`, `api_token` | `string` | Stores token in secure storage and updates provider row |
| `register_docker_provider_token` | `provider_id`, `provider_name`, `api_token` | `string` | Dev-only provider token registration (`dkr_v1_` / `docker_` prefix) |
| `validate_provider_token` | `provider_id` | `string` | Validates token through the selected provider adapter |
| `list_provider_regions` | `provider_id` | `string[]` | Provider catalog regions via adapter |
| `list_provider_pricing` | `provider_id`, `region` | `ProviderPricingResponse` | Provider plan/pricing via adapter |
| `create_vpn_node_to_ready` | `node_id`, `provider_id`, `node_name`, `region`, `plan_key` | `string` | Provisions provider node and waits for readiness |
| `create_and_activate_client_tunnel` | `tunnel_id`, `node_id`, `tunnel_name`, `client_address_cidr`, `dns_server?`, `allowed_ips` | `string` | Generates keypair, imports tunnel, activates it |
| `activate_client_tunnel` | `tunnel_id` | `void` | Local WireGuard activation |
| `deactivate_client_tunnel` | `tunnel_id` | `void` | Local WireGuard deactivation |
| `connect_tunnel` | `node_id`, `tunnel_id` | `void` | Marks tunnel connected and updates node status |
| `disconnect_tunnel` | `node_id`, `tunnel_id` | `void` | Marks tunnel disconnected and updates node status |
| `suspend_provider` | `provider_id` | `void` | Suspends adapter operations for provider |
| `resume_provider` | `provider_id` | `void` | Resumes adapter operations for provider |
| `delete_provider` | `provider_id` | `void` | Deletes provider when no dependent nodes exist |
| `destroy_vpn_node` | `node_id` | `void` | Conservative destroy path |
| `force_cleanup_vpn_node` | `node_id` | structured cleanup result | Best-effort cleanup for interrupted provisioning |
| `list_vpn_nodes` | none | `VpnNodeRecord[]` | Reads local node rows |
| `list_providers` | none | `ProviderRecord[]` | Reads configured providers |
| `list_tunnels_for_node` | `node_id` | `TunnelRecord[]` | Reads tunnels for one node |
| `read_client_tunnel_status` | `tunnel_id` | `string` | Human-readable status summary |
| `get_health_panel_data` | `node_id`, `tunnel_id?` | `HealthPanelData` | UI health card data |
| `get_diagnostics_bundle` | none | `DiagnosticsBundleResponse` | Redacted bundle for support |
| `export_diagnostics_bundle` | none | `string` | Returns local file path to sanitized JSON export |
| `check_docker_readiness` | none | `DockerReadinessResponse` | Dev-only check; fails when `GOATSTARE_DEV_DOCKER` is disabled |
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
Provider registered with token_ref: do-token__provider-1__...
```

### Save Docker dev provider token

Request:

```json
{
  "provider_id": "provider-docker",
  "provider_name": "Docker",
  "api_token": "dkr_v1_local_dev_token"
}
```

Response:

```text
Provider registered with token_ref: do-token__provider-docker__...
```

### Create a node

Request:

```json
{
  "node_id": "node-1",
  "provider_id": "provider-1",
  "node_name": "Primary Node",
  "region": "nyc1",
  "plan_key": "s-1vcpu-1gb"
}
```

Response:

```text
Create node complete: node_id=node-1, droplet_id=42, endpoint_ip=203.0.113.10, listen_port=51820, correlation_id=..., idempotent_reuse=false
```

### Check Docker readiness (dev mode)

Response shape:

```json
{
  "installed": true,
  "daemon_reachable": true,
  "cli_version": "Docker version 27.x.x, build ...",
  "daemon_version": "27.x.x",
  "message": "Docker CLI and daemon are available for development emulation.",
  "action_guidance": "You can use Docker development mode workflows from Settings and Node planning surfaces."
}
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
| Provider | token validation, create/delete, bootstrap wait, region/pricing fetch | Validate token, check provider status, then retry |
| WireGuard | local service missing, command failure, tunnel not found | Repair WireGuard and use force cleanup for stale artifacts |
| Database / local state | missing rows, invalid transitions | Refresh dashboard or force cleanup |
| Invalid state | lifecycle mismatch, interrupted workflow | Force cleanup, refresh, then retry |
| Unknown | anything else | Retry with caution and inspect diagnostics |

## Assumptions

- Node-level `edit` and `suspend` operations will be expanded over the same provider adapter contract.
