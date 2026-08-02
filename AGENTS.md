## 1) Mission and Scope Lock (Hard Constraints)

You are implementing **Phase 1 MVP only** of a desktop VPN manager.

### In Scope (allowed)
- **Platform:** Windows desktop only
- **Provider baseline:** DigitalOcean production adapter first (default implementation path)
- **Provider architecture:** Multi-provider adapter model is in scope now (provider registry + adapter routing), with incremental provider implementation when explicitly requested
- **Dev provider path:** Docker provider is allowed for local dev/test emulation only, guarded by feature flags and synthetic responses
- **VPN protocol:** WireGuard only
- **Stack:** Rust backend + Tauri + Next.js frontend + Docker (local dev/test orchestration)
- Local secure secret storage on Windows
- SQLite local metadata/state
- Generalized node lifecycle API direction: create, edit, suspend/resume, delete, get/list system info
- Implemented core workflows in this phase: provision/create, connect, disconnect, destroy, force-cleanup

## 3) Coding Standards

### General
- Prefer clarity over cleverness.
- Keep functions small and single-purpose.
- Use explicit types and structured error handling.
- No dead code, no commented-out blocks, no placeholder TODO logic unless explicitly marked and justified.
- Keep public interfaces minimal and stable.

### Rust
- Use idiomatic Rust (`Result`, `thiserror`/typed errors, `serde` where appropriate).
- Avoid `unwrap()`/`expect()` in production code paths.
- Centralize error taxonomy and map to user-actionable messages.
- Use module boundaries:
  - orchestrator
  - provider registry + adapters (DigitalOcean production + Docker dev emulation)
  - bootstrap generator
  - wireguard integration
  - secure storage
  - db/repository
  - logging/diagnostics
- Keep provider-specific behavior in adapter modules; keep orchestrator/provider APIs provider-neutral.
- Redact secrets at log boundaries.

### Next.js / Frontend
- Keep UI simple, deterministic, and state-driven.
- Prefer strongly typed API bindings to Tauri commands.
- Handle loading, success, and error states explicitly.
- Never expose secrets in UI logs or persisted browser-like storage.

### Tauri bridge
- Validate command inputs.
- Return structured responses/errors (not ad hoc strings only).
- Ensure backend errors are safely mapped for UI display.
- Keep command contracts aligned to generalized provider/node operations and capability limits.

---

## 4) Security Requirements (Non-Negotiable)

- Never store API tokens or private keys in plaintext in SQLite/files/logs.
- Use Windows secure storage mechanisms for secrets.
- Redact:
  - API tokens
  - private keys
  - authorization headers
  - full WireGuard config private sections
- Do not print secrets in debug output, tests, snapshots, or error messages.
- Use least-privilege defaults for network/firewall assumptions.
- Treat all external inputs as untrusted.
- Treat Docker CLI/runtime output as untrusted process input in dev mode.

If a requested change risks secret exposure, stop and propose a safer alternative.

---

## 5) Testing Requirements

Every change must include tests appropriate to scope.

### Required test types
- **Unit tests** for pure logic (state machine transitions, parsers, config generation).
- **Integration-style tests** for DB repositories and adapters (using mocks/fakes for network/process calls).
- **Error-path tests** for retries/timeouts/failure handling.
- **Redaction tests** proving secrets cannot leak to logs.

### Test quality rules
- Deterministic tests only (no flaky timing dependencies).
- No real external API calls in normal test suite.
- Use fixtures/mocks for DigitalOcean, Docker dev-provider contracts, and WireGuard interactions.
- Include both happy path and key failure path coverage.

When finished, report:
- tests added/updated,
- tests run,
- pass/fail status.

---

## 6) Reliability and Idempotency Rules

- Provision/connect/disconnect/destroy flows must be idempotent where practical.
- Implement bounded retries with exponential backoff + jitter for transient failures.
- Add explicit operation timeouts.
- On partial failure, provide safe rollback/cleanup strategy.
- Reconciliation on startup should detect and repair/flag drift (local vs remote state mismatch).
- Provider adapter operations must preserve idempotent destroy/cleanup semantics across providers.

---

## 7) Performance and UX Guardrails

- Prioritize predictable UX over aggressive optimization.
- Long-running operations must expose progress/state updates.
- Never block UI thread with backend long tasks.
- Surface actionable error messages:
  - what failed,
  - likely cause,
  - next recovery action.
- When an operation is provider-specific or unsupported, surface explicit capability guidance.

---

## 8) Provider and Environment Rules

- Keep production behavior anchored to the DigitalOcean adapter unless explicitly expanding scope.
- Docker provider must remain development-only and must not be treated as a production security boundary.
- Respect feature-flag boundaries:
  - backend: `GOATSTARE_DEV_DOCKER`
  - frontend dev surface: `NEXT_PUBLIC_DEV_DOCKER`
- For provider lifecycle modeling, align request/response semantics with DigitalOcean droplet APIs where practical.

---


## 9) Dependency and Tooling Rules

- Minimize new dependencies; justify each addition.
- Prefer mature, well-maintained crates/packages.
- Pin/lock versions consistent with project policy.
- If adding a dependency, include:
  - why needed,
  - security/maintenance rationale,
  - alternative considered.

---

## 10) Definition of Done (for each task)

A task is done only if:
- Code compiles,
- tests pass,
- no scope violations,
- no secret-handling violations,
- docs stay consistent with current provider architecture and phase boundaries,
- clear verification steps are documented.

If not fully complete, explicitly state what remains and why.

---

## 11) Prompt Interaction Rule

If requirements are ambiguous:
- Make the safest minimal assumption,
- proceed with smallest useful implementation chunk,
- and list assumptions explicitly at top of response.

Do not expand scope without explicit approval.

---

## 12) Canonical Phase 1 Boundaries Reminder (repeat mentally before coding)

Windows + WireGuard + Rust + Tauri + Next.js + multi-provider adapter architecture.  
Production provider: DigitalOcean. Dev provider emulation: Docker behind feature flags.  
Expand to additional providers only when explicitly requested, in small scoped increments.

---


<!-- BEGIN:nextjs-agent-rules -->
# This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` before writing any code. Heed deprecation notices.
<!-- END:nextjs-agent-rules -->
