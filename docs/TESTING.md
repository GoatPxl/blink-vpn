# Testing

## Test Strategy

The test program combines fast deterministic checks and real workflow lanes:

- Rust unit/integration tests for state, provider contracts, and recovery paths
- Type and lint checks for frontend correctness
- Real Tauri E2E lane for frontend/backend contract validation
- Real Docker backend lane for provisioning and teardown validation

## Required Verification Matrix

Frontend changes:

```bash
npm run typecheck
npm run lint
```

Backend changes:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Cross-boundary changes:

```bash
npm run typecheck
npm run lint
cargo test --manifest-path src-tauri/Cargo.toml
```

Docker or tunnel lifecycle changes:

```bash
npm run e2e:docker:preflight
npm run tauri:e2e:docker:real
npm run backend:test:docker:real
```

## Real E2E Lane

Command:

```bash
npm run tauri:e2e:docker:real
```

Coverage focus:

- token save and validation
- provider readiness
- node create and ready transition
- tunnel connect lifecycle
- diagnostics contract validation

## Fast Local Checks

```bash
npm run typecheck
npm run lint
cargo test --manifest-path src-tauri/Cargo.toml
```

## Release Confidence Checklist

- Run required matrix commands.
- Confirm real E2E lane passes.
- Confirm cleanup and recovery flows pass.
- Confirm diagnostics redaction expectations pass.
- Confirm installer path works on clean Windows host.

## Test Failure Handling

1. Reproduce in the real lane first.
2. Classify as provider, runtime, state transition, or UI contract issue.
3. Add regression coverage for every escaped defect.
4. Re-run full required matrix before merge.
