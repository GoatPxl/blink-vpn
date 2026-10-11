import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildConnectivityChecks,
  describeKillSwitch,
  describeTunnelConfig,
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
    phase: "connected",
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

describe("describeTunnelConfig", () => {
  const CONFIG = [
    "[Interface]",
    "PrivateKey = [REDACTED]",
    "Address = 10.66.0.2/32",
    "DNS = 9.9.9.9",
    "",
    "[Peer]",
    "PublicKey = serverpub",
    "AllowedIPs = 0.0.0.0/0",
    "Endpoint = 203.0.113.10:51820",
    "PersistentKeepalive = 25",
  ].join("\n");

  function bundle(overrides: Record<string, unknown> = {}): DiagnosticsBundle {
    return {
      generated_at: "1786800000000",
      providers: [],
      nodes: [],
      tunnels: [],
      activity: [],
      redacted_notes: [],
      wireguard_debug: {
        commands: [],
        last_import_tunnel_name: "wg-primary",
        last_import_config: CONFIG,
        last_import_config_temp_path: "C:\\Users\\me\\AppData\\Local\\Blink\\tunnels\\wg-primary.conf",
        ...overrides,
      },
    } as unknown as DiagnosticsBundle;
  }

  it("says it is unavailable before any diagnostics arrive", () => {
    const result = describeTunnelConfig(null);
    assert.equal(result.available, false);
    assert.equal(result.configText, null);
    assert.match(result.emptyReason, /waiting/i);
  });

  // Distinct from "not loaded yet": the app is running and simply has not built a config.
  it("distinguishes a loaded bundle with no config from an unloaded one", () => {
    const result = describeTunnelConfig(bundle({ last_import_config: null }));
    assert.equal(result.available, false);
    assert.match(result.emptyReason, /connect to a server/i);
  });

  it("exposes the config, its tunnel name and its source path", () => {
    const result = describeTunnelConfig(bundle());
    assert.equal(result.available, true);
    assert.equal(result.tunnelName, "wg-primary");
    assert.match(result.sourcePath ?? "", /wg-primary\.conf$/);
  });

  /**
   * Passes the backend's text through untouched.
   *
   * The card exists so a user can confirm a DNS or keepalive change reached the running tunnel, so
   * this must not filter the config down to a summary. That the private key is redacted is the
   * backend's job and is asserted there — see
   * `the_config_snapshot_hides_the_private_key_and_keeps_every_verifiable_setting`.
   */
  it("passes the settings through rather than summarising them", () => {
    const text = describeTunnelConfig(bundle()).configText ?? "";
    assert.ok(text.includes("DNS = 9.9.9.9"), "DNS must be checkable");
    assert.ok(text.includes("AllowedIPs = 0.0.0.0/0"), "the kill-switch route must be checkable");
    assert.ok(text.includes("PersistentKeepalive = 25"), "keepalive must be checkable");
    assert.equal(text, CONFIG, "the backend's text is shown verbatim");
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
