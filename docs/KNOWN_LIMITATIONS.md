# Known Limitations

## Explicit Phase 1 Boundaries

- Windows desktop only.
- WireGuard only.
- Rust backend + Tauri + Next.js only.
- Local-only logging and diagnostics; no remote telemetry.
- Multi-provider architecture is present, but production cloud path is still limited to DigitalOcean in this phase.
- Docker provider is development-only and not a production deployment target.

## Deferred to Phase 2+

- Additional production provider adapters beyond DigitalOcean.
- macOS, Linux, mobile, or browser extension support.
- Advanced VPN features such as multi-hop, mesh, or obfuscation.
- Certificate pinning / transport hardening beyond the OS CA trust model.
- Queue-based background job system.
- Team/org collaborative features.
- Automated remote telemetry or analytics pipelines.

## Generalized Node API Gaps (Current Iteration)

- Node create/list/delete/system info paths are implemented through the provider abstraction.
- Node edit and node suspend/resume are architectural contract goals but not yet fully exposed as first-class node commands across adapters.
- Some provider-specific capabilities remain normalized to the lowest common denominator in this phase.

## Current Technical Constraints

- Tauri CSP is currently not locked down in the repo config; release hardening should address that before wider distribution.
- WireGuard private keys are generated locally and exist briefly in a temporary import flow.
- Force cleanup is best-effort for remote provider resource deletion if the provider token is stale or unavailable.
- Docker dev mode depends on Docker Desktop/daemon availability and synthetic credentials.
- Setup and release validation still assume a developer or operator can run the desktop app locally.

## Assumptions

- TODO: project-specific fill if a second production provider is promoted in this code line.
