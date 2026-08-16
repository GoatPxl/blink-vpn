# Security

## Security Objectives

- Keep secret values out of SQLite and logs.
- Keep command boundaries validated and least-privilege by design.
- Keep diagnostics export safe for sharing.
- Keep lifecycle operations recoverable under failure.

## Secret Handling

- Provider tokens are stored in Windows Credential Manager through secure storage abstraction.
- SQLite stores secret references and operational metadata.
- Tunnel private keys are handled in secure storage paths and excluded from diagnostics/log output.

## Boundary Model

- Frontend to backend: untrusted input boundary.
- Backend to provider APIs: remote boundary over TLS.
- Backend to the WireGuard tunnel service: privileged local boundary (elevation on connect and disconnect only).
- Backend to Docker runtime: local process boundary with validated command output.

## Leak Protection (Kill Switch)

**The kill switch is WireGuard for Windows', not this app's.** This app installs no firewall rules
on Windows at all. When a config has exactly one peer, an `AllowedIPs` carrying a `/0` route, and
no `Table = off`, wireguard-windows installs WFP filters that block traffic not going through the
tunnel. Every config this app generates meets those conditions for a public endpoint.

What that means when a tunnel drops:

| How it drops | Filters | Exposed? |
|---|---|---|
| Peer unreachable, network blip, roaming, sleep/wake | Service still running, filters intact | **No — fails closed.** Offline, not leaking |
| **Tunnel service stops or crashes** | Torn down with the service | **Yes.** Traffic resumes over the ISP |
| User disconnects | Removed with the service | Intended |

**Two limits to keep in mind:**

- **Docker dev tunnels have no protection.** `scope_allowed_ips_for_endpoint` strips the `/0` routes
  for loopback/RFC1918 endpoints, so the dev lane runs a split tunnel and WireGuard restricts
  nothing. The Kill Switch page reports this explicitly rather than implying parity with production.
- **A service crash is an exposure window the app detects but does not prevent.** The health poll
  classifies an unexpected stop — distinguished from a user disconnect by an intent marker recorded
  at disconnect time — and raises a visible alert with a Reconnect action. It does not block traffic
  in the meantime.

**No second firewall layer, by decision.** App-managed rules that outlive the tunnel service would
close that window, but a failure to clean them up leaves the machine with no connectivity and no
obvious remedy, and they would need elevation outside connect/disconnect (§4 of AGENTS.md). Two
mechanisms would also be the parallel-implementation drift §5 forbids. Revisit as its own task if
the residual window proves unacceptable.

## Tunnel Coverage

**The tunnel carries IPv4 only.** The server bootstrap enables IPv4 forwarding and installs
`iptables` MASQUERADE rules; there is no `ip6tables` equivalent, no IPv6 server CIDR, and the
client interface is issued a `/32` with no IPv6 address.

The client config does list `::/0` in `AllowedIPs`, which is what stops IPv6 from silently
egressing on the physical interface. That is a routing assertion, not IPv6 support: any IPv6
traffic that *does* reach the internet while connected has bypassed the tunnel.

The dashboard probes IPv6 egress in every state and reports a reachable address while connected as
a bypass rather than as a second protected route. Treat a persistent warning there as a real
finding. Adding genuine IPv6 support is a bootstrap and addressing change, not a UI change.

The egress probe contacts a third-party IP service (`ipify` / `icanhazip`) in both connected and
disconnected states, so that service observes the user's real address while disconnected. This is
inherent to measuring egress; it is cached for 60 seconds and re-probed on a connection change, so
it is not issued on every poll.

## Redaction Policy

Always redact:

- bearer tokens
- authorization headers
- provider token references
- API token fields
- private keys

Rules:

- Never emit raw credential material in logs.
- Never include secret fields in diagnostics export.
- Keep redaction logic in backend serialization and logging helpers.

## Secure Coding Requirements

- Validate command inputs before state mutation.
- Preserve idempotent cleanup semantics for destructive operations.
- Avoid introducing plaintext secret persistence.
- Prefer explicit, typed error classes for UI guidance.

## Incident Handling

1. Pause provisioning/destroy operations.
2. Rotate affected provider credentials.
3. Remove stale secure-storage references.
4. Export diagnostics and capture runtime logs.
5. Reconcile and cleanup local state.
6. Resume operations after verification.
