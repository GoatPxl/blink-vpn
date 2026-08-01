# Windows Setup

## Prerequisites

- Windows 10 or Windows 11
- Node.js 20+ and npm
- Rust toolchain matching the repo's `rust-version`
- WireGuard for Windows installed locally
- A DigitalOcean API token with droplet management permissions

## Local Setup

1. Clone the repository.
2. Install frontend dependencies.

```bash
npm install
```

3. Run the desktop app in development mode.

```bash
npm run tauri:dev
```

4. Use the UI to register your DigitalOcean token, then validate it before provisioning a node.

## Environment / Config

The current Phase 1 app keeps runtime configuration intentionally small:

- Tauri reads the app data directory from the Windows profile.
- SQLite is created under the app data directory.
- Secrets are stored in Windows Credential Manager, not in environment variables.
- Diagnostics exports are written under the app data diagnostics folder.

### Known config points

- `src-tauri/tauri.conf.json` currently bundles to NSIS.
- `csp` is currently `null` in the Tauri config; treat CSP hardening as a release gate.
- TODO: project-specific fill if a future installer adds explicit environment flags or registry-based config.

## Common Setup Pitfalls

| Symptom | Likely Cause | Recovery |
| --- | --- | --- |
| Token validation fails immediately | Invalid DigitalOcean token or missing API scope | Re-enter the token and validate again |
| WireGuard actions fail with not installed / not found | WireGuard for Windows is absent or not on PATH | Install or repair WireGuard, then restart the app |
| Node create stops at bootstrapping | Droplet never became active or bootstrap marker did not become ready | Review node status, reconcile, or force cleanup and retry |
| Secret reads return missing reference | Credential Manager entry was removed or the reference is stale | Re-save the provider token |
| App opens but backend commands fail | Running outside Tauri or from a plain browser tab | Launch with `npm run tauri:dev` |

## Local Validation Checklist

- Confirm the app starts and the dashboard loads.
- Register a provider token and validate it.
- Create a node and observe provisioning -> bootstrapping -> ready.
- Create a tunnel and confirm connect/disconnect works.
- Destroy a node and confirm cleanup completes.

## Assumptions

- TODO: project-specific fill for any company-managed Windows images or preinstalled tooling.
