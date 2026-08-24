# AGENTS.md

Operating rules for AI agents working on GoatStare VPN.

This file encodes invariants that were learned expensively. Most rules below exist because the
codebase already violated them once and the project stalled as a result. Where a rule has a
non-obvious reason, that reason is stated — read it before deciding the rule doesn't apply to you.

---

## 1. What this project is

A Windows desktop app that provisions a user's own WireGuard VPN server on a cloud provider and
connects to it. Rust backend + Tauri v2 + Next.js frontend + SQLite + Windows Credential Manager.

**Current scope.** This table describes what the code actually supports, not a lock.

| Axis | In scope |
|---|---|
| Platform | Windows 10/11 desktop only |
| Protocol | WireGuard only |
| Production providers | DigitalOcean, Amazon EC2, Linode, Google Compute Engine, OpenStack (experimental) |
| Dev provider | Docker, behind feature flags, local emulation only |
| Workflows | provision, connect, disconnect, destroy, force-cleanup, add device |

Multi-provider *architecture* is in scope. Additional provider *implementations* are not, until
asked for.

Platform expansion is **not** in scope and nothing here moves toward it. Android and iOS are served
instead by handing out a device peer for the official WireGuard app (§13) — the app provisions the
server, the phone runs a client someone else audits.

---

## 2. The provisioning contract — the most important rule in this file

**Provisioning is write-only.**

1. All key material is generated on the Windows host, before the node is created.
2. Everything the server needs — its private key, the client's public key, the full interface and
   peer config — is baked into a single payload and handed to the provider at create time.
3. After create, the app may only poll **provider metadata**: status, and the endpoint to connect to.

That is the entire contract. The app never talks to the server it provisioned.

**Forbidden, without exception:**

- SSH, or any other remote shell into a provisioned node.
- `docker exec` / `docker cp` anywhere in the provisioning path.
- Reading any file back off the server, including the readiness marker.

**Why.** The original design had the server generate its own keys and the app read them back from
`/var/lib/goatstare/bootstrap-ready.json`. Reading a file off a fresh droplet requires SSH, which
the app does not have and should not gain. So the DigitalOcean adapter shipped a hardcoded
`Unsupported` stub at exactly that step — meaning **every production provision created a billable
droplet, failed, and rolled back**. Meanwhile the Docker adapter faked its way past the same step
with `docker exec`, so the dev lane went green while proving nothing. That single design choice
cost the project months. Do not reintroduce a read-back channel; if a feature appears to need one,
the design is wrong — stop and raise it.

The readiness marker still gets written by the bootstrap script. It is a **self-check and debugging
aid only**. Nothing in the app may depend on reading it.

---

## 3. Provider neutrality

- **No `if provider.slug == "..."` outside an adapter.** Ever. That branch is precisely how the
  Docker path drifted until it shared no meaningful code with production.
- Adapters report their own endpoint (host + port). The orchestrator must not know that Docker,
  or `127.0.0.1`, exists.
- **Docker receives the same generated payload DigitalOcean receives.** Delivery differs — cloud-init
  `write_files` versus a create-time bind mount — but the script itself is identical. If the two
  renderers diverge, the dev lane has stopped being evidence.
- Capability differences belong in the adapter, surfaced as explicit capability errors, not as
  special cases upstream.

---

## 4. WireGuard on Windows: one backend, one privilege model

**The only supported mechanism is the WireGuard for Windows tunnel service:**

| Operation | Command | Elevation |
|---|---|---|
| connect | `wireguard.exe /installtunnelservice <conf>` | 1 UAC prompt |
| status | `sc query WireGuardTunnel$<name>` | none |
| stats | `wg show <name> dump` | **required** — best-effort only |
| disconnect | `wireguard.exe /uninstalltunnelservice <name>` | 1 UAC prompt |

**`wg show` needs administrator.** It talks to the tunnel service's pipe, which is restricted, so
an unelevated app gets `Unable to access interface: Permission denied`. Peer metrics are therefore
enrichment, never load-bearing: whether a tunnel is up is decided by `sc query`, which needs no
elevation. Treating a failed metrics read as fatal made the app report an error for a tunnel that
had installed, started, and completed a handshake correctly.

