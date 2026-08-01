# GoatStare VPN

GoatStare VPN is a Phase 1 Windows desktop VPN manager built with a Rust backend, Tauri, and a Next.js frontend. It provisions and manages a single-provider WireGuard workflow on DigitalOcean, stores secrets locally, and keeps observability intentionally local-only.

## Phase 1 Scope

- Windows desktop only
- DigitalOcean only
- WireGuard only
- Rust backend + Tauri + Next.js frontend
- Local secure storage for provider tokens and client keys
- SQLite for metadata, node state, tunnel state, and runtime snapshots

## What It Does

- Store and validate a DigitalOcean API token in Windows Credential Manager.
- Provision a droplet and bootstrap WireGuard on the remote host.
- Create, connect, disconnect, and inspect local client tunnels.
- Surface node health, tunnel status, and recovery guidance in the UI.
- Destroy nodes and perform explicit force cleanup when provisioning is interrupted.

## Architecture Snapshot

- [Rust backend](docs/ARCHITECTURE.md) owns orchestration, persistence, secret handling, WireGuard integration, and provider access.
- [Tauri commands](docs/API_COMMANDS.md) expose a small typed command surface to the UI.
- [Next.js dashboard](src/components/vpn-dashboard.tsx) is a state-driven control plane for provisioning, tunnel control, health, diagnostics, and recovery.
- [SQLite schema](docs/DB_SCHEMA.md) stores providers, nodes, tunnels, and runtime snapshots.

## Quickstart

Prerequisites:

- Windows 10/11
- Node.js and npm
- Rust toolchain
- WireGuard for Windows installed locally
- DigitalOcean API token with permission to manage droplets

Run locally:

```bash
npm install
npm run tauri:dev
```

The UI starts at `http://localhost:3000` in development, with the Rust backend launched by Tauri.

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Windows setup](docs/SETUP_WINDOWS.md)
- [Operations runbook](docs/OPERATIONS_RUNBOOK.md)
- [Security](docs/SECURITY.md)
- [Testing](docs/TESTING.md)
- [Tauri command contracts](docs/API_COMMANDS.md)
- [Database schema](docs/DB_SCHEMA.md)
- [Release checklist](docs/RELEASE_CHECKLIST.md)
- [Known limitations](docs/KNOWN_LIMITATIONS.md)
- [Phase 2 roadmap](docs/ROADMAP_PHASE2.md)
- [Contributing](docs/CONTRIBUTING.md)

## Current Release Posture

- Implemented now: local secrets, provision/connect/disconnect/destroy, health, diagnostics, force cleanup, and redacted logs.
- Deferred to later phases: multi-provider support, non-Windows targets, telemetry, and advanced VPN features.

## Assumptions

- TODO: project-specific fill for CI/CD signing and distribution policy.
- TODO: project-specific fill for installer packaging automation if it differs from the current NSIS bundle.