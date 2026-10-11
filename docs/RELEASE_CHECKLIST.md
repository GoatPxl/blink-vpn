# Release Checklist

## Build and Package

- Build desktop app artifacts.
- Build installer artifacts.
- Verify version metadata consistency across app and installer.
- Verify release was produced from a clean git worktree.

## Security and Compliance

- Confirm binaries and installer are signed.
- Confirm diagnostics export remains sanitized.
- Confirm no plaintext secrets in SQLite, logs, or packaged assets.
- Confirm credential storage behavior is unchanged.

## Functional Verification

Run and record outcomes for:

```bash
npm run typecheck
npm run lint
cargo test --manifest-path src-tauri/Cargo.toml
npm run e2e:preflight
npm run tauri:e2e:provisioning
npm run backend:test:docker:real
```

Manual verification:

- Save and validate provider token.
- Build a server from the Map and wait for it to connect by itself.
- Create, connect, and disconnect tunnel.
- Destroy node and verify cleanup behavior.
- Export diagnostics and confirm redaction.

## Release Decision

Go criteria:

- Validation commands succeed.
- Real Tauri E2E lane succeeds.
- Cleanup and recovery behavior confirmed.
- Signing and packaging checks complete.

Hold criteria:

- Any secret leakage.
- Any lifecycle regression in provision/connect/destroy.
- Any unsigned or mismatched release artifact.
