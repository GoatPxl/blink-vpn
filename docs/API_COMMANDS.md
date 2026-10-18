# Tauri Command Contracts

## Overview

The frontend calls Rust through a typed Tauri command surface. Commands route through a provider registry, a unified NodeProvider interface, and a shared orchestration layer so UI behavior stays consistent across adapters.

Provider adapters currently included:

- DigitalOcean adapter for cloud node lifecycle
- Amazon EC2 adapter for cloud node lifecycle
- Linode adapter for cloud node lifecycle
- Google Compute Engine adapter for cloud node lifecycle
- OpenStack adapter for cloud node lifecycle (experimental)
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
| `delete` | Deprovision remote and local state | `destroy_vpn_node`, `force_cleanup_vpn_node`, `turn_off_saved_location` |
| `saved locations` | Turn a server off to stop billing and back on with the same provider, region and plan | `list_saved_locations`, `turn_on_saved_location`, `turn_off_saved_location`, `forget_saved_location` |
| `get_system_info` | Retrieve runtime health and diagnostics | `get_health_panel_data`, `get_tunnel_phase`, diagnostics commands |
| `list` / `get` | Query local metadata | `list_providers`, `list_vpn_nodes`, `list_tunnels_for_node` |

## Command Reference

| Command | Request | Response | Notes |
| --- | --- | --- | --- |
| `register_provider_credential` | `provider_id`, `provider_name`, `credentials` (map of field key to value) | `string` | Stores the credential in secure storage and persists its reference. Provider-neutral: the adapter is resolved from the stored provider record, so there is no per-provider command. Every field the adapter declares must be present and non-empty. |
| `list_provider_credential_fields` | `provider_id` | `CredentialField[]` | The credential inputs this provider needs — `key`, `label`, `placeholder`, `secret`, `help`, and on a provider's first field `token_page_url` and `scope_hint` (where to get the key, what to tick; null for OpenStack, whose console varies by cloud). The UI renders whatever it returns rather than branching on a provider slug. |
| `open_provider_token_page` | `provider_id` | `void` | Opens the adapter-declared `token_page_url` in the default browser. **Takes a provider id, never a URL**: the backend opens only the address the adapter declared, and refuses anything that is not plain https with no shell metacharacters. Rejected with `invalid-input` for a provider with no such page |
| `validate_provider_token` | `provider_id` | `string` | Checks the stored credential through the adapter and records the verdict on that provider row. Reads the *stored* credential, so it never validates something newly typed but unsaved. The returned string names the account the credential belongs to — display text only; nothing parses it |
| `list_provider_regions` | `provider_id` | `ProviderRegion[]` | Live region catalog with normalized geography — needs a key. See below |
| `list_known_provider_regions` | `provider_id` | `ProviderRegion[]` | The adapter's curated region table: no key, no network call. Empty for an adapter without one (OpenStack, Docker). For showing where a provider runs before the user has a key; what can actually be built comes from `list_provider_regions` |
| `list_provider_pricing` | `provider_id`, `region` | `ProviderPricingResponse` | Plan and pricing catalog |
| `create_vpn_node_to_ready` | `node_id`, `provider_id`, `node_name`, `region`, `plan_key` | `string` | Provision and bootstrap workflow |
| `create_and_activate_client_tunnel` | `tunnel_id`, `node_id`, `tunnel_name`, `dns_server?`, `allowed_ips` | `string` | Binds the tunnel to the node’s slot 0 peer and marks it ready. The address comes from that slot, never from the caller — the server only accepts that key at that address |
| `list_node_peers` | `node_id` | `NodePeerRecord[]` | Every baked device slot, claimed and unclaimed, in slot order |
| `claim_node_peer` | `node_id`, `peer_id`, `label` | `NodePeerRecord` | Names an unclaimed slot so it becomes a device. Local bookkeeping only — nothing is sent to the server |
| `get_peer_config` | `node_id`, `peer_id` | `PeerConfigPayload` | The slot’s WireGuard config plus a QR module matrix. Refused for an endpoint only this machine can reach |
| `export_peer_config` | `node_id`, `peer_id` | `string` | Writes the config under `peer-configs/` in the app data directory and returns the path |
| `activate_client_tunnel` | `tunnel_id` | `void` | Activates a local tunnel through the WireGuard tunnel service |
| `deactivate_client_tunnel` | `tunnel_id` | `void` | Deactivates a local tunnel through the WireGuard tunnel service |
| `connect_tunnel` | `node_id`, `tunnel_id` | `void` | Registers and starts the tunnel service. **Returning means the service is running, not that the tunnel carries traffic** — poll `get_tunnel_phase` for that. Rejects a second concurrent connect for the same tunnel with `invalid-state` |
| `disconnect_tunnel` | `node_id`, `tunnel_id` | `void` | Marks tunnel disconnected and updates node lifecycle |
| `suspend_provider` | `provider_id` | `void` | Suspends provider operations |
| `resume_provider` | `provider_id` | `void` | Resumes provider operations |
| `delete_provider` | `provider_id` | `void` | Deletes provider when dependencies are clear |
| `destroy_vpn_node` | `node_id` | `void` | Standard destroy path ("Turn off" in the UI). Deletes at the provider **first**; if the provider refuses, fails with `provider` and removes nothing, so a server that is still billing stays listed. The node's saved location is kept, turned off |
| `list_saved_locations` | none | `SavedLocationView[]` | Saved locations, most recently used first, each with `provider_name`, `region_geo` and the linked server's `node_status` (`null` while off) |
| `turn_on_saved_location` | `saved_location_id`, `node_id` | `string` | Provisions the location again under `node_id` — an ordinary create, so fresh keys and a new IP. Blocks until ready. `invalid-state` if it is already on |
| `turn_off_saved_location` | `saved_location_id` | `void` | Destroys the location's server, with `destroy_vpn_node`'s failure semantics. A location that is already off is left alone |
| `forget_saved_location` | `saved_location_id` | `void` | Removes the location. `invalid-state` while it is on |
| `force_cleanup_vpn_node` | `node_id` | `ForceCleanupResult` | Recovery cleanup path — see the note below on reading its result |
| `list_vpn_nodes` | none | `VpnNodeRecord[]` | Local node records, enriched with live `provider_status` and adapter-supplied `region_geo` |
| `get_current_node_id` | none | `string \| null` | The default node; `null` when several exist and none was chosen |
| `set_current_node_id` | `node_id` (nullable) | `void` | Sets or clears the default node; rejects an unknown id |
| `get_tunnel_settings` | none | `TunnelSettings` | DNS server and keepalive used when a tunnel next starts |
| `set_tunnel_settings` | `dns_server` (nullable), `keepalive_seconds` | `void` | Validates before storing; a non-IP DNS value is rejected with `invalid-input` |
| `get_app_preferences` | none | `AppPreferences` | `close_to_tray` (default `true`), `mobile_devices` (default empty), `auto_connect_after_provision` (default `true`), `first_connect_done` (default `false`), `explain_uac` (default `true`), `favourites` (default empty), `game_high_score` (default `0`) |
| `set_app_preferences` | `preferences: AppPreferences` | `AppPreferences` | Validates and stores the whole struct, returning it as stored (device names trimmed, favourites trimmed and deduplicated). One struct argument, so adding a preference does not change the signature. `close_to_tray` applies to the running window immediately; `mobile_devices` to servers created afterwards. An unusable device list or a favourite that is not `provider_id:region` is rejected with `invalid-input`, and nothing is stored |
| `get_launch_at_login` | none | `boolean` | Whether Windows will start Blink, hidden in the tray, at the next sign-in. Read from the registry on every call — the `Run` value must exist and Task Manager's `StartupApproved` flag must not mark it disabled. Not stored in SQLite |
| `set_launch_at_login` | `enabled` | `boolean` | Writes or removes the HKCU `Run` value (`"<exe>" --autostart`) and clears Task Manager's flag, then returns the state read back. No elevation. A registry failure is `dependency` |
| `list_providers` | none | `ProviderRecord[]` | Local provider records, including `last_validation_status` / `last_validated_at` from the last key check |
| `list_tunnels_for_node` | `node_id` | `TunnelRecord[]` | Local tunnels for node |
| `get_tunnel_phase` | `node_id`, `tunnel_id` | `TunnelPhaseSnapshot` | How far along a connect is. Cheap enough to poll at 1 Hz — see below |
| `get_health_panel_data` | `node_id`, `tunnel_id?` | `HealthPanelData` | Health card payload, including the same `phase` verdict. Also reconciles a stale tunnel row against the live service state — see below |
| `get_diagnostics_bundle` | none | `DiagnosticsBundleResponse` | Sanitized diagnostics payload, including the `activity` history — see below |
| `export_diagnostics_bundle` | none | `string` | Path to exported diagnostics JSON |
| `list_app_locations` | none | `AppLocation[]` | The folders this app writes to, with a `key`, a display `path`, and whether it `exists` yet |
| `reveal_app_location` | `key` | `void` | Opens one listed location in Explorer. **Takes a key, never a path** — see below |
| `check_docker_readiness` | none | `DockerReadinessResponse` | Docker runtime readiness signal |
| `delete_provider_node` | `provider_id`, `node_id` | `void` | Direct provider cleanup helper. `node_id` is the opaque provider reference stored in `vpn_nodes.external_ref`, not a numeric id. |
| `provision_provider_node` | `provider_id`, `node_name` | `string` | Lower-level provision helper |
| `check_wireguard_readiness` | none | `WireGuardReadinessResponse` | Local WireGuard install probe, plus `version` (from `wg --version`, null when unreadable) and `app_version`. Infallible — reports readiness in the payload rather than erroring |
| `open_wireguard_install_page` | none | `void` | Opens the WireGuard download page in the default browser (Windows only) |
| `install_wireguard_with_prerequisites` | none | `WireGuardInstallResult` | Runs the WireGuard installer with elevation (Windows only) |
| `force_disconnect_node_service` | `node_id` | `void` | Best-effort teardown of every tunnel service for a node; ignores individual failures |
| `cancel_connect` | `tunnel_id` | `boolean` | Closes the activity entry of a connect still waiting on its UAC prompt; spawns nothing and never elevates. If the prompt is approved afterwards, the connect tears itself down. `false` if no connect was running |
| `update_tray` | `state: TrayState` | `void` | Replaces the tray icon colour, tooltip and menu. `TrayState` is `{ tone: "secure" \| "busy" \| "off" \| "failed", status, primary?, places_label, places[], power_off? }`, items `{ id, label }`. Menu clicks come back as the `tray-action` event carrying the item id |
| `quit_app` | none | `void` | Exits the app. Called by the frontend after `tray-quit`, once any pending power-off has run. A connected tunnel stays up |

