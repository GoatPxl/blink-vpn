# Security

## Security Objectives

- Keep secret values out of SQLite and logs.
- Keep command boundaries validated and least-privilege by design.
- Keep diagnostics export safe for sharing.
- Keep lifecycle operations recoverable under failure.

## Secret Handling

- Provider tokens are stored in Windows Credential Manager through secure storage abstraction.
- SQLite stores secret references and operational metadata.
- Tunnel private keys are handled in secure storage paths and excluded from diagnostics/log output.

## Boundary Model

- Frontend to backend: untrusted input boundary.
- Backend to provider APIs: remote boundary over TLS.
- Backend to the WireGuard tunnel service: privileged local boundary (elevation on connect and disconnect only).
- Backend to Docker runtime: local process boundary with validated command output.

## Redaction Policy

Always redact:

- bearer tokens
- authorization headers
- provider token references
- API token fields
- private keys

Rules:

- Never emit raw credential material in logs.
- Never include secret fields in diagnostics export.
- Keep redaction logic in backend serialization and logging helpers.

## Secure Coding Requirements

- Validate command inputs before state mutation.
- Preserve idempotent cleanup semantics for destructive operations.
- Avoid introducing plaintext secret persistence.
- Prefer explicit, typed error classes for UI guidance.

## Incident Handling

1. Pause provisioning/destroy operations.
2. Rotate affected provider credentials.
3. Remove stale secure-storage references.
4. Export diagnostics and capture runtime logs.
5. Reconcile and cleanup local state.
6. Resume operations after verification.
