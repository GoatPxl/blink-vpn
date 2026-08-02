# Windows Setup

## Prerequisites

- Windows 10 or Windows 11
- Node.js 20+ and npm
- Rust toolchain matching the repo's `rust-version`
- WireGuard for Windows installed locally
- Provider API token for your selected provider (DigitalOcean for production path in this phase)
- Docker Desktop for Windows (optional, only for Docker dev provider mode)

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

4. Use the UI to register your provider token, then validate it before provisioning a node.

## Docker Dev Provider Mode (Optional)

Use this when you want local provider-emulation behavior without a live cloud account.

1. Enable backend feature flag:

```bash
set GOATSTARE_DEV_DOCKER=1
```

2. Enable frontend dev surfaces:

```bash
set NEXT_PUBLIC_DEV_DOCKER=1
```

3. Start Docker Desktop and verify daemon is running.
4. Launch app (`npm run tauri:dev`).
5. In Settings, select Docker provider (`provider-docker`) and save faux token with prefix `dkr_v1_` or `docker_`.
6. Run Docker readiness check.
7. Create a node normally; Docker provider will emulate VPS provisioning with local containers and synthetic API-style data.

## Environment / Config

The current Phase 1 app keeps runtime configuration intentionally small:

- Tauri reads the app data directory from the Windows profile.
- SQLite is created under the app data directory.
- Secrets are stored in Windows Credential Manager, not in environment variables.
- Diagnostics exports are written under the app data diagnostics folder.
- Optional dev Docker mode is controlled by `GOATSTARE_DEV_DOCKER` (backend) and `NEXT_PUBLIC_DEV_DOCKER` (frontend).

### Known config points

- `src-tauri/tauri.conf.json` currently bundles to NSIS.
- `csp` is currently `null` in the Tauri config; treat CSP hardening as a release gate.
- TODO: project-specific fill if a future installer adds explicit environment flags or registry-based config.

## Common Setup Pitfalls

| Symptom | Likely Cause | Recovery |
| --- | --- | --- |
| Token validation fails immediately | Invalid provider token or missing API scope | Re-enter the token and validate again |
| WireGuard actions fail with not installed / not found | WireGuard for Windows is absent or not on PATH | Install or repair WireGuard, then restart the app |
| Node create stops at bootstrapping | Provider node never became active or bootstrap marker did not become ready | Review node status, reconcile, or force cleanup and retry |
| Docker readiness check fails in dev mode | Docker CLI missing, daemon stopped, or dev flag disabled | Install/start Docker and ensure `GOATSTARE_DEV_DOCKER=1` |
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
