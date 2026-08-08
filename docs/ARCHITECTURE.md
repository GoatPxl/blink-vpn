# Architecture

## Overview

GoatStare is a desktop control plane for running user-owned WireGuard VPN infrastructure through provider adapters and a unified NodeProvider interface.

Stack:

- Next.js + React frontend for operator workflows
- Tauri command bridge for typed frontend/backend calls
- Rust backend for orchestration, provider adapters, lifecycle state, and diagnostics
- SQLite for local metadata and runtime snapshots
- Windows Credential Manager for secret storage

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
  ORCH --> WG[WireGuard runtime]
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
- WireGuard manager: import, activate, deactivate, remove local tunnel interfaces.

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
- Local runtime boundary: WireGuard and Docker command output is treated as untrusted process output.

## Data Flow

1. Register and validate provider credentials.
2. Fetch region and pricing catalog.
3. Provision node and wait for readiness.
4. Generate/import/activate client tunnel.
5. Poll health and diagnostics.
6. Disconnect, destroy, or force-cleanup as required.

## Reliability Patterns

- Idempotent cleanup semantics for partial failures.
- Startup reconciliation for stale local/remote state.
- Explicit recovery flows in the UI for provider and WireGuard failures.
