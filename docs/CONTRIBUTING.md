# Contributing

## Coding Standards

- Keep changes Phase 1 scoped: Windows, WireGuard, provider-adapter architecture.
- Preserve provider-neutral behavior in orchestration paths; keep provider-specific logic inside adapters.
- Treat Docker provider as development-only behavior unless scope is explicitly expanded.
- Prefer small, compile-ready changes over broad rewrites.
- Keep functions and command handlers narrowly focused.
- Use explicit error mapping and actionable user-facing messages.
- Do not log secrets, token values, or private keys.
- Keep redaction at the source, not only in the UI.

## Branching / PR Guidance

- Use one feature or fix per branch.
- Keep PRs small enough to review against the state machine, not just the UI.
- Include tests for any logic change that affects provisioning, cleanup, or redaction.
- Update docs when workflows, commands, or schema behavior changes.

## Commit Conventions

- Prefer short, imperative commit messages.
- Example style: `docs: add Phase 1 release checklist`, `fix: tighten cleanup retry path`.
- Keep commits aligned with the feature slice under review.

## Review Expectations

Reviewers should check:

- idempotency of retries and cleanup
- secret handling and redaction coverage
- Windows-only assumptions
- SQLite schema compatibility
- UI recovery guidance for each error class
- tests covering the changed path

## When to Ask for Help

- If a change would expand beyond Phase 1 scope.
- If a change risks plaintext secret exposure.
- If a change requires installer signing or release infra details that are not documented yet.

## Assumptions

- TODO: project-specific fill for team review cadence or approval policy.
