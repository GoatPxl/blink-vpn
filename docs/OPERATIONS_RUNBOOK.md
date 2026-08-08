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
4. Create node and wait for `ready`.

### Connect

1. Create and activate tunnel.
2. Connect tunnel.
3. Confirm health panel reports `connected`.

### Disconnect and Destroy

1. Disconnect tunnel.
2. Destroy node when no longer needed.
3. Confirm local tunnel rows and provider resources are cleaned up.

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
| WireGuard import/connect failure | Repair WireGuard runtime and retry | Tunnel reaches `connected` |
| Stale node/tunnel state | Force cleanup and re-run flow | Local and remote state aligned |
| Docker readiness failure | Start Docker and rerun readiness check | Readiness reports installed + daemon reachable |

## Escalation

Escalate with diagnostics export and exact command history when:

- repeated failures persist after cleanup
- provider delete path fails repeatedly
- local runtime initialization fails after repair
- database access is unhealthy
