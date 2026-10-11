# Windows Setup

## Prerequisites

- Windows 10 or Windows 11
- Node.js 20+
- npm
- Rust toolchain
- WireGuard for Windows
- WireGuard for Windows installed (provides `wireguard.exe` and `wg.exe`)
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
5. Validate token, create node, and connect tunnel.

## Docker Adapter Setup

Set backend and frontend flags in your terminal session:

```bash
set BLINK_DEV_DOCKER=1
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
npm run e2e:preflight
npm run tauri:e2e:provisioning
```

## Runtime Notes

- No WireGuard driver DLL is bundled or downloaded. The app requires WireGuard for Windows to be installed and drives tunnels through its service.
- Tunnel lifecycle uses `wireguard.exe /installtunnelservice` and `sc` for start/stop/query. `wg show` supplies peer metrics but requires administrator, so handshake and byte counters are unavailable when the app runs unelevated; the tunnel still works.
- App metadata persists in local app-data SQLite.
- Secret values remain in secure storage.

## Quick Troubleshooting

| Symptom | Action |
| --- | --- |
| Token validation fails | Re-enter token and validate again |
| Tunnel operations fail | Confirm the selected mode, repair the WireGuard runtime if needed, and restart the app |
| Docker readiness fails | Start Docker Desktop and rerun readiness |
| Node stuck in error | Run force cleanup and retry |
