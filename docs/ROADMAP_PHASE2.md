# Phase 2 Roadmap

## Priority 1: Release Hardening

Goal: make Phase 1 safer to distribute without changing scope.

- Lock down CSP for the Tauri webview.
- Add/confirm installer signing automation.
- Add startup checks for WireGuard prereqs and clearer first-run failure messages.
- Improve uninstall/rollback cleanup for local app data and temp artifacts.

Dependency risks:

- Signing process may depend on company PKI or release infrastructure not present in the repo.
- CSP hardening may require frontend asset review.

## Priority 2: Operational Resilience

Goal: reduce operator friction during partial failures.

- Improve timeout coverage for long-running provisioning phases.
- Add more explicit recovery states or guidance in the UI.
- Expand restart reconciliation coverage for more drift cases.

Dependency risks:

- Long-running retries must remain idempotent.
- WireGuard service availability remains an external dependency.

## Priority 3: Observability and Supportability

Goal: make troubleshooting easier without adding telemetry.

- Improve local diagnostics summaries.
- Add more structured event logging around lifecycle transitions.
- Document common failure signatures in the runbook.

Dependency risks:

- Must keep all outputs redacted and local-only.

## Priority 4: Scope Expansion

Deferred until Phase 1 is stable.

- Multi-provider support.
- Additional operating systems.
- Advanced routing and VPN features.
- Collaborative/team workflows.

## Suggested Milestone Slicing

1. Security and release hardening.
2. Cleanup/recovery UX improvements.
3. Reconciliation and timeout refinement.
4. Any scope expansion only after Phase 1 is externally stable.

## Assumptions

- TODO: project-specific fill for the exact Phase 2 release target and ownership model.
