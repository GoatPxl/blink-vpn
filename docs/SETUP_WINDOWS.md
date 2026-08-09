# Windows Setup

## Prerequisites

- Windows 10 or Windows 11
- Node.js 20+
- npm
- Rust toolchain
- WireGuard for Windows
- DefGuard runtime DLL resources bundled for the desktop app
- Docker Desktop (for Docker adapter workflows)

## Local Development Setup

1. Install dependencies:

```bash
npm install
```

2. Start desktop app:

```bash
npm run tauri:dev
```

3. Open Settings and register provider credentials.
4. Choose the WireGuard connection mode in Settings: `Admin Mode` for virtual-adapter flows with elevation prompts, or `User Mode` for the installed WireGuard service path.
5. Validate token, create node, and connect tunnel.

## Docker Adapter Setup

Set backend and frontend flags in your terminal session:

```bash
set GOATSTARE_DEV_DOCKER=1
set NEXT_PUBLIC_DEV_DOCKER=1
```

Then run:

```bash
npm run tauri:dev
```

In app:

1. Select Docker provider.
2. Save token with `dkr_v1_` or `docker_` prefix.
3. Run readiness check.
4. Create node and run tunnel lifecycle actions.

## E2E Validation Setup

```bash
npm run e2e:docker:preflight
npm run tauri:e2e:docker:real
```

## Runtime Notes

- WireGuard DLL resources are loaded from [src-tauri/resources-windows/binaries](src-tauri/resources-windows/binaries).
- The app does not rely on direct WireGuard CLI calls; local tunnel lifecycle is handled through the DefGuard runtime and the selected connection mode.
- App metadata persists in local app-data SQLite.
- Secret values remain in secure storage.

## Quick Troubleshooting

| Symptom | Action |
| --- | --- |
| Token validation fails | Re-enter token and validate again |
| Tunnel operations fail | Confirm the selected mode, repair the WireGuard runtime if needed, and restart the app |
| Docker readiness fails | Start Docker Desktop and rerun readiness |
| Node stuck in error | Run force cleanup and retry |