The tunnel config passed to `/installtunnelservice` must **outlive the import**. The path is
recorded in the service's command line and read on every start; it is not copied. Configs live
under `%LOCALAPPDATA%\GoatStare\tunnels\` and are removed when the tunnel is unregistered.

**Do not reintroduce the `defguard_wireguard_rs` runtime path or `wireguard-nt`.** They were tried
and reverted. Three independent reasons, all still true:

- `wireguard_nt::Adapter` closes the adapter in `Drop`. Any `WGApi`/`Adapter` created inside a
  function is destroyed when that function returns, so a tunnel brought up this way cannot survive
  the call. Nothing holds a handle across calls.
- `WGApi::new()` sets `adapter: None` and `read_interface_data()` returns `AdapterNotFound` unless
  `create_interface()` was called on that same instance. Status reads through this path always fail.
- defguard loads `wireguard.dll` from a hardcoded **CWD-relative** path behind a `LazyLock` with
  `.expect()`. An installed app's CWD is neither stable nor writable, and once that `LazyLock`
  panics it stays poisoned for the process lifetime.

`defguard_wireguard_rs` is retained **for key generation only** — it is pure Rust and avoids
requiring `wg.exe` on PATH just to provision.

**Elevation rules:**

- Elevate for connect and disconnect only. Nothing else.
- Try direct first; escalate only on access-denied.
- Connect and disconnect must be **symmetric** — a disconnect that cannot elevate strands the user
  in a connected state.
- Never prompt for UAC from inside a polling loop.
- Never elevate to read status. `sc query` works unelevated.
- **There is exactly one privilege model.** No admin/user mode setting. The previous toggle was
  fully wired through migration, DB, commands and UI, and was behaviorally a no-op in both
  directions — it only ever added confusion.

**A running service is not a working tunnel.** The adapter captures the default route the instant
its service starts, so from that moment the user's traffic is inside a tunnel whose peer has not
answered — they have *no internet* until the handshake completes. Measured: service running at
`10:22:07.851`, first handshake response at `10:22:23.051`. The UI reported "Connected" for that
entire fifteen seconds because it keyed off `sc query`.

Nothing may present a tunnel as connected on service state alone. The verdict is `TunnelPhase`
(`backend/orchestrator/tunnel_phase.rs`), and because `wg show` needs administrator, it turns on
evidence available unelevated: **adapter inbound octets > 0**. WireGuard discards anything it cannot
decrypt, so non-zero inbound bytes cannot exist without a completed handshake. `connect_tunnel`
returning still means only "the service started" — that is why it no longer announces success.

While the tunnel is coming up the user is watching their internet fail, so that window must be
explained on screen, and the app must stay responsive through it. Both were broken by the same
root cause: **every Tauri command ran synchronously on the UI thread.**

**Commands run on the blocking pool, via `run_blocking` in `commands/system.rs`.** Not on the UI
thread, and — this part was learned the hard way — not on the async runtime either.
`#[tauri::command(async)]` schedules the body on a tokio *async* worker, and `reqwest::blocking`
(the DigitalOcean client and the egress probe) builds its own runtime internally; dropping one
inside an async context panics with *"Cannot drop a runtime in a context where blocking is not
allowed"*. That panic killed the IPC channel, so **every command in the app failed at once** — an
empty dashboard, no nodes, no settings. Keep new commands on `run_blocking`; the bodies are all
blocking work (subprocesses, sleeps, UAC, SQLite, Credential Manager, HTTP) and the blocking pool is
where that is legal.

The other consequence is that commands genuinely run concurrently now, where the UI thread used to
serialize them by accident. Anything that was implicitly safe under that serialization no longer is:
SQLite sets a `busy_timeout`, `create_provider` is `INSERT OR IGNORE` because four commands race to
ensure the built-in rows, and connect holds a per-tunnel in-flight guard. Assume no serialization
when adding a check-then-write.

