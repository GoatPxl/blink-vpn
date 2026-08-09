# Tauri Command Contracts

## Overview

The frontend calls Rust through a typed Tauri command surface. Commands route through a provider registry, a unified NodeProvider interface, and a shared orchestration layer so UI behavior stays consistent across adapters.

Provider adapters currently included:

- DigitalOcean adapter for cloud node lifecycle
- Docker adapter for local container-backed development workflows

## Command Design Principles

- Command inputs are validated at the boundary.
- Secrets are never returned in command payloads.
- Lifecycle operations are idempotent where practical.
- Provider-specific errors are mapped to actionable UI messages.

## Generalized Node Operations

| Logical operation | Intent | Current mapping |
| --- | --- | --- |
| `create` | Provision node and persist endpoint metadata through the shared NodeProvider contract | `create_vpn_node_to_ready` |
| `edit` | Update mutable node metadata/settings | Implemented through provider/state flows and ready for expansion |
| `suspend` | Pause provider or node operations | `suspend_provider` |
| `resume` | Resume provider or node operations | `resume_provider` |
| `delete` | Deprovision remote and local state | `destroy_vpn_node`, `force_cleanup_vpn_node` |
| `get_system_info` | Retrieve runtime health and diagnostics | `get_health_panel_data`, `read_client_tunnel_status`, diagnostics commands |
| `list` / `get` | Query local metadata | `list_providers`, `list_vpn_nodes`, `list_tunnels_for_node` |

## Command Reference

| Command | Request | Response | Notes |
| --- | --- | --- | --- |
| `hello` | `name?: string` | `string` | Connectivity sanity check |
| `register_digitalocean_provider_token` | `provider_id`, `provider_name`, `api_token` | `string` | Stores token in secure storage and persists token reference |
| `register_docker_provider_token` | `provider_id`, `provider_name`, `api_token` | `string` | Development token flow (`dkr_v1_` / `docker_` prefixes) |
| `validate_provider_token` | `provider_id` | `string` | Adapter-level token validation |
| `list_provider_regions` | `provider_id` | `string[]` | Region catalog |
| `list_provider_pricing` | `provider_id`, `region` | `ProviderPricingResponse` | Plan and pricing catalog |
| `create_vpn_node_to_ready` | `node_id`, `provider_id`, `node_name`, `region`, `plan_key` | `string` | Provision and bootstrap workflow |
| `create_and_activate_client_tunnel` | `tunnel_id`, `node_id`, `tunnel_name`, `client_address_cidr`, `dns_server?`, `allowed_ips` | `string` | Generates keypair, configures the local runtime, and activates the tunnel according to the selected mode |
| `activate_client_tunnel` | `tunnel_id` | `void` | Activates a local tunnel through the configured WireGuard mode |
| `deactivate_client_tunnel` | `tunnel_id` | `void` | Deactivates a local tunnel through the configured WireGuard mode |
| `connect_tunnel` | `node_id`, `tunnel_id` | `void` | Marks tunnel connected and updates node lifecycle |
| `disconnect_tunnel` | `node_id`, `tunnel_id` | `void` | Marks tunnel disconnected and updates node lifecycle |
| `suspend_provider` | `provider_id` | `void` | Suspends provider operations |
| `resume_provider` | `provider_id` | `void` | Resumes provider operations |
| `delete_provider` | `provider_id` | `void` | Deletes provider when dependencies are clear |
| `destroy_vpn_node` | `node_id` | `void` | Standard destroy path |
| `force_cleanup_vpn_node` | `node_id` | `ForceCleanupResult` | Recovery cleanup path |
| `list_vpn_nodes` | none | `VpnNodeRecord[]` | Local node records |
| `list_providers` | none | `ProviderRecord[]` | Local provider records |
| `list_tunnels_for_node` | `node_id` | `TunnelRecord[]` | Local tunnels for node |
| `read_client_tunnel_status` | `tunnel_id` | `string` | Runtime status summary |
| `get_health_panel_data` | `node_id`, `tunnel_id?` | `HealthPanelData` | Health card payload |
| `get_diagnostics_bundle` | none | `DiagnosticsBundleResponse` | Sanitized diagnostics payload |
| `export_diagnostics_bundle` | none | `string` | Path to exported diagnostics JSON |
| `check_docker_readiness` | none | `DockerReadinessResponse` | Docker runtime readiness signal |
| `delete_provider_droplet` | `provider_id`, `droplet_id` | `void` | Direct provider cleanup helper |
| `provision_provider_droplet` | `provider_id`, `droplet_name` | `string` | Lower-level provision helper |

## Request and Response Examples

### Register a provider token

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

### Create a node

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

## Error Classes

| Class | Typical source | UI guidance |
| --- | --- | --- |
| Provider | token validation, create/delete, region/pricing fetch | Validate token, check provider status, retry |
| WireGuard | local runtime, mode selection, import, or connect failures | Repair the local runtime, retry, use cleanup when needed |
| State | stale rows, transition conflicts, interrupted flows | Refresh, reconcile, run cleanup |
| Unknown | unclassified exceptions | Retry and export diagnostics |
