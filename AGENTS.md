## 1) Mission and Scope Lock (Hard Constraints)

You are implementing **Phase 1 MVP only** of a desktop VPN manager.

### In Scope (allowed)
- **Platform:** Windows desktop only
- **Provider:** DigitalOcean only
- **VPN protocol:** WireGuard only
- **Stack:** Rust backend + Tauri + Next.js frontend
- Local secure secret storage on Windows
- SQLite local metadata/state
- Provision, connect, disconnect, destroy workflows

### Out of Scope (forbidden in this phase)
- macOS, Linux, iOS, Android
- AWS, GCP, Azure, or any provider besides DigitalOcean
- OpenVPN, IKEv2, or any protocol besides WireGuard
- Browser extension implementation
- Multi-provider abstractions beyond what is minimally necessary
- Multi-hop, mesh VPN, obfuscation, advanced routing features
- Remote telemetry/analytics pipelines
- Team/org multi-user features

If asked to implement out-of-scope items, **do not implement them**.  
Instead, clearly mark them as `Deferred to later phase`.

---

## 2) Execution Style

For each request:
1. Start with a short implementation plan (3–7 bullets).
2. Make small, coherent, compile-ready changes.
3. Explain why each change was made.
4. End with:
   - test results,
   - manual verification steps,
   - known risks/todos.

Do not produce giant monolithic rewrites unless explicitly requested.

---

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
  - provider adapter (DigitalOcean)
  - bootstrap generator
  - wireguard integration
  - secure storage
  - db/repository
  - logging/diagnostics
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
- Use fixtures/mocks for DigitalOcean and WireGuard interactions.
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

---

## 7) Performance and UX Guardrails

- Prioritize predictable UX over aggressive optimization.
- Long-running operations must expose progress/state updates.
- Never block UI thread with backend long tasks.
- Surface actionable error messages:
  - what failed,
  - likely cause,
  - next recovery action.

---

## 8) Output Format Requirements (Strict)

For every response, use this structure:

1. `Plan`
2. `Changes Made`
3. `Tests`
4. `Manual Verification`
5. `Known Limitations / Deferred`

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

Windows + DigitalOcean + WireGuard + Rust + Tauri + Next.js.  
Nothing else.

---


<!-- BEGIN:nextjs-agent-rules -->
# This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` before writing any code. Heed deprecation notices.
<!-- END:nextjs-agent-rules -->
