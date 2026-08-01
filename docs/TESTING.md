# Testing

## Test Strategy

Phase 1 tests are organized around deterministic behavior and failure handling.

### What is Covered

- SQLite repository behavior
- provider token upsert and cleanup
- DigitalOcean retry policy behavior
- WireGuard config generation and runtime parsing
- startup reconciliation and force cleanup paths
- log redaction helpers
- diagnostics redaction

### What is Mocked

- DigitalOcean HTTP interactions use `mockito` or local test doubles.
- WireGuard command execution uses a mock `CommandRunner` in unit tests.
- Secure storage uses the in-memory mock store outside Windows.

### What is Not Mocked

- The UI itself is not fully end-to-end automated in this repo.
- Live DigitalOcean accounts are not required for the normal test suite.
- Real WireGuard services are not required for unit tests.

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

## Critical Test Cases

- token storage does not write plaintext into SQLite
- token validation retries transient provider failures
- node create rolls back a partially created droplet on failure
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
