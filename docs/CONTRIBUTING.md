# Contributing

## Principles

- Keep changes small, testable, and scoped.
- Preserve provider-neutral orchestration behavior.
- Keep provider-specific behavior inside adapter modules.
- Never log secrets or raw credential material.
- Prefer explicit errors with actionable recovery guidance.

## Workflow

1. Restate scope, constraints, and affected components.
2. Confirm acceptance criteria before implementation.
3. Implement smallest viable change.
4. Run required validation lanes.
5. Report verified results and residual risk.

## Required Validation

Frontend-focused changes:

```bash
npm run typecheck
npm run lint
```

Backend-focused changes:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Cross-boundary changes:

```bash
npm run typecheck
npm run lint
cargo test --manifest-path src-tauri/Cargo.toml
```

Docker lifecycle or tunnel lifecycle changes:

```bash
npm run e2e:docker:preflight
npm run tauri:e2e:docker:real
npm run backend:test:docker:real
```

## Pull Request Checklist

- Include summary of behavior changes.
- Include updated docs when behavior/contracts change.
- Include test evidence and exact commands run.
- Include rollback considerations for destructive flows.

## Commit Guidance

- Use concise, imperative messages.
- Keep one concern per commit when practical.
- Avoid mixing refactor and behavior changes without clear separation.
