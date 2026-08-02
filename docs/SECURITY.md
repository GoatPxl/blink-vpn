# Security

## Threat Model Summary

This is a lightweight STRIDE-style summary for Phase 1.

### Token Handling

- Threats: token theft, token_ref leakage, accidental logging, stale secret references.
- Current controls: tokens are stored in Windows Credential Manager, SQLite stores only token_ref, logs are redacted, diagnostics exports are sanitized.
- Residual risk: credential-manager compromise on the host still exposes the token.

### Local Key Material

- Threats: client private key leakage, config file residue, memory/temporary-file exposure.
- Current controls: keypairs are generated locally, the private key is not persisted in SQLite, import temp files are cleaned up best-effort.
- Residual risk: the import temp file exists briefly on disk and could survive a crash.

### API Abuse

- Threats: repeated provisioning, repeated cleanup, accidental destructive reruns, UI-triggered abuse.
- Current controls: state machine validation, idempotent-ish cleanup paths, recovery prompts, and safe no-op behavior when resources are already gone.
- Residual risk: there is no server-side rate limiter or job queue in Phase 1.

### MITM Assumptions

- Threats: TLS interception, malicious network path, provider API spoofing.
- Current controls: HTTPS to cloud provider APIs, WireGuard peer keys for tunnel security.
- Residual risk: no certificate pinning in Phase 1; trust is delegated to the OS CA store and provider TLS.

### Dev Adapter Boundary

- Threats: local Docker daemon exposure, local container misuse in development workflows.
- Current controls: Docker mode is feature-flag gated, development-only, and intentionally non-production.
- Residual risk: local machine Docker security posture directly affects dev-mode behavior.

## Secret Storage Model

- Provider API tokens are stored in Windows Credential Manager via `SecureStorage`.
- Client private keys are stored in the same secret store when needed.
- SQLite stores `token_ref` and runtime metadata, not plaintext secrets.
- Secret references are sanitized and namespaced by kind and owner.
- Non-Windows test/dev builds use the in-memory mock secret store only.

## Log Redaction Policy

Redaction is enforced in the logging helper and diagnostics export path.

Redacted values include:

- bearer tokens
- `authorization` headers
- `token_ref`
- `api_token`
- `private_key` / `PrivateKey`
- generic `Token` fields

Rules:

- Never print raw provider tokens.
- Never print raw client private keys.
- Never echo secret-bearing command output back to the UI without redaction.
- Prefer event-style structured logs with correlation IDs and explicit non-secret fields.

## Secure Coding Guardrails

- Do not add plaintext secret persistence to SQLite.
- Do not add remote telemetry in Phase 1.
- Keep frontend state derived from backend commands; do not duplicate secret state in browser storage.
- Treat all command inputs as untrusted.
- Keep WireGuard and provider operations idempotent where practical.
- Preserve best-effort cleanup semantics during failures and restarts.

## Incident Response Basics

If a Phase 1 security incident is suspected:

1. Stop issuing new provisioning or cleanup commands.
2. Revoke or rotate the affected provider token.
3. Remove stale local secrets from Windows Credential Manager.
4. Delete local app data if the local state itself is compromised.
5. Reinstall WireGuard for Windows if the local tunnel service is suspect.
6. Review redacted logs only; do not rely on them for raw secret recovery.

## Assumptions

- TODO: project-specific fill for incident contacts, rotation policy, and retention policy.
