# Project docs and tool references

## Guidance for agents
- Always respect the project documentation in this repository and use it as the source of truth for implementation decisions.
- Prefer the documented architecture, workflows, and conventions over assumptions.

## Required execution protocol
- Start every non-trivial task by restating scope, constraints, and affected components.
- Ask clarifying questions before editing when behavior, state transitions, security posture, or release risk is ambiguous.
- Define a verification plan before changes: list exact commands and expected pass criteria.
- Run verification after changes and report real results, not assumptions.
- If checks cannot run, state exactly why and what risk remains.

## Clarification gate (ask before coding)
When requirements are incomplete, ask focused questions in these areas:
- Product behavior: expected user-visible behavior and edge cases.
- Safety and security: secret handling, privileged operations, and destructive actions.
- Compatibility: contract or schema constraints that must remain stable.
- Test acceptance: minimum tests required for this change.

## Minimum validation matrix
- Frontend-only changes:
	- `npm run typecheck`
	- `npm run lint`
- Rust backend-only changes:
	- `cargo test --manifest-path src-tauri/Cargo.toml`
- Cross-boundary changes (frontend + tauri/backend contracts):
	- `npm run verify:fast`
	- `cargo test --manifest-path src-tauri/Cargo.toml`
- Docker provider or tunnel lifecycle changes:
	- `npm run tauri:e2e:docker:real`
	- `npm run backend:test:docker:real` (when Docker is available)

## Reporting contract
- Include: what changed, what was verified, and what was not verified.
- Include concrete file paths when describing changes.
- Include explicit residual risks if any lane was skipped.

## Major software documentation

### Frontend and app framework
- Next.js: https://nextjs.org/docs
- React: https://react.dev/reference/react
- TypeScript: https://www.typescriptlang.org/docs/
- Tailwind CSS: https://tailwindcss.com/docs/installation

### Desktop and native runtime
- Tauri: https://v2.tauri.app/
- Rust: https://doc.rust-lang.org/book/

### Networking and infrastructure
- WireGuard: https://www.wireguard.com/quickstart/
- Docker: https://docs.docker.com/
- DigitalOcean API: https://docs.digitalocean.com/reference/api/

### Testing and tooling
- Playwright: https://playwright.dev/docs/intro
- tauri-playwright (Playwright bridge for Tauri native webviews): https://github.com/srsholmes/tauri-playwright
- Node.js and npm: https://nodejs.org/docs/latest-v20.x/api/
- SQLite: https://www.sqlite.org/docs.html

## Project-specific docs

### Core project docs
- Repository README: README.md
- Architecture: docs/ARCHITECTURE.md
- Windows setup: docs/SETUP_WINDOWS.md
- Operations runbook: docs/OPERATIONS_RUNBOOK.md
- Testing: docs/TESTING.md