---

## 5. No parallel implementations

When you replace a mechanism, **delete the old one in the same change.**

This repo accumulated three tunnel backends and roughly fourteen unreachable functions because each
migration left its predecessor in place "just in case". The result was a 2,800-line module where the
only way to learn which path executed was to trace four levels of dispatch.

- A `dead_code` warning is a bug, not noise.
- No commented-out blocks, no unreferenced alternates, no `_v2` siblings.
- If you genuinely need to keep a fallback, it must be reachable, tested, and documented as to when
  it fires.

---

## 6. Secrets

One thing in this app is a secret: the **provider API credential**. Everything else follows from
that distinction, so make it before reaching for redaction.

**Provider credentials — DigitalOcean tokens, AWS access keys.** Long-lived, account-wide, and able
to create billable resources. A leaked one costs the user money and outlives any server it made.

- **Never** in SQLite, logs, error messages, diagnostics bundles, test fixtures, snapshots, or any
  committed file. SQLite stores a `*_ref` pointer; the secret lives in Windows Credential Manager.
- Redacted at the **write** boundary — `ActivityRecorder::record`, `CommandError::new` — so nothing
  downstream has to remember to redact on read. The list lives in `redact_sensitive_text`:
  `api_token`, `secret_access_key`, `session_token`, `token_ref`, `Authorization`, bearer tokens.
- `access_key_id` is deliberately **not** redacted. It identifies which key is in use, which is a
  real diagnostic, and it is not a secret.
- **A live DigitalOcean API token was previously committed** to `tests/e2e/fixtures/tauri-mocks.ts`,
  disguised as a fake `token_ref`. It was removed from the tree but remains in git history. Fake
  credentials in fixtures must be *obviously* fake (`dop_v1_EXAMPLE_NOT_A_REAL_TOKEN`), never a
  real value with the label changed.

**WireGuard key material is not a secret here.** Server and client private keys, and whole client
configs, may be written to files, shown on screen, encoded into QR codes, copied to the clipboard,
and included in diagnostics. Do not add redaction for them, and do not treat their appearance in
output as a defect.

The reason is lifetime. A GoatStare server is provisioned on demand, lives for hours, and is
destroyed; its keys are worthless the moment it is gone, and they grant access to nothing except
that one ephemeral box. Against that, redaction cost real things: it blanked the `PrivateKey` line
in the config snapshot — the one line that says *which peer is actually loaded* — and a client
config that a phone can import must carry its key, so device peers (§13) would be impossible.

What stays true regardless: private keys still live in Windows Credential Manager rather than
SQLite, because a `*_ref` indirection is how the schema is built, not because of a threat model.

- Treat all external input as untrusted, including Docker CLI output and provider API responses.

If a change risks exposing a **provider credential**, stop and propose a safer alternative.

---

## 7. Errors cross the IPC boundary structured

Tauri commands return `{ kind, message, next_action }` — never a bare formatted string.

**The frontend must never classify errors by matching on backend message text.** It used to, with
`String.includes` checks against literals like `"OpenService FAILED 1060"` and
`"Provider orchestration failed"`. Every backend copy edit silently broke UI recovery paths.

Error messages are for humans and must say what failed, the likely cause, and the next recovery
action. Control flow keys off `kind`.

---

## 8. Coding standards

**General.** Prefer clarity over cleverness. Small, single-purpose functions. Explicit types.
No placeholder TODO logic unless explicitly marked and justified.

**Rust.**
- Idiomatic `Result` + `thiserror`. **No `unwrap()`/`expect()` in production paths.**
- Module boundaries: orchestrator · provider registry + adapters · bootstrap · wireguard ·
  secure storage · db/repository · logging/diagnostics · crypto.
- Provider-specific behavior stays inside adapter modules (see §3).
- Bounded retries with exponential backoff + jitter for transient failures; explicit timeouts.
- Idempotent provision/connect/disconnect/destroy where practical, with safe rollback on partial
  failure. **Destroy must report real failures** — never swallow a failed remote delete and return
  success; that leaves the user paying for an invisible droplet.

