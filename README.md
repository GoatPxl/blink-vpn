# Blink VPN

Blink VPN is a Windows desktop VPN control plane built with a Rust backend, Tauri, and a Next.js frontend.

High-level goal:

- Let an end user create and operate their own WireGuard VPN server on a VPS provider with minimal infrastructure knowledge.
- User flow should be: add provider API key -> choose location/plan -> app provisions server -> app configures WireGuard -> user connects securely.

Project inspiration:

- https://www.digitalocean.com/solutions/vpn

## Current Capability Profile

- Windows desktop workflow driving the WireGuard for Windows tunnel service
- Rust backend + Tauri + Next.js frontend architecture
- Multi-provider adapter model
- DigitalOcean, Amazon EC2, Linode and Google Compute Engine cloud adapters, an experimental
  OpenStack adapter for any Keystone cloud, and a Docker local adapter
- Secure storage for provider tokens and client keys
- SQLite for provider, node, tunnel, and runtime metadata

## What It Does Today

- Stores and validates provider credentials in Windows Credential Manager (no plaintext secrets in SQLite).
- Uses a generalized provider orchestration path for node lifecycle operations.
- Provisions a node on DigitalOcean, Amazon EC2, Linode, Google Compute Engine, an OpenStack cloud, or the local Docker dev provider, and connects to
  it. All paths
  are verified end to end against real infrastructure, including a completed WireGuard handshake.
- Creates and manages local WireGuard client tunnels through the WireGuard for Windows service.
- Shows health, diagnostics, and recovery guidance in the dashboard.
- Supports conservative destroy and force cleanup for interrupted operations.

### The provisioning contract

Provisioning is write-only. Both keypairs are generated on the Windows host before the node exists,
and everything the server needs — its private key, the client's public key, the full interface and
peer config — is baked into a single payload handed to the provider at create time. Afterwards the
app only polls provider metadata for status and endpoint. It never connects to the server it
provisioned, so no SSH or remote shell is required or supported.

## Dev Docker Provider

The Docker provider supports local development and repeatable integration testing.

- Enabled by backend flag: `BLINK_DEV_DOCKER=1`, honoured in debug builds only.
- Exposed in frontend development UX when `NEXT_PUBLIC_DEV_DOCKER=1` and the backend lists Docker.
- Never offered by a release build (`npm run tauri:build`), whatever either flag says.
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
- Provider credentials for production-style testing (a DigitalOcean API token, or an AWS access key
  id and secret access key)
- Docker Desktop (only when testing dev Docker provider)

Run locally:

```bash
npm install
npm run tauri:dev
```

### Tests

Backend unit and integration tests:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Backend tests against a real Docker daemon (provision, delete, client config):

```bash
npm run backend:test:docker:real
```

End-to-end through the real app and the real daemon. These drive the WebView2
devtools protocol directly, so no Tauri test plugin is involved. Check the
environment first:

```bash
npm run e2e:preflight
npm run tauri:e2e:provisioning
```

Connecting a tunnel installs a WireGuard tunnel service, which needs
administrator rights. Run this from an elevated terminal, or it stops on a UAC
prompt Playwright cannot answer:

```bash
npm run tauri:e2e:connect
```

Remove containers left by an interrupted run:

```bash
npm run docker:cleanup
```

Against real DigitalOcean infrastructure. **This creates billable droplets**, destroyed on every
exit path including panics:

```bash
BLINK_DO_TEST_TOKEN=<token> npm run backend:test:do:real
```

It provisions a droplet, connects to it as a real WireGuard client, confirms a handshake, and
destroys it — proving cloud-init brings the server up from the baked-in payload alone.

The equivalent Amazon EC2 lane. **This creates a billable instance and public IPv4 address**, both
destroyed on every exit path along with the security group created alongside them:

```bash
BLINK_AWS_TEST_ACCESS_KEY_ID=<key> BLINK_AWS_TEST_SECRET_ACCESS_KEY=<secret> npm run backend:test:ec2:real
```

See [Testing](docs/TESTING.md).

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