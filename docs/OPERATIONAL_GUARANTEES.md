# Operational Guarantees

## Product Guarantees

- Provider credentials are stored in secure storage and referenced in SQLite.
- Node and tunnel lifecycle operations follow validated transition rules.
- Diagnostics exports are sanitized for safe sharing.
- Cleanup flows are designed for repeatable recovery after interrupted operations.

## Platform and Runtime Notes

- Primary desktop target is Windows, driving the WireGuard for Windows tunnel service.
- Provider orchestration supports cloud adapter and local Docker adapter workflows.
- Local diagnostics and logs are available for support and incident analysis.

## Capability Notes

- Provider adapters share a normalized contract for token validation, catalog fetch, provisioning, readiness checks, and teardown.
- Lifecycle APIs are expandable without breaking the existing command surface.
- Recovery tooling supports destroy and force-cleanup paths for stale or partial state.

## Reliability Notes

- Startup reconciliation repairs stale lifecycle state after app restarts.
- Recovery commands prefer safe idempotent behavior.
- Docker readiness checks provide actionable guidance before provisioning.
