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
them down before asserting anything, so a failed assertion cannot strand a paid server or a
container.

## Layers

| Layer | What it covers | Cost |
|---|---|---|
| Rust unit/integration | state machine, parsers, config rendering, redaction, retry, error mapping | free, deterministic |
| Frontend unit | pure presentation logic — formatters, state-to-label mapping | free, deterministic |
| Real Docker backend | provisioning, bootstrap readiness, teardown against a live daemon | needs Docker |
| Real Tauri E2E | the app itself — UI, commands, backend, daemon | needs Docker |
| Real DigitalOcean | the production path against a live droplet | **billable** |
| Real Amazon EC2 | the production path against a live instance | **billable** |
| Real Linode | the production path against a live instance | **billable** |
| Real Google Compute Engine | the production path against a live instance | **billable** |
| Real OpenStack | the production path against a live cloud | **manual, billable** |

## Required Verification Matrix

| Change scope | Commands |
|---|---|
| Frontend only | `npm run typecheck` · `npm run lint` · `npm run test:unit` |
| Rust backend only | `cargo test --manifest-path src-tauri/Cargo.toml` |
| Cross-boundary | both of the above |
| Docker provider / bootstrap | `npm run backend:test:docker:real` |
| UI or command surface | `npm run tauri:e2e:provisioning` |
| Connect / disconnect | `npm run tauri:e2e:connect` (**elevated shell**) |
| DigitalOcean adapter or cloud-init | `npm run backend:test:do:real` (**billable**) |
| Amazon EC2 adapter or cloud-init | `npm run backend:test:ec2:real` (**billable**) |
| Linode adapter or cloud-init | `npm run backend:test:linode:real` (**billable**) |
| Google Compute Engine adapter or cloud-init | `npm run backend:test:gce:real` (**billable**) |
| OpenStack adapter or cloud-init | `npm run backend:test:openstack:real` (**manual, billable**) |

## Frontend unit lane

```bash
npm run test:unit
```

Runs on Node's built-in test runner with native type stripping — no test framework dependency.
Tests live in `tests/unit/*.test.mts` and import source modules directly by path.

Type stripping cannot handle JSX, so anything under test must live in a plain `.ts` module.
`src/lib/connection-display.ts` is the pattern: pure state-to-display logic pulled out of the
components, which is also what makes the "is this value real or a placeholder" question testable.

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

### Amazon EC2

```bash
GOATSTARE_AWS_TEST_ACCESS_KEY_ID=<key> \
GOATSTARE_AWS_TEST_SECRET_ACCESS_KEY=<secret> \
npm run backend:test:ec2:real
```

`GOATSTARE_AWS_TEST_REGION` overrides the default `us-east-1`.

**Creates real, billable resources** — a `t3.micro` instance *and* a public IPv4 address, which AWS
bills separately. The test destroys both on every exit path, including panics, and asserts only
after teardown.

EC2 leaves **two** resources behind rather than one: the instance and the security group created
alongside it. A security group cannot be deleted until its instance has finished releasing its
network interface, so teardown waits for termination first. The adapter reports a group it could not
remove rather than swallowing it — a silent success there is how an account accumulates orphans.

Confirm nothing is left over if a run is interrupted:

```bash
aws ec2 describe-instances --region us-east-1 \
  --filters "Name=tag:goatstare,Values=managed" "Name=instance-state-name,Values=pending,running,stopping,stopped"
aws ec2 describe-security-groups --region us-east-1 --filters "Name=tag:goatstare,Values=managed"
```

Every resource this app creates carries the `goatstare=managed` tag, which is what makes that sweep
possible.

The lane asserts an instance reaches ready with an endpoint and a locally-generated server key, that
`external_ref` records `<region>/<instance-id>`, and that the rendered client config carries a full
tunnel — with nothing read back off the server. There is no EC2 handshake test: the handshake lane
proves cloud-init brings WireGuard up from the baked-in payload, and both providers render the
*identical* script through `render_cloud_init`, so the DigitalOcean handshake test already covers
that behavior. What differs for EC2 is resource orchestration, which is what this lane exercises.

The client runs in a container so no Windows elevation is needed. It is the *client*; nothing execs
into the provisioned droplet.

### Linode

```bash
GOATSTARE_LINODE_TEST_TOKEN=<personal access token> npm run backend:test:linode:real
```

`GOATSTARE_LINODE_TEST_REGION` overrides the default `us-ord`. The token needs read/write access to
Linodes.

**Creates a real, billable resource** — a `g6-nanode-1`, destroyed on every exit path including
panics, with assertions running only after teardown. Linode bills hourly up to the monthly cap, and
**keeps billing while an instance is powered off**, so a stranded instance is not free.

