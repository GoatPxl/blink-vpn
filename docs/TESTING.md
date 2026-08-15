# Testing

## Principles

**A test that cannot fail is worse than no test.** The E2E suite once waited for "the expected
message *or* any error title", so it passed whether the app worked or not — a run that provisioned
nothing and connected nothing looked identical to a healthy one. Never accept an error state as a
success condition, and never assert something that is true for almost any input.

**Assert the outcome, not the appearance.** A tunnel can install, start, and report `connected`
while carrying no traffic. Where a real signal exists — a handshake, a byte counter, a row in the
database — assert that instead of a status field.

**Real lanes must clean up on every path.** Tests that create billable or long-lived resources tear
them down before asserting anything, so a failed assertion cannot strand a droplet or a container.

## Layers

| Layer | What it covers | Cost |
|---|---|---|
| Rust unit/integration | state machine, parsers, config rendering, redaction, retry, error mapping | free, deterministic |
| Real Docker backend | provisioning, bootstrap readiness, teardown against a live daemon | needs Docker |
| Real Tauri E2E | the app itself — UI, commands, backend, daemon | needs Docker |
| Real DigitalOcean | the production path against a live droplet | **billable** |

## Required Verification Matrix

| Change scope | Commands |
|---|---|
| Frontend only | `npm run typecheck` · `npm run lint` |
| Rust backend only | `cargo test --manifest-path src-tauri/Cargo.toml` |
| Cross-boundary | both of the above |
| Docker provider / bootstrap | `npm run backend:test:docker:real` |
| UI or command surface | `npm run tauri:e2e:provisioning` |
| Connect / disconnect | `npm run tauri:e2e:connect` (**elevated shell**) |
| Provider adapter or cloud-init | `npm run backend:test:do:real` (**billable**) |

## Real Lanes

### Docker backend

```bash
npm run backend:test:docker:real
```

Provisions real containers and asserts the properties the connect path depends on: the client peer
is present in the server config before the container is reachable, the published port is ephemeral,
and two nodes coexist without evicting each other.

```bash
npm run backend:test:docker:clientconf
```

Leaves a container running and writes a client config, for verifying a handshake by hand.

### Tauri E2E

Check the environment first — it reports what is missing and how to fix it:

```bash
npm run e2e:preflight
npm run tauri:e2e:provisioning
```

Drives the real app through WebView2's devtools protocol. No Tauri test plugin is involved.

```bash
npm run tauri:e2e:connect
```

Installs a WireGuard tunnel service, which requires administrator. **Run it from an elevated
shell**, or it stops on a UAC prompt Playwright cannot answer. Asserts a completed handshake, not
just a `connected` status.

### DigitalOcean

```bash
GOATSTARE_DO_TEST_TOKEN=<token> npm run backend:test:do:real
```

**Creates real, billable droplets.** Both tests destroy the droplet on every exit path, including
panics, and assert only after teardown. A run costs a fraction of a cent; confirm the account has
no leftover droplets afterwards if a run is interrupted:

```bash
curl -H "Authorization: Bearer $TOKEN" https://api.digitalocean.com/v2/droplets
```

Two tests:

- **Provisioning** — a droplet reaches ready with an endpoint and a locally-generated server key,
  with nothing read back off the server.
- **Handshake** — the production path proven end to end. A client container connects to the droplet
  and completes a real WireGuard handshake, confirming that cloud-init brought the server up from
  the baked-in payload alone.

The client runs in a container so no Windows elevation is needed. It is the *client*; nothing execs
into the provisioned droplet.

## Environment Notes

These are properties of the dev machine, not the app, and each one has produced a confusing failure:

- **`docker exec` stdout does not reach the caller** when the daemon is reached over TCP, which is
  the case when Docker runs inside WSL2. Exit codes are reliable. Tests must key off exit status or
  copy files out with `docker cp` — a probe that read stdout reported "no handshake" for every run
  regardless of what the server was doing.
- **`wg show` requires administrator.** Unelevated it reports `Permission denied`, so handshake age
  is unavailable. `sc query` and adapter byte counters need no elevation.
- **Published container ports bind the daemon's loopback**, which under WSL2 is not the Windows
  loopback, and WSL2 does not forward UDP to localhost. The Docker adapter resolves a reachable
  endpoint rather than assuming `127.0.0.1`.

## Cleanup

```bash
npm run docker:cleanup
```

Removes containers labelled `goatstare.provider=docker`. Scoped to that label so nothing else on
the host is touched.

## Failure Handling

1. Reproduce in the real lane before theorising.
2. Confirm the failure is the app's and not the environment's — see Environment Notes.
3. Add regression coverage for every escaped defect.
4. Re-run the required matrix before merge.
