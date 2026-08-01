# Known Limitations

## Explicit Phase 1 Boundaries

- Windows desktop only.
- DigitalOcean only.
- WireGuard only.
- Rust backend + Tauri + Next.js only.
- Local-only logging and diagnostics; no remote telemetry.

## Deferred to Phase 2+

- Multi-provider support.
- macOS, Linux, mobile, or browser extension support.
- Advanced VPN features such as multi-hop, mesh, or obfuscation.
- Certificate pinning / transport hardening beyond the OS CA trust model.
- Queue-based background job system.
- Team/org collaborative features.
- Automated remote telemetry or analytics pipelines.

## Current Technical Constraints

- Tauri CSP is currently not locked down in the repo config; release hardening should address that before wider distribution.
- WireGuard private keys are generated locally and exist briefly in a temporary import flow.
- Force cleanup is best-effort for remote droplet deletion if the provider token is stale or unavailable.
- Setup and release validation still assume a developer or operator can run the desktop app locally.

## Assumptions

- TODO: project-specific fill if Phase 2 introduces a supported platform list or multi-provider roadmap change.