The lane asserts an instance reaches ready with an endpoint and a locally-generated server key, that
`external_ref` records a bare numeric instance id, and that the rendered client config carries a
full tunnel and the peer's preshared key — with nothing read back off the server.

The region is the part worth watching. `user_data` is accepted by the create call in every region
but only *honoured* where the Metadata service runs; elsewhere it is silently ignored and the
instance boots healthy and never bootstraps. `list_regions` filters on the `Metadata` capability,
and this lane is what proves the filter is pointed at the right thing.

Confirm nothing is left over if a run is interrupted:

```bash
curl -s -H "Authorization: Bearer $GOATSTARE_LINODE_TEST_TOKEN" \
  https://api.linode.com/v4/linode/instances | grep goatstare
```

Every instance this app creates carries the `goatstare` tag.

### Google Compute Engine

```bash
GOATSTARE_GCE_TEST_SERVICE_ACCOUNT_FILE=/path/to/key.json npm run backend:test:gce:real
```

`GOATSTARE_GCE_TEST_ZONE` overrides the default `us-central1-a`. The variable holds a **path**, not
the key itself: a multi-KB JSON blob in an environment variable is hostile to quote on Windows.

The service account needs the Compute Admin role, and the project needs billing enabled and the
Compute Engine API turned on. Organisation-managed Google accounts often block service-account key
creation outright (`iam.disableServiceAccountKeyCreation`), in which case this lane cannot run
without a personal project.

**Creates real, billable resources** — an `e2-micro` *and* its external IPv4, which Google has
billed hourly since February 2024. The test destroys both on every exit path, including panics, and
asserts only after teardown.

GCE leaves **two** resources behind rather than one: the instance and the firewall rule opened for
it. The boot disk is created with `autoDelete`, so it goes with the instance — a disk left behind is
the classic way a "deleted" GCE node keeps costing money. The adapter reports a rule it could not
remove rather than swallowing it; a leftover rule is not billable, but it blocks a later node from
reusing the name.

The lane also asserts the **MTU hint** reaches the node row and the rendered config. GCE instance
NICs run at 1460 rather than 1500, so the default 1420 tunnel MTU is 40 bytes too large — a tunnel
without the hint connects and then stalls on large transfers, which is the failure mode hardest to
tell from a slow network.

Confirm nothing is left over if a run is interrupted:

```bash
gcloud compute instances list --filter="labels.goatstare=managed"
gcloud compute firewall-rules list --filter="name~'-wg$'"
```

Expect the exit IP's geolocation to disagree with the zone. Google's published region is where the
hardware is; geo-IP databases frequently place GCE addresses elsewhere. That is a property of the
provider, not a bug in the adapter.

### OpenStack — a manual lane

```bash
GOATSTARE_OPENSTACK_TEST_AUTH_URL=https://identity.example.com/v3 \
GOATSTARE_OPENSTACK_TEST_APPLICATION_CREDENTIAL_ID=<id> \
GOATSTARE_OPENSTACK_TEST_APPLICATION_CREDENTIAL_SECRET=<secret> \
GOATSTARE_OPENSTACK_TEST_FLAVOR=<flavor id> \
npm run backend:test:openstack:real
```

`GOATSTARE_OPENSTACK_TEST_REGION` is optional on a single-region cloud and required on the rest.
There is no default flavor: flavor ids are operator-defined, so no guess is right across clouds —
list them with `openstack flavor list` and pick the smallest that has ~1 GB of RAM.

**This lane is manual, and deliberately so.** The other real lanes each target one vendor whose API
behaves one way, so running them repeatedly against that vendor is meaningful. OpenStack is software
many operators run, and they differ in exactly the places this adapter absorbs — whether a server
gets a routable address directly or needs a floating IP, what images are called, which flavors
exist. **Running this against a second cloud is worth more than running it twice against one.**
Rackspace's OpenStack Flex is the reference target. Standing up DevStack in CI is out of scope: a
single default configuration would not exercise the variation the adapter exists for.

**Creates billable resources on a commercial cloud** — a server, and on most clouds a floating IP.
The test destroys both on every exit path, including panics, and asserts only after teardown.

Teardown order is forced by Neutron and is the part most worth watching: the security group cannot
be deleted while the server's port still references it, so the run deletes the server, waits for it
to disappear, then releases the floating IP and removes the group. Ids for the floating IP and the
group are **rediscovered** at teardown rather than remembered, so the run still does the right thing
if an operator removed one by hand. Only groups named `goatstare-*` are candidates — a group the
operator attached themselves is left alone.

Confirm nothing is left over if a run is interrupted:

```bash
openstack server list --long | grep goatstare
openstack floating ip list --status DOWN
openstack security group list | grep goatstare
```

Every server this app creates carries the `goatstare=managed` metadata key.

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