### Reading a `ForceCleanupResult`

```ts
interface ForceCleanupResult {
  node_id: string;
  correlation_id: string;
  local_tunnels_removed: number;
  remote_node_delete_attempted: boolean;
  remote_node_delete_succeeded: boolean;
  node_record_removed: boolean;
}
```

**`remote_node_delete_attempted && !remote_node_delete_succeeded` is a failure, and callers must
present it as one.** It means the local records are gone while the server is not: nothing in the app
points at it any more, and it keeps billing until the user deletes it in the provider's console. The
backend records that case as an `error` activity event; a caller that reports "cleanup complete"
regardless — which the dashboard did — buries the one outcome the user has to act on.

These fields were `remote_droplet_delete_*`. "Droplet" is DigitalOcean's word for a virtual machine,
and the names outlived the assumption that DigitalOcean was the only provider.

## Request and Response Examples

### Register a provider credential

The field keys come from `list_provider_credential_fields`, so the caller never hardcodes them.
DigitalOcean declares one field:

```json
{
  "provider_id": "provider-1",
  "provider_name": "DigitalOcean",
  "credentials": { "api_token": "dop_v1_EXAMPLE_NOT_A_REAL_TOKEN" }
}
```

Amazon EC2 declares two:

```json
{
  "provider_id": "provider-aws-ec2",
  "provider_name": "Amazon EC2",
  "credentials": {
    "access_key_id": "AKIAEXAMPLE_NOT_A_REAL_KEY",
    "secret_access_key": "EXAMPLE_NOT_A_REAL_SECRET"
  }
}
```

