# Release Checklist

## Windows Packaging Checklist

- Build the Tauri app for Windows only.
- Package with NSIS as configured in `src-tauri/tauri.conf.json`.
- Sign the application binaries and the installer before release.
- Verify the app launches from the installer on a clean Windows host.
- Verify the installer does not bundle plaintext secrets or debug logs.

## Signing / Release Gates

- TODO: project-specific fill for the code-signing certificate source and CI signing step.
- Confirm the installer metadata, product name, and identifier are correct.
- Confirm the release artifact version matches the repo version.
- Verify the release artifact is not built from an unclean worktree.

## Installer Prerequisites

- WireGuard for Windows must be installed on the target machine.
- The WireGuard service/control tooling must be available to the app.
- The user may need permission to install or manage services.
- Production provider network access must be allowed from the machine (DigitalOcean in current phase).

## Rollback / Uninstall Plan

- Uninstall should remove local app data if the deployment process requires a full reset.
- Force cleanup should be run before uninstall if a node may still exist remotely.
- Delete any diagnostics exports or temporary artifacts if they are no longer needed.
- Rotate or revoke provider tokens if the machine is being decommissioned.

## Go / No-Go Criteria

### Go

- Rust tests pass.
- Frontend typecheck passes.
- Frontend lint passes.
- Token storage and redaction tests pass.
- Force cleanup behaves correctly after a simulated interrupted provision.
- A clean Windows host can save a token, create a node, connect a tunnel, disconnect, and destroy safely.

### No-Go

- Secrets appear in logs, diagnostics, or SQLite.
- WireGuard is missing from the demo host.
- The installer is unsigned for external distribution.
- Force cleanup fails to remove partial local state.
- The app can only be validated on a developer machine with hidden setup.

## Release Decision Table

| Condition | Decision |
| --- | --- |
| Tests green, secrets redacted, installer signed, WireGuard prereq verified | Go |
| Any secret leakage, unsigned release artifact, or missing WireGuard prereq | No-Go |
| Cleanup/recovery path unverified | No-Go |

## Assumptions

- TODO: project-specific fill for release owner, store, and signing policy.
