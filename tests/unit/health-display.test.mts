import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildConnectivityChecks,
  buildLogLines,
  describeKillSwitch,
  formatDropAge,
  type ProviderAccess,
} from "../../src/lib/health-display.ts";
import type { DiagnosticsBundle, HealthPanelData, KillSwitchState, WireGuardReadiness } from "../../src/lib/tauri.ts";

const SERVER_IP = "203.0.113.10";

function killSwitch(overrides: Partial<KillSwitchState> = {}): KillSwitchState {
  return {
    active: true,
    blocks_ipv4: true,
    blocks_ipv6: true,
    reason: "active",
    routes: ["0.0.0.0/0", "::/0"],
    ...overrides,
  };
}

function health(overrides: Partial<HealthPanelData> = {}): HealthPanelData {
  return {
    node_id: "node-1",
    tunnel_id: "tunnel-1",
    endpoint_ip: SERVER_IP,
    handshake_age_seconds: null,
    rx_bytes: 1000,
    tx_bytes: 500,
    external_ipv4: SERVER_IP,
    external_ipv6: null,
    tunnel_status: "connected",
    metrics_source: "adapter",
    connected_at: "2026-08-16T10:00:00.000Z",
    kill_switch: killSwitch(),
    unexpected_drop: null,
    ...overrides,
  };
}

function readiness(overrides: Partial<WireGuardReadiness> = {}): WireGuardReadiness {
  return {
    installed: true,
    executable_found: true,
    service_available: true,
    version: "wireguard-tools v1.0.20210914",
    app_version: "0.1.0",
    message: "WireGuard runtime is installed and ready.",
    action_guidance: "Continue onboarding.",
    ...overrides,
  };
}

describe("describeKillSwitch", () => {
  it("reports active protection and names the covered families", () => {
    const result = describeKillSwitch(killSwitch());
    assert.equal(result.status, "pass");
    assert.equal(result.headline, "Active");
    assert.match(result.detail, /IPv4 and IPv6/);
    // The limitation has to travel with the claim, or "Active" reads as always-on.
    assert.match(result.detail, /only while the tunnel service runs/i);
  });

  // A Docker dev tunnel has its default routes stripped, so nothing is blocked. Claiming
  // protection here would be the most dangerous lie on the page.
  it("reports no protection when routes are scoped", () => {
    const result = describeKillSwitch(killSwitch({
      active: false, blocks_ipv4: false, blocks_ipv6: false, reason: "routes_scoped", routes: ["10.66.0.0/24"],
    }));

    assert.equal(result.status, "fail");
    assert.equal(result.headline, "Not protected");
    assert.match(result.detail, /10\.66\.0\.0\/24/);
  });

  it("distinguishes a stopped tunnel from scoped routes", () => {
    const result = describeKillSwitch(killSwitch({ active: false, reason: "tunnel_not_running" }));
    assert.equal(result.status, "warn");
    assert.equal(result.headline, "Inactive");
  });

  it("reports partial coverage when only IPv4 is routed", () => {
    const result = describeKillSwitch(killSwitch({ blocks_ipv6: false, routes: ["0.0.0.0/0"] }));
    assert.match(result.detail, /IPv4 traffic/);
    assert.doesNotMatch(result.detail, /IPv6/);
  });

  it("is unknown before any health data arrives", () => {
    assert.equal(describeKillSwitch(null).status, "unknown");
  });
});