Response:

```text
Provider registered with token_ref: provider-cred__provider-aws-ec2__...
```

Credentials saved by earlier versions carry a `do-token__...` reference and a bare token rather than
a field set. Both are still read; only new writes use the current form.

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
Create node complete: node_id=node-1, external_ref=us-east-1/i-0abc123def4567890, endpoint_ip=203.0.113.10, listen_port=51820, correlation_id=..., idempotent_reuse=false
```

### Region catalog

```json
[
  {
    "slug": "sfo3",
    "city": "San Francisco",
    "area": "California",
    "country": "United States",
    "country_code": "US",
    "continent": "North America",
    "latitude": 37.77,
    "longitude": -122.42
  },
  { "slug": "mars1", "city": null, "area": null, "country": null, "country_code": null, "continent": null, "latitude": null, "longitude": null }
]
```

`latitude`/`longitude` come from one place table keyed by city and country
(`backend/provider/place_coordinates.rs`), not by any provider's slug: a city is in the same place
whichever cloud runs a datacentre there. A city the table does not list has no coordinates, and the
map lists it without a pin.

Only `slug` is guaranteed. Providers add regions faster than a curated table follows, so an
unrecognised slug carries no geography rather than a guessed city — **render the slug in that case,
never a fallback place name.** The same shape is attached to each node as `region_geo` by
`list_vpn_nodes`, resolved from the adapter's own table with no token and no network call, so the
fleet stays readable when a provider key goes bad.

### Activity history

`get_diagnostics_bundle` carries the newest 200 rows of `activity_events` as `activity`:

```json
[
  {
    "id": 41,
    "node_id": "node-1",
    "provider_slug": "digitalocean",
    "category": "provision",
    "stage": "create.error",
    "outcome": "error",
    "correlation_id": "6f0c…",
    "detail": "Node creation or bootstrap setup failed <- droplet name must be a valid hostname",
    "created_at": "2026-08-16T10:00:09.123Z"
  }
]
```

`category` is `provision` · `connect` · `disconnect` · `destroy` · `cleanup` · `app`; `outcome` is
`info` · `success` · `error`. Stages sharing a `correlation_id` are one run, and the UI collapses
them into one entry — a failed provision writes eight rows, of which one says what went wrong.

An `error` detail carries the **flattened source chain** (`outer <- cause`), not just the outermost
message. `node_id` is null for app-level events such as a startup reconcile.

This field replaced `init_runs`, whose records came only from the create flow. Consumers must read
`activity`; there is no `init_runs` fallback.

### App locations

`reveal_app_location` accepts only a `key` returned by `list_app_locations` — `diagnostics`,
`logs`, `tunnels`, or `database` — and resolves the path in Rust. **It must never take a path from
the caller**: passing a webview-supplied path to the Windows shell would let anything running in the
page open an arbitrary target. An unrecognised key returns `invalid-input`, and a path that does not
exist yet is refused rather than opened.

### Connect progress: `tunnel_status` vs `phase`

These answer different questions, and conflating them was a real bug.

- **`tunnel_status`** tracks the Windows *service*. It reads `connected` as soon as the service is
  running.
- **`phase`** tracks whether the tunnel actually **carries traffic**. Only this may turn a UI green.

The gap between them is not incidental. The WireGuard adapter captures the default route the moment
its service starts, so from that instant the user's traffic is inside a tunnel whose peer has not
answered — their internet is down. Observed in the field: service running at `10:22:07.851`, first
handshake response at `10:22:23.051`. For fifteen seconds the app reported success while nothing
worked.

```json
{
  "tunnel_id": "tunnel-node-1",
  "tunnel_name": "Primary-Node-wg",
  "phase": "connecting",
  "service_state": "running",
  "elapsed_seconds": 6,
  "rx_bytes": 0,
  "tx_bytes": 592
}
```

| `phase` | Meaning |
| --- | --- |
| `disconnected` | No service, or it is stopped. Traffic uses the normal connection. |
| `connecting` | Service up, nothing has come back. **The user has no working internet in this state.** |
| `stalled` | Still connecting past when a working server would have answered. |
| `connected` | Traffic has provably crossed the tunnel. |
| `degraded` | A handshake happened, but old enough that the link may be dead. |

**How `connected` is decided, without administrator.** `wg show` reads the tunnel service's
restricted pipe, so `handshake_age_seconds` is `null` in every unelevated run and cannot be the
gate. The load-bearing signal is instead **`rx_bytes > 0`**: WireGuard discards anything it cannot
decrypt and authenticate, so the adapter's inbound octet counter only advances for payload that was
decrypted with a live session key. Non-zero inbound bytes cannot exist without a completed
handshake, and the counter comes from Win32 `GetIfTable2` — in-process, no elevation, no subprocess.

The converse does not hold: a handshake whose only traffic is a keepalive decrypts to a zero-length
payload that is never injected, so `rx_bytes == 0` can briefly outlive a real handshake. That is why
zero maps to `connecting` and a slow peer becomes `stalled` rather than an error. When the app *is*
elevated, a fresh handshake age outranks the counter.

`elapsed_seconds` is `null` for a tunnel that was already running when the app started — it has no
knowable attempt time, and such a tunnel is never reported as `stalled`, because asserting a stall
from an unknown baseline would be a lie.

Both `get_tunnel_phase` and `get_health_panel_data` compute this through the same
`classify_tunnel_phase`, so the 1 Hz and 5 s polls cannot disagree. `get_tunnel_phase` is the cheap
one — one `sc query` spawn plus an in-process counter read, no credential-store access, no HTTP, and
no database writes — and is what the connect-progress UI polls.

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
  "phase": "connected",
  "metrics_source": "adapter",
  "connected_at": "2026-08-15T10:00:00.000Z",
  "kill_switch": {
    "active": true,
    "blocks_ipv4": true,
    "blocks_ipv6": true,
    "reason": "active",
    "routes": ["0.0.0.0/0", "::/0"]
  },
  "unexpected_drop": null
}
```