**Frontend.**
- Typed bindings to Tauri commands in `src/lib/tauri.ts`. Bindings are hand-written, so when a Rust
  type changes, update the TS type in the same change — nothing enforces it.
- Handle loading, success, and error states explicitly.
- Never persist secrets to browser-like storage.

**Dependencies.** Minimize. For each addition state: why needed, maintenance/security rationale,
alternative considered. Prefer deleting two dependencies over adding one.

---

## 9. Verification

Run what your change touches. Report **real results**, including failures.

| Change scope | Commands |
|---|---|
| Frontend only | `npm run typecheck` · `npm run lint` |
| Rust backend only | `cargo test --manifest-path src-tauri/Cargo.toml` |
| Cross-boundary | both of the above |
| Docker provider / tunnel lifecycle | `npm run backend:test:docker:real` (requires Docker) |
| UI or command surface | `npm run tauri:e2e:provisioning` (requires Docker) |
| Connect / disconnect | `npm run tauri:e2e:connect` (requires Docker **and an elevated shell**) |
| DigitalOcean adapter or cloud-init | `npm run backend:test:do:real` (**creates billable droplets**) |
| Amazon EC2 adapter or cloud-init | `npm run backend:test:ec2:real` (**creates billable instances**) |
| Linode adapter or cloud-init | `npm run backend:test:linode:real` (**creates billable instances**) |
| Google Compute Engine adapter or cloud-init | `npm run backend:test:gce:real` (**creates billable instances**) |
| OpenStack adapter or cloud-init | `npm run backend:test:openstack:real` (**manual; billable on a commercial cloud**) |

The Linode lane needs `GOATSTARE_LINODE_TEST_TOKEN`; the GCE lane needs
`GOATSTARE_GCE_TEST_SERVICE_ACCOUNT_FILE`, a **path** to the JSON key file rather than its contents.
The OpenStack lane needs `GOATSTARE_OPENSTACK_TEST_AUTH_URL`, `..._APPLICATION_CREDENTIAL_ID`,
`..._APPLICATION_CREDENTIAL_SECRET` and `..._FLAVOR`, and has no default cloud to point at.
The DigitalOcean lane needs `GOATSTARE_DO_TEST_TOKEN`; the EC2 lane needs
`GOATSTARE_AWS_TEST_ACCESS_KEY_ID` and `GOATSTARE_AWS_TEST_SECRET_ACCESS_KEY`. Every one of these
tests destroys what it created on every exit path, including panics, and asserts only after teardown
so a failed assertion cannot strand a paid resource. Keep it that way.

EC2 creates **two** billable things — an instance and a public IPv4 — plus a security group that
cannot be deleted until the instance releases its network interface. Its teardown waits for that and
reports a group it could not remove rather than swallowing it.

Required test types for any non-trivial change:

- **Unit** tests for pure logic — state transitions, parsers, config generation.
- **Integration** tests for repositories and adapters, using mocks/fakes for network and process calls.
- **Error-path** tests for retries, timeouts, and failure handling.
- **Redaction** tests proving secrets cannot reach logs or diagnostics.

Rules: deterministic only, no real external API calls in the normal suite, both happy path and key
failure paths.

**A test that cannot fail is worse than no test.** The E2E suite once waited for
"the expected message *or* any error title", so it passed whether the app worked or not — a run
that provisioned nothing and connected nothing looked identical to a healthy one. Never accept an
error state as a success condition, and never assert something that is true for almost any input.
Assert the specific outcome, and prefer backend state over UI text where both are available.

The E2E lane drives the WebView2 devtools protocol directly (`--remote-debugging-port`). There is
no Tauri test plugin; `tauri-plugin-playwright` and its `e2e-testing` feature were removed because
nothing reached them. Do not reintroduce a second harness.

---

## 10. Working protocol

- Start non-trivial work by restating scope, constraints, and affected components.
- Ask before editing when behavior, state transitions, security posture, or release risk is
  genuinely ambiguous — not for choices with an obvious default.
