# Phase 2 Roadmap

## Baseline Carried Into Phase 2

Already in current Phase 1 iteration:

- Provider adapter architecture with a generalized node lifecycle path.
- Production adapter for DigitalOcean.
- Development-only Docker adapter for local emulation using synthetic provider responses.

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

## Priority 3: Generalized Node API Completion

Goal: complete provider-neutral node management semantics and close contract gaps.

- Promote node-level `edit` semantics across adapters where capability exists.
- Add explicit node-level `suspend` / `resume` lifecycle operations.
- Keep `create`, `delete`, and `get_system_info` behavior consistent across provider adapters.
- Improve capability signaling so UI can clearly indicate provider-specific feature availability.

Dependency risks:

- Providers differ in mutable fields and suspend semantics.
- Contract must stay backward-compatible for existing DigitalOcean and Docker flows.

## Priority 4: Observability and Supportability

Goal: make troubleshooting easier without adding telemetry.

- Improve local diagnostics summaries.
- Add more structured event logging around lifecycle transitions.
- Document common failure signatures in the runbook.

Dependency risks:

- Must keep all outputs redacted and local-only.

## Priority 5: Provider Expansion

Deferred until Phase 1 is stable.

- Additional production provider adapters beyond DigitalOcean.
- Additional operating systems.
- Advanced routing and VPN features.
- Collaborative/team workflows.

## Suggested Milestone Slicing

1. Security and release hardening.
2. Cleanup/recovery UX improvements.
3. Node API contract completion (`edit` + node suspend/resume).
4. Reconciliation and timeout refinement.
5. Additional provider adapters only after external stability.

## Assumptions

- TODO: project-specific fill for the first non-DigitalOcean production provider target.