`kill_switch` reports whether WireGuard's own WFP filters are blocking untunneled traffic. It is
derived from the routes the **running config** carries — `scope_allowed_ips_for_endpoint`, not the
stored `allowed_ips` column, because the two differ for a locally-scoped endpoint and the column
would claim protection a Docker dev tunnel does not have. `reason` is `active`,
`tunnel_not_running`, or `routes_scoped`.

`unexpected_drop` is set when the tunnel stopped without the app asking — traffic is unprotected
until reconnect. A user-initiated disconnect records intent first and is never reported here.
`observed_at_epoch_ms` is Unix milliseconds, unlike the ISO-8601 timestamps elsewhere in this
payload.

Takes an optional `force_ip_probe` flag. A successful egress reading is cached for 60 seconds
because health polls far more often than an address changes; a **failed** one is cached for only 5,
because the usual cause of failure is the tunnel coming up and that answer stops being true within
seconds. The cache is bypassed automatically whenever the connection state changes, and explicitly
when the user asks for a refresh. Leave the flag off for ambient polling or the app will hammer a
third-party IP service.

Five fields carry meaning in their absence, and callers must not paper over it:

| Field | `null` means | Never |
|---|---|---|
| `external_ipv4` | The egress probe failed | Substituted with `endpoint_ip`. Comparing the two is the egress check, so a stand-in would make "verified" and "never checked" identical |
| `external_ipv6` | This machine has no IPv6 egress | Treated as an error — for this deployment it is the *desired* state, see below |
| `handshake_age_seconds` | `wg show` needs administrator (see AGENTS.md §4) | Reported as `0` |
| `rx_bytes` / `tx_bytes` | No counters could be read | Reported as `0`, which would claim a measured idle tunnel |
| `connected_at` | Not connected, or the session started before the app was watching | Backfilled with the current time |

`connected_at` is stamped on the first observation where `phase` proves the tunnel carries traffic,
not when the service started — the two are seconds apart, and stamping at service start made
"Connection Time" count the outage as connected time. It is only ever stamped for a tunnel this
session started, so one found already running keeps `null` rather than showing a timer counting from
an arbitrary moment.

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
| `secret-storage` | Windows Credential Manager read/write failure | Re-save the credential in Setup |
| `invalid-input` | request the command cannot accept | Correct the input and retry |
| `dependency` | missing local prerequisite (WireGuard, Docker) | Install or start the prerequisite |
| `unknown` | unclassified exceptions | Retry and export diagnostics |
