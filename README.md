# GoatStare VPN

GoatStare VPN is a Windows desktop VPN control plane built with a Rust backend, Tauri, and a Next.js frontend.

High-level goal:

- Let an end user create and operate their own WireGuard VPN server on a VPS provider with minimal infrastructure knowledge.
- User flow should be: add provider API key -> choose location/plan -> app provisions server -> app configures WireGuard -> user connects securely.

Project inspiration:

- https://www.digitalocean.com/solutions/vpn

## Current Capability Profile

- Windows desktop workflow driving the WireGuard for Windows tunnel service
- Rust backend + Tauri + Next.js frontend architecture
- Multi-provider adapter model
- DigitalOcean cloud adapter and Docker local adapter
- Secure storage for provider tokens and client keys
- SQLite for provider, node, tunnel, and runtime metadata

## What It Does Today

- Stores and validates provider credentials in Windows Credential Manager (no plaintext secrets in SQLite).
- Uses a generalized provider orchestration path for node lifecycle operations.
- Provisions a VPS-like node, waits for readiness, and applies WireGuard bootstrap flows.
- Creates and manages local WireGuard client tunnels through the WireGuard for Windows service.
- Shows health, diagnostics, and recovery guidance in the dashboard.
- Supports conservative destroy and force cleanup for interrupted operations.

## Dev Docker Provider

The Docker provider supports local development and repeatable integration testing.

- Enabled by backend flag: `GOATSTARE_DEV_DOCKER=1`.
- Exposed in frontend development UX when `NEXT_PUBLIC_DEV_DOCKER=1`.
- Uses local Docker containers to emulate provider node provisioning behavior.
- Returns deterministic synthetic provider plan and account data for local workflows.

This allows testing the provider abstraction path without requiring a live cloud account.

## Quickstart

Prerequisites:

- Windows 10/11
- Node.js and npm
- Rust toolchain
- WireGuard for Windows installed locally
- WireGuard for Windows installed (provides `wireguard.exe` and `wg.exe`)
- Provider API key for production-style testing (DigitalOcean)
- Docker Desktop (only when testing dev Docker provider)

Run locally:

```bash
npm install
npm run tauri:dev
```

Run real desktop E2E lane:

```bash
npm run frontend:e2e
```

Run Tauri app with Playwright plugin enabled for native-webview E2E work:

```bash
npm run tauri:dev:e2e
```

Run real Docker backend adapter integration test (no mock fixtures):

```bash
npm run backend:test:docker:real
```

Run real Tauri Docker flow E2E (no mock fixtures):

```bash
npm run tauri:e2e:docker:real
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Windows setup](docs/SETUP_WINDOWS.md)
- [Operations runbook](docs/OPERATIONS_RUNBOOK.md)
- [Security](docs/SECURITY.md)
- [Testing](docs/TESTING.md)
- [Tauri command contracts](docs/API_COMMANDS.md)
- [Database schema](docs/DB_SCHEMA.md)
- [Release checklist](docs/RELEASE_CHECKLIST.md)
- [Operational guarantees](docs/OPERATIONAL_GUARANTEES.md)
- [Product roadmap](docs/ROADMAP.md)
- [Contributing](docs/CONTRIBUTING.md)