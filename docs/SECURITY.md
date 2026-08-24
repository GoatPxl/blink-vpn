# Security

## Security Objectives

- Keep secret values out of SQLite and logs.
- Keep command boundaries validated and least-privilege by design.
- Keep diagnostics export safe for sharing.
- Keep lifecycle operations recoverable under failure.

## Where Credentials Live

Provider API keys are stored in **Windows Credential Manager**, under the service name
`com.goatstare.vpn`, written as a generic credential. SQLite holds only a non-secret
`token_ref` handle pointing at that entry — never the key.

A credential is a **set of named fields**, serialized to a JSON object inside that single entry:
DigitalOcean and Linode store `api_token`, Amazon EC2 stores `access_key_id` and
`secret_access_key`, and Google Compute Engine stores `service_account_json` — the whole downloaded
key file as one field, from which the adapter reads the project id, the service account address and
the RSA signing key. Asking separately for a project id that is already in the file only invites a
mismatch that surfaces later as a 404 on a project that does exist. Only the
provider's own adapter interprets them, so the schema never learns that some providers need two
values. A stored value that is not a JSON object is read as a single `api_token` field, which is how
credentials saved by earlier versions keep working.

OpenStack stores three fields: `auth_url`, `application_credential_id` and
`application_credential_secret`. Application credentials are asked for rather than a username and
password because they are scoped to one project, independently revocable, and carry no domain or
project name for the user to get wrong.

Redaction covers `secret_access_key`, `session_token`, `service_account_json`, `assertion` (the
signed JWT sent to Google's token endpoint), `application_credential_secret` and `X-Subject-Token`
(the issued Keystone token) alongside `api_token`. `access_key_id`, `client_email`, `project_id`,
`auth_url` and `application_credential_id` are deliberately left readable: they identify which
credential and which cloud are in use, which is a real diagnostic, and none of them is a secret.

**The RSA key inside a Google key file is matched by shape, not by field name.** Google calls it
`private_key` — the same name WireGuard client configs use for material this app deliberately does
not redact (see below). Redacting that name would blank the one line most worth reading when a
tunnel comes up wrong. The two are unambiguous by content instead: a service account key is PEM, a
WireGuard key is a bare base64 blob that never carries PEM markers, so any `BEGIN…PRIVATE KEY`
block is replaced wherever it appears. Certificates are left intact.

Two qualifications worth stating rather than glossing:

- The app performs **no encryption of its own**. Protection at rest is Credential Manager's, which
  means the credential is readable by anything running as the same Windows user.
- On non-Windows builds the store is `MockSecretStore`, an in-process `HashMap` that is neither
  encrypted nor persistent. It exists for tests and must not be treated as a secure store.

The UI names Credential Manager rather than claiming the keys are "encrypted at rest", because
knowing *where* the credential is, is what lets a user go and revoke it.

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

## Server Hardening

The bootstrap payload (migration-0021-era nodes onward) hardens the server it creates. All of it
is baked at create time — provisioning is write-only (AGENTS.md §2), so none of it can be adjusted
on a running node.

- **Ingress is default-drop.** The only listener is WireGuard's UDP port. There is no SSH rule and
  no sshd: the bootstrap disables the service outright, since nothing ever dials in. ICMP echo is
  answered but rate-limited per source. UFW, if the provider image shipped it enabled, is reset so
  exactly one thing writes firewall rules.
- **VPN peers cannot reach the cloud metadata service.** A FORWARD drop for 169.254.0.0/16 keeps a
  device on the tunnel from querying the instance's IMDS (169.254.169.254) through the exit.
- **Peers are isolated from each other.** A subnet-to-subnet FORWARD drop backs up WireGuard's
  per-peer AllowedIPs routing.
- **NAT is pinned to the default-route interface**, resolved at boot — unpinned masquerade picks
  the wrong interface on multi-homed instances.
- **Every peer has a per-peer preshared key**, generated on the Windows host with the keypair and
  baked into the server config. PSKs are WireGuard key material, not secrets (§6): they live in
  SQLite and travel inside exported configs and QR codes, because a client without its PSK cannot
  complete a handshake at all.
- **Security patches apply themselves, and reboot the box.** unattended-upgrades runs with
  `Automatic-Reboot "true"` (04:00 server time): no operator ever logs in, so a kernel patch that
  waited for a manual reboot would wait forever. The tunnel and firewall are reboot-safe —
  `wg-quick@` is enabled and re-applies forward/NAT rules via PostUp, and the ingress chain is
  restored by `netfilter-persistent`. The reboot itself is a brief tunnel drop, documented in the
  operations runbook.

Nodes provisioned before these changes keep the server they were created with; the upgrade path is
destroy-and-recreate, same as every other server-side change.

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
- API token fields (`api_token`, `secret_access_key`, `session_token`)

Never redacted:

- `access_key_id` — an identifier, not a secret, and knowing which key is in use is a real
  diagnostic.
- **WireGuard key material.** A server exists for hours and is then destroyed, and its keys die
  with it. Client configs, including `PrivateKey`, may be written to files, shown on screen,
  encoded into QR codes and included in diagnostics. Redacting the key removed the one line most
  worth reading when a tunnel came up wrong, and a config a phone can import has to carry it.

Rules:

- Never emit provider credential material in logs.
- Never include provider credential fields in a diagnostics export.
- Keep redaction logic in backend serialization and logging helpers, applied at the **write**
  boundary so nothing downstream has to remember to redact on read.

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
