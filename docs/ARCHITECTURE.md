# Architecture

## Overview

GoatStare is a desktop control plane for running user-owned WireGuard VPN infrastructure through provider adapters and a unified NodeProvider interface.

Stack:

- Next.js + React frontend for operator workflows
- Tauri command bridge for typed frontend/backend calls
- Rust backend for orchestration, provider adapters, lifecycle state, and diagnostics
- SQLite for local metadata and runtime snapshots
- Windows Credential Manager for secret storage
- DefGuard-managed WireGuard runtime on the machine running the app, backed by `wireguard-nt` and a single connection-mode setting.

Included provider adapters:

- DigitalOcean adapter for cloud VPS lifecycle
- Docker adapter for local container-backed development workflows

## System Diagram

```mermaid
flowchart LR
  UI[Next.js dashboard] -->|invoke| CMD[Tauri commands]
  CMD --> ORCH[Orchestrator]
  ORCH --> REG[Provider registry]
  REG --> NP[Unified NodeProvider]
  NP --> DO[DigitalOcean adapter]
  NP --> DKR[Docker adapter]
  ORCH --> DB[(SQLite)]
  ORCH --> SEC[Credential Manager]
  ORCH --> RUNTIME[DefGuard WireGuard runtime]
  DO --> DOAPI[DigitalOcean API]
  DKR --> DKRCLI[Docker CLI]
```

## Core Responsibilities

- Frontend: user workflows for provider, node, tunnel, health, and diagnostics.
- Command layer: typed boundary and error mapping.
- Orchestrator: lifecycle transitions, idempotency, reconciliation, cleanup.
- Provider registry: adapter resolution by provider slug and NodeProvider construction.
- NodeProvider: provider-neutral lifecycle contract for init, bootstrap readiness, status, existence checks, and delete.
- Provider adapters: low-level provider implementation details for validate, catalog, create, wait, and delete.
- WireGuard manager: generate keys, configure local tunnels through the DefGuard runtime, activate/deactivate connections, and clean up local interfaces.
- Key management: generate and store key material on the app host; never generate client keys on remote nodes.

## Connection Modes

- Settings exposes one WireGuard mode toggle: `Admin Mode` or `User Mode`.
- `Admin Mode` uses a privileged virtual adapter workflow and can require Windows elevation prompts during tunnel creation or cleanup.
- `User Mode` uses the installed WireGuard for Windows service path without creating an app-managed virtual adapter.
- Both modes flow through the same typed Tauri/backend contracts; the selected mode only changes how the local runtime is configured.

## Global App Settings

Desktop-wide runtime settings are persisted in SQLite through a dedicated `app_settings` table owned by the Rust backend.

Rules:

- The backend is the source of truth for global runtime settings used during tunnel lifecycle operations.
- The frontend reads and updates these settings only through typed Tauri commands.
- Settings must be additive, versioned through migrations, and safe for existing installs.
- Secret values must not be stored in app settings.

Initial settings in this category:

- `wireguard_connection_mode`: `admin` or `user`

Behavior:

- Default value is `admin`.
- Mode changes are blocked while any tunnel is connected.
- Tunnel connect and disconnect flows read the current persisted mode at execution time.

## Unified NodeProvider Contract

The orchestrator and command layer operate against a shared NodeProvider abstraction instead of reaching directly into provider-specific execution paths.

The contract covers:

- `init`: provision a node, persist the bootstrapping context, and return a bootstrap contract preview.
- `wait_for_bootstrap_ready`: wait for provider-specific readiness markers and resolve the bootstrap output contract.
- `get_status`: report the provider-level node status in a normalized form.
- `exists`: check whether the node still exists.
- `delete`: tear down the provider-side node resource.

The registry resolves the correct provider adapter, loads the provider token from secure storage, and presents that provider through the shared NodeProvider interface to the orchestrator.

## Lifecycle Model

### Node states

- `new`
- `provisioning`
- `bootstrapping`
- `ready`
- `connected`
- `error`
- `deleting`

### Tunnel states

- `new`
- `ready`
- `connected`
- `disconnected`
- `error`
- `deleting`

State changes are validated in backend transition guards.

## Trust Boundaries

- Frontend input boundary: all command inputs are treated as untrusted.
- Secret boundary: secure storage keeps secret values out of SQLite.
- Provider boundary: remote API responses are validated and normalized.
- Local runtime boundary: DefGuard/WireGuard runtime and Docker command output is treated as untrusted process output.

## Data Flow

1. Register and validate provider credentials.
2. Fetch region and pricing catalog.
3. Verify the DefGuard-managed WireGuard runtime prerequisites are available on the app host.
4. Generate WireGuard keypair on the app host (where the app resides) and persist secrets locally.
5. Provision node with an init script that receives and applies the generated public key.
6. Wait for bootstrap readiness and verify node-side WireGuard configuration.
7. Configure and activate the client tunnel locally using the selected connection mode.
8. Poll health and diagnostics.
9. Disconnect, destroy, or force-cleanup as required.

## Key Generation And Bootstrap Requirements

- Key generation location: WireGuard key generation must happen on the same machine where the app runs.
- Host dependency: the app host must provide the DefGuard runtime DLL resources and any Windows WireGuard service dependencies required by the selected connection mode.
- Private key handling: private key material remains local to the app host and secure storage boundary.
- Public key propagation: the generated public key is injected into the provider/node init script payload.
- Node bootstrap contract: init script must set the public key in node WireGuard config during first boot.
- Verification: bootstrap readiness must include confirmation that the node has applied the expected public key.

## Reliability Patterns

- Idempotent cleanup semantics for partial failures.
- Startup reconciliation for stale local/remote state.
- Explicit recovery flows in the UI for provider and local WireGuard runtime failures.