describe("buildConnectivityChecks", () => {
  const healthy = {
    readiness: readiness(),
    providerAccess: "ready" as ProviderAccess,
    health: health({ handshake_age_seconds: 20 }),
    tunnelConnected: true,
  };

  it("passes every check when everything is genuinely healthy", () => {
    const checks = buildConnectivityChecks(healthy);
    const failing = checks.filter((check) => check.status !== "pass");
    assert.deepEqual(failing, [], "a fully healthy state should show no faults");
  });

  // The bug this replaces: the rows were a literal array with `active: true`, so they reported
  // success with no tunnel, no token, and no node.
  it("fails every check when nothing is configured", () => {
    const checks = buildConnectivityChecks({
      readiness: readiness({ installed: false, message: "WireGuard for Windows is missing." }),
      providerAccess: "missing",
      health: null,
      tunnelConnected: false,
    });

    for (const id of ["wireguard", "provider", "tunnel"]) {
      const check = checks.find((entry) => entry.id === id);
      assert.equal(check?.status, "fail", `${id} must be able to fail`);
    }
    assert.ok(!checks.some((check) => check.status === "pass"), "nothing should claim to pass");
  });

  it("fails the provider check on an invalid token", () => {
    const checks = buildConnectivityChecks({ ...healthy, providerAccess: "invalid" });
    assert.equal(checks.find((entry) => entry.id === "provider")?.status, "fail");
  });

  // Absent handshake age is an elevation limit, not a fault — reporting it as failure would cry
  // wolf on every unelevated run.
  it("treats an unreadable handshake as unknown, not failed", () => {
    const checks = buildConnectivityChecks({ ...healthy, health: health({ handshake_age_seconds: null }) });
    const check = checks.find((entry) => entry.id === "handshake");
    assert.equal(check?.status, "unknown");
    assert.match(check!.detail, /administrator/i);
  });

  it("warns when the handshake is stale", () => {
    const checks = buildConnectivityChecks({ ...healthy, health: health({ handshake_age_seconds: 600 }) });
    assert.equal(checks.find((entry) => entry.id === "handshake")?.status, "warn");
  });

  it("fails the egress check when traffic does not exit through the server", () => {
    const checks = buildConnectivityChecks({
      ...healthy,
      health: health({ external_ipv4: "198.51.100.7", handshake_age_seconds: 20 }),
    });
    assert.equal(checks.find((entry) => entry.id === "egress")?.status, "fail");
  });

  it("fails the IPv6 check when IPv6 is reachable while connected", () => {
    const checks = buildConnectivityChecks({
      ...healthy,
      health: health({ external_ipv6: "2001:db8::1", handshake_age_seconds: 20 }),
    });
    const check = checks.find((entry) => entry.id === "ipv6");
    assert.equal(check?.status, "fail");
    assert.match(check!.detail, /bypassing/i);
  });

  it("passes the IPv6 check when no IPv6 egress exists", () => {
    const checks = buildConnectivityChecks(healthy);
    assert.equal(checks.find((entry) => entry.id === "ipv6")?.status, "pass");
  });
});

describe("buildLogLines", () => {
  const diagnostics = {
    generated_at: "1786800000000",
    providers: [],
    nodes: [],
    tunnels: [],
    init_runs: [
      { id: 1, node_id: "node-1", provider_slug: "digitalocean", init_kind: "cloud-init", stage: "create.start", outcome: "info", output: "init flow started", created_at: "2026-08-16T04:53:54.905Z" },
      { id: 2, node_id: "node-1", provider_slug: "digitalocean", init_kind: "cloud-init", stage: "create.error", outcome: "error", output: "droplet create rejected", created_at: "2026-08-16T04:53:56.488Z" },
    ],
    redacted_notes: ["node node-1 failed: bad name"],
    wireguard_debug: {
      commands: ["1786800000 | sc query WireGuardTunnel$wg-node-1", "1786800005 | wg show wg-node-1 dump"],
      last_import_tunnel_name: null,
      last_import_config_redacted: null,
      last_import_config_temp_path: null,
    },
  } as unknown as DiagnosticsBundle;

  it("returns nothing before diagnostics arrive", () => {
    assert.deepEqual(buildLogLines(null, "all"), []);
  });

  it("includes real command trace and init runs on the All tab", () => {
    const lines = buildLogLines(diagnostics, "all");
    assert.ok(lines.some((line) => line.message.includes("sc query")), "real commands should appear");
    assert.ok(lines.some((line) => line.message.includes("init flow started")));
    // The fixture this replaces invented a subsystem that does not exist in the codebase.
    assert.ok(!lines.some((line) => line.source.includes("NetGuard")));
  });

  it("narrows to the command trace on the WireGuard tab", () => {
    const lines = buildLogLines(diagnostics, "wireguard");
    assert.ok(lines.length > 0);
    assert.ok(lines.every((line) => line.source === "WireGuard"), "only WireGuard entries");
  });

  it("narrows to failures on the Errors tab", () => {
    const lines = buildLogLines(diagnostics, "errors");
    assert.ok(lines.length > 0);
    assert.ok(!lines.some((line) => line.message.includes("init flow started")), "successes are excluded");
    assert.ok(lines.some((line) => line.message.includes("droplet create rejected")));
    assert.ok(lines.some((line) => line.message.includes("bad name")), "recovery notes are failures too");
  });

  it("renders trace timestamps as clock times", () => {
    const line = buildLogLines(diagnostics, "wireguard")[0];
    assert.match(line.timestamp, /^\d{2}:\d{2}:\d{2}$/);
  });
});

describe("formatDropAge", () => {
  it("scales from seconds to hours", () => {
    const now = 1_000_000_000;
    assert.equal(formatDropAge(now - 5_000, now), "5s ago");
    assert.equal(formatDropAge(now - 120_000, now), "2m ago");
    assert.equal(formatDropAge(now - 7_200_000, now), "2h ago");
  });

  it("never reports a negative age from clock skew", () => {
    assert.equal(formatDropAge(2_000, 1_000), "0s ago");
  });
});
