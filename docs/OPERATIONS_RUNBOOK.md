# Operations Runbook

## Daily Health Routine

1. Open dashboard and confirm provider and node data loads.
2. Validate provider token if any provider status warning appears.
3. Confirm active node is in `ready` or `connected` state.
4. Confirm health panel updates handshake and transfer stats.

## Standard Workflows

### Provision

1. Register token in Settings.
2. Validate token.
3. Select provider, region, and plan.
4. Create node. A progress card in the corner follows it to `ready`; the percentage is an estimate
   from typical durations, and the line beneath it names the real stage. On success the app returns
   to the server list.

### Connect

1. Confirm WireGuard for Windows is installed and the app can reach `wireguard.exe`.
2. Create and activate tunnel.
3. Connect tunnel.
4. Confirm health panel reports `connected`.

### Turn off and turn on

1. Disconnect the tunnel.
2. Turn the server off (Nodes → Saved locations, or Turn off in the server's details). This deletes it
   at the provider and stops billing; the location stays saved.
3. To use it again, choose Turn on. The current price is shown before anything is created. It
   provisions a new server with new keys and a new IP, so phones must scan their new codes.
4. Forget removes a location that is off. It never touches the provider.

If turning off fails with "could not be deleted at the provider", nothing was removed locally and the
server is still billing. Retry; if it keeps failing, delete it in the provider's console and then use
Force cleanup on the node.

### Closing and quitting

Closing the window hides GoatStare to the system tray by default; the tray icon restores it, and its
"Quit" item exits. Turn this off in Settings → Window. Quitting leaves a connected tunnel running.

## Recovery Workflow

Use this order for fast recovery:

1. Retry action once for transient failures.
2. Refresh dashboard state.
3. Re-validate provider token.
4. Run force cleanup for stale or partial resources.
5. Re-run the desired workflow.

## Issue-to-Action Table

| Signal | Action | Success criteria |
| --- | --- | --- |
| Provider token invalid | Re-save token, validate token | Provider status healthy |
| Provision timeout | Refresh state, run cleanup if needed | Node reaches `ready` or is fully removed |
| WireGuard runtime or mode failure | Repair the local runtime, confirm the selected mode, and retry | Tunnel reaches `connected` |
| Stale node/tunnel state | Force cleanup and re-run flow | Local and remote state aligned |
| Docker readiness failure | Start Docker and rerun readiness check | Readiness reports installed + daemon reachable |
| Brief disconnect around 04:00 server time | None — the server applies security patches unattended and reboots itself when a patch requires it; the tunnel returns on its own | Tunnel reconnects within a few minutes |

## Escalation

Escalate with diagnostics export and exact command history when:

- repeated failures persist after cleanup
- provider delete path fails repeatedly
- local runtime initialization fails after repair
- database access is unhealthy
