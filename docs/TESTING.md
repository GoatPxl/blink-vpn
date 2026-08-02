# Testing

## Test Strategy

Phase 1 tests are organized around deterministic behavior and failure handling.

### What is Covered

- SQLite repository behavior
- provider token upsert and cleanup
- provider adapter retry policy behavior
- provider region/pricing catalog normalization
- WireGuard config generation and runtime parsing
- startup reconciliation and force cleanup paths
- log redaction helpers
- diagnostics redaction

### What is Mocked

- DigitalOcean HTTP interactions use `mockito` or local test doubles.
- Docker provider behavior uses local adapter-level mocks/stubs and synthetic data contracts.
- WireGuard command execution uses a mock `CommandRunner` in unit tests.
- Secure storage uses the in-memory mock store outside Windows.

### What is Not Mocked

- The UI itself is not fully end-to-end automated in this repo.
- Live cloud provider accounts are not required for the normal test suite.
- Real WireGuard services are not required for unit tests.

## Docker E2E Real-Daemon Lane

This lane is the contributor gate for Docker dashboard flows in Prompts 3-7.

### Prerequisites

- Windows host (PowerShell or cmd)
- Docker Desktop installed and running
- Docker CLI available on `PATH`
- Dependencies installed with `npm install`

### Preflight

Run preflight first. It must print both client and server version details.

```bash
npm run frontend:e2e:docker:preflight
```

### Required Run Order

Use the ordered lane command so dependencies between prompts are exercised consistently:

```bash
npm run frontend:e2e:docker:lane
```

The lane runs specs in this exact order:

1. `tests/e2e/dashboard-docker-provider.spec.ts` (feature flag/readiness/token flow)
2. `tests/e2e/dashboard-docker-nodes.spec.ts` (create-to-ready)
3. `tests/e2e/dashboard-docker-tunnel.spec.ts` (connect/disconnect + health transition)
4. `tests/e2e/dashboard-docker-destroy-cleanup.spec.ts` (destroy + force-cleanup idempotency)

For interactive debugging:

```bash
npm run frontend:e2e:docker:lane:headed
```

### Failure Handling

- If preflight fails, do not run the lane; start Docker Desktop and rerun preflight.
- If lane fails, fix in prompt order (provider -> nodes -> tunnel -> destroy/cleanup) and rerun the full lane.

## How to Run Tests

Backend:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Frontend typecheck:

```bash
npm run typecheck
```

Frontend lint:

```bash
npm run lint
```

General Playwright (all E2E specs):

```bash
npm run frontend:e2e
```

Native-webview Tauri E2E support (plugin enabled build):

```bash
npm run tauri:dev:e2e
```

## Critical Test Cases

- token storage does not write plaintext into SQLite
- token validation retries transient provider failures
- node create rolls back a partially created provider node on failure
- force cleanup removes local tunnel artifacts after an interrupted provisioning path
- startup reconciliation repairs stale tunnel state
- redaction removes token/private-key material from logs and diagnostics
- WireGuard unavailable on Windows surfaces a clear, actionable error

## Release Test Checklist

Before a Phase 1 demo release:

- Run the full Rust test suite.
- Run `npm run typecheck`.
- Run `npm run lint`.
- Smoke test token save/validate.
- Smoke test create/connect/disconnect/destroy.
- Smoke test force cleanup after a simulated partial provision.
- Confirm diagnostics export contains no plaintext secrets.
- Confirm the installer path works on a clean Windows machine with WireGuard installed.

## Assumptions

- TODO: project-specific fill for any additional integration tests in CI.
