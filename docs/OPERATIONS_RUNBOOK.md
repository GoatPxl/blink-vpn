# Operations Runbook

## Day-to-Day Operations

### Morning checks

- Open the dashboard and confirm the selected node health is loading.
- Validate the selected provider token if the dashboard shows provider errors.
- Confirm the selected node is in a sane state: `ready` or `connected` for active use.
- Check diagnostics only for redacted output; do not rely on logs for secrets.

### Common actions

- Provision a node only after token validation succeeds.
- Create the tunnel after the node reports a public endpoint and server public key.
- Use connect/disconnect rather than destroy when the node should remain available.
- Use destroy when the node and its tunnels are no longer needed.
- Use force cleanup after interrupted provisioning, app restart mid-provision, or stale local WireGuard artifacts.
- In Docker dev mode, confirm Docker readiness before create/destroy operations.

## Troubleshooting Flow

1. Identify the error class shown in the UI.
2. Check whether the failure is provider, WireGuard, database/local state, or lifecycle mismatch.
3. Choose the smallest recovery action:
   - Retry the same action if the error was transient and the current state is still valid.
   - Refresh the dashboard if the UI state appears stale.
   - Validate the provider token if the issue is provider-related.
   - Force cleanup if provisioning was interrupted or the local tunnel state is inconsistent.
4. Re-run the workflow only after the state is known to be safe.

## Recovery Actions

| Situation | Action | Expected Result |
| --- | --- | --- |
| Token invalid / missing | Re-save and validate token | Provider becomes usable again |
| Provider node creation timed out | Wait briefly, then reconcile or force cleanup | Local node state matches remote reality |
| App restart mid-provision | Force cleanup the partially created node | Local tunnels removed, remote delete attempted |
| WireGuard service missing | Repair/reinstall WireGuard for Windows | Local tunnel operations become available |
| Tunnel status stale | Refresh health | Status snapshot updates from runtime |
| Node stuck in error state | Force cleanup, then retry provisioning if needed | Partial state is removed |
| Docker readiness fails in dev mode | Start Docker Desktop and re-run readiness check | Dev adapter operations become available |

## Force Cleanup Behavior

Force cleanup is the preferred recovery path for interrupted provisioning in Phase 1.

- Removes local tunnel artifacts.
- Attempts best-effort remote provider resource deletion.
- Deletes local node/tunnel rows when possible.
- Treats missing local state as success, not as a hard failure.

## Escalation Guidance

Escalate to manual intervention if:

- The provider token cannot be recovered.
- The remote node is unreachable and cannot be deleted through the provider API.
- Local WireGuard tools are missing after reinstall attempts.
- The SQLite database is unreadable or corrupted.

## Assumptions

- TODO: project-specific fill for support ownership and escalation contacts.