- Before an architectural decision (new dependency, schema change, API contract change, directory
  restructure), consult [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). If it has no guidance, present
  2–3 options with trade-offs and the exact doc diff for each, and wait for a decision.
- Report: what changed, what was verified, what was **not** verified, and residual risk. Use
  concrete file paths.
- If a task is incomplete, say so explicitly and say why. Scaling work down is the user's call.

**Docs must stay true.** When you change behavior, update the doc in the same commit; when you find
a false claim, fix or delete it rather than working around it.

Two such claims — a `GOATSTARE_WG_RUNTIME` env var with no code reference, and a required DLL at
`resources-windows/binaries/wireguard-amd64.dll` — have since been removed from the docs. This
paragraph kept naming them after the fact, which made *this* file the stale one.

---

## 11. Feature flags

| Flag | Side | Effect |
|---|---|---|
| `GOATSTARE_DEV_DOCKER` | backend | Enables the Docker dev provider |
| `NEXT_PUBLIC_DEV_DOCKER` | frontend | Exposes Docker dev surface in the UI |

The backend flag also auto-reads `.env.local`, `.env`, and `src-tauri/.env`, so it may be on without
being set in your shell. Docker is **never** a production security boundary.

---

## 12. Device peers

A server serves more than one device. This is the only reason multi-peer support exists, and it is
shaped entirely by §2: **provisioning is write-only, so every peer a server will ever have must be
baked into its cloud-init payload at create time.** There is no channel to register one afterwards,
and adding one is not a feature request — it is a request to break the contract in §2.

The consequences follow directly:

- Node create generates `DEFAULT_PEER_SLOTS` keypairs, not one, and bakes every public key into the
  server config. Slot 0 is the Windows desktop; the rest are spare.
- "Add a device" is a **local claim** against an already-baked slot. It contacts nothing.
- A node provisioned before multi-peer support has one peer and cannot gain more. It must be
  destroyed and recreated. The UI hides the device section for such a node rather than offering
  something that cannot work.
- Running out of slots is a destroy-and-recreate, by design. Raising `DEFAULT_PEER_SLOTS` only
  affects servers created afterwards.

The server NAT rule must masquerade the **tunnel subnet**, not one `/32`. It was `-s ${CLIENT_CIDR}`
while only one peer existed; a second peer under that rule completes a handshake and then has no
internet, which looks like a client bug and is not one.

Mobile devices run the **official WireGuard app**, imported by QR code. The app does not implement a
tunnel for any platform other than Windows (§4) and there is no plan to — see §1. A client config is
only importable if it carries its `PrivateKey`, which is why §6 declassifies WireGuard key material.

---

## 13. Reference

**Project docs:** [README.md](README.md) ·
[ARCHITECTURE.md](docs/ARCHITECTURE.md) ·
[SETUP_WINDOWS.md](docs/SETUP_WINDOWS.md) ·
[OPERATIONS_RUNBOOK.md](docs/OPERATIONS_RUNBOOK.md) ·
[TESTING.md](docs/TESTING.md) ·
[SECURITY.md](docs/SECURITY.md) ·
[API_COMMANDS.md](docs/API_COMMANDS.md) ·
[DB_SCHEMA.md](docs/DB_SCHEMA.md)

**External:**
[Tauri v2](https://v2.tauri.app/) ·
[Next.js](https://nextjs.org/docs) ·
[React](https://react.dev/reference/react) ·
[Rust](https://doc.rust-lang.org/book/) ·
[WireGuard](https://www.wireguard.com/quickstart/) ·
[DigitalOcean API](https://docs.digitalocean.com/reference/api/) ·
[Docker](https://docs.docker.com/) ·
[Playwright](https://playwright.dev/docs/intro) ·
[SQLite](https://www.sqlite.org/docs.html)

> **Next.js note:** this project uses a Next.js version with breaking changes from common training
> data. Read the relevant guide under `node_modules/next/dist/docs/` before writing frontend code.
