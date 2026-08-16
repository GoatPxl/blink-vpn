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
| `register_provider_token` | `provider_id`, `provider_name`, `api_token` | `string` | Stores the credential in secure storage and persists its reference. Provider-neutral: the adapter is resolved from the stored provider record, so there is no per-provider command. |
| `validate_provider_token` | `provider_id` | `string` | Adapter-level token validation |
| `list_provider_regions` | `provider_id` | `string[]` | Region catalog |
| `list_provider_pricing` | `provider_id`, `region` | `ProviderPricingResponse` | Plan and pricing catalog |
| `create_vpn_node_to_ready` | `node_id`, `provider_id`, `node_name`, `region`, `plan_key` | `string` | Provision and bootstrap workflow |
| `create_and_activate_client_tunnel` | `tunnel_id`, `node_id`, `tunnel_name`, `client_address_cidr`, `dns_server?`, `allowed_ips` | `string` | Reuses the client keypair generated at provision time and marks the tunnel ready |
| `activate_client_tunnel` | `tunnel_id` | `void` | Activates a local tunnel through the WireGuard tunnel service |
| `deactivate_client_tunnel` | `tunnel_id` | `void` | Deactivates a local tunnel through the WireGuard tunnel service |
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
| `get_health_panel_data` | `node_id`, `tunnel_id?` | `HealthPanelData` | Health card payload. Also reconciles a stale tunnel row against the live service state — see below |
| `get_diagnostics_bundle` | none | `DiagnosticsBundleResponse` | Sanitized diagnostics payload |
| `export_diagnostics_bundle` | none | `string` | Path to exported diagnostics JSON |
| `check_docker_readiness` | none | `DockerReadinessResponse` | Docker runtime readiness signal |
| `delete_provider_droplet` | `provider_id`, `droplet_id` | `void` | Direct provider cleanup helper |
| `provision_provider_droplet` | `provider_id`, `droplet_name` | `string` | Lower-level provision helper |
| `check_wireguard_readiness` | none | `WireGuardReadinessResponse` | Local WireGuard install probe. Infallible — reports readiness in the payload rather than erroring |
| `open_wireguard_install_page` | none | `void` | Opens the WireGuard download page in the default browser (Windows only) |
| `install_wireguard_with_prerequisites` | none | `WireGuardInstallResult` | Runs the WireGuard installer with elevation (Windows only) |
| `force_disconnect_node_service` | `node_id` | `void` | Best-effort teardown of every tunnel service for a node; ignores individual failures |

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

### Health panel payload

```json
{
  "node_id": "node-1",
  "tunnel_id": "tunnel-node-1",
  "endpoint_ip": "203.0.113.10",
  "handshake_age_seconds": null,
  "rx_bytes": 184320,
  "tx_bytes": 90112,
  "external_ipv4": "203.0.113.10",
  "external_ipv6": null,
  "tunnel_status": "connected",
  "metrics_source": "adapter",
  "connected_at": "2026-08-15T10:00:00.000Z"
}
```

Takes an optional `force_ip_probe` flag. The egress addresses are cached for 60 seconds because
health polls far more often than an address changes; the cache is bypassed automatically whenever
the connection state changes, and explicitly when the user asks for a refresh. Leave it off for
ambient polling or the app will hammer a third-party IP service.

Five fields carry meaning in their absence, and callers must not paper over it:

| Field | `null` means | Never |
|---|---|---|
| `external_ipv4` | The egress probe failed | Substituted with `endpoint_ip`. Comparing the two is the egress check, so a stand-in would make "verified" and "never checked" identical |
| `external_ipv6` | This machine has no IPv6 egress | Treated as an error — for this deployment it is the *desired* state, see below |
| `handshake_age_seconds` | `wg show` needs administrator (see AGENTS.md §4) | Reported as `0` |
| `rx_bytes` / `tx_bytes` | No counters could be read | Reported as `0`, which would claim a measured idle tunnel |
| `connected_at` | Not connected, or the session started before the app was watching | Backfilled with the current time |

`external_ipv4` is measured in **every** connection state, not only while connected — the address
changing as the tunnel comes up is the user-visible evidence that it carries traffic.

**IPv6 is not carried by this tunnel.** The bootstrap configures `iptables` MASQUERADE and IPv4
forwarding only, and the client interface is issued a `/32` with no IPv6 address. A non-null
`external_ipv6` while connected therefore means IPv6 traffic is reaching the internet *outside*
the tunnel, and the dashboard reports it as a bypass. If IPv6 egress is ever wanted, it needs
`ip6tables` rules, an IPv6 server CIDR, and an IPv6 client address — not a UI change.

`metrics_source` names the tier that produced the byte counters — `peer` (`wg show`, elevated) or
`adapter` (Win32 `GetIfTable2`, unelevated). The two count different things, so anything
differencing samples over time must reset its baseline when this value changes.

This command also repairs a stale tunnel row when the stored status disagrees with the live
service, writing only on an actual change and never over an `Error` status.

## Error Envelope

Every command rejects with a structured object, never a bare string:

```json
{
  "kind": "wire-guard",
  "message": "Could not start the WireGuard tunnel service.",
  "next_action": "Approve the elevation prompt and retry Connect."
}
```

`kind` is the only field callers may branch on. `message` and `next_action` are for display, and
both are already redacted. The UI must never classify a failure by matching on message text —
that is how reworded backend messages used to silently break recovery paths.

| `kind` | Typical source | UI guidance |
| --- | --- | --- |
| `provider` | token validation, create/delete, region or pricing fetch | Validate the credential, check provider status, retry |
| `wire-guard` | tunnel import, activation, or service failures | Repair the local WireGuard install, retry, use cleanup if needed |
| `invalid-state` | transition conflicts, interrupted flows | Refresh, reconcile, run Force Cleanup |
| `missing-resource` | node, tunnel, or provider row absent | Refresh the dashboard so lists reload |
| `persistence` | local SQLite read/write failure | Refresh; use Force Cleanup if state stays stale |
| `secret-storage` | Windows Credential Manager read/write failure | Re-save the credential in Settings |
| `invalid-input` | request the command cannot accept | Correct the input and retry |
| `dependency` | missing local prerequisite (WireGuard, Docker) | Install or start the prerequisite |
| `unknown` | unclassified exceptions | Retry and export diagnostics |
