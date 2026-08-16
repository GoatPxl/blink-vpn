/**
 * Pure presentation logic for the Kill Switch page.
 *
 * Free of JSX and React so it can be unit tested directly. The point of testing it is that the
 * page it feeds is a safety surface: its previous rows were a literal array with `active: true`
 * written in, so every check passed even with no tunnel, no token, and no node. Every derivation
 * here must be able to return a failing state.
 */

// Explicit extension: this is a value import, so Node's test runner has to resolve it at runtime
// (type-only imports are erased and need none).
import { describeEgress, describeIpv6 } from "./connection-display.ts";
import type { DiagnosticsBundle, HealthPanelData, KillSwitchState, WireGuardReadiness } from "./tauri";

export type CheckStatus = "pass" | "fail" | "warn" | "unknown";

export type ConnectivityCheck = {
  id: string;
  title: string;
  detail: string;
  status: CheckStatus;
};

/** Provider access states the dashboard already tracks, narrowed to what this page needs. */
export type ProviderAccess = "ready" | "missing" | "invalid" | "unknown";

/**
 * Plain-language account of whether traffic is protected right now.
 *
 * Deliberately explicit about the limitation: WireGuard's filters are torn down with the tunnel
 * service, so "active" means "while this service runs", not "always".
 */
export function describeKillSwitch(state: KillSwitchState | null | undefined): {
  headline: string;
  detail: string;
  status: CheckStatus;
} {
  if (!state) {
    return { headline: "Unknown", detail: "No health data yet.", status: "unknown" };
  }

  if (state.reason === "routes_scoped") {
    return {
      headline: "Not protected",
      detail: `This tunnel only routes ${state.routes.join(", ") || "a limited range"}, so other traffic uses your normal connection. WireGuard engages leak protection only for a full-tunnel route.`,
      status: "fail",
    };
  }

  if (state.reason === "tunnel_not_running") {
    return {
      headline: "Inactive",
      detail: "Protection applies only while the tunnel service is running. Connect to enable it.",
      status: "warn",
    };
  }

  const families = [state.blocks_ipv4 ? "IPv4" : null, state.blocks_ipv6 ? "IPv6" : null]
    .filter(Boolean)
    .join(" and ");

  return {
    headline: "Active",
    detail: `WireGuard is blocking ${families} traffic that does not go through the tunnel. This lasts only while the tunnel service runs — if it stops, protection stops with it.`,
    status: "pass",
  };
}

/**
 * The connectivity checks, each derived from a real measurement.
 *
 * Order runs outward: local tooling, then credentials, then the tunnel, then what the internet
 * actually sees.
 */
export function buildConnectivityChecks(input: {
  readiness: WireGuardReadiness | null;
  providerAccess: ProviderAccess;
  health: HealthPanelData | null;
  tunnelConnected: boolean;
}): ConnectivityCheck[] {
  const { readiness, providerAccess, health, tunnelConnected } = input;
  const checks: ConnectivityCheck[] = [];

  checks.push({
    id: "wireguard",
    title: "WireGuard installed",
    detail: readiness
      ? readiness.installed
        ? `Runtime available${readiness.version ? ` · ${readiness.version}` : ""}`
        : readiness.message
      : "Not checked yet.",
    status: readiness ? (readiness.installed ? "pass" : "fail") : "unknown",
  });

  checks.push({
    id: "provider",
    title: "Provider credential",
    detail: {
      ready: "Token stored and validated.",
      missing: "No provider token stored. Save one in Settings.",
      invalid: "Stored token was rejected. Re-validate it in Settings.",
      unknown: "Provider access not checked yet.",
    }[providerAccess],
    status: providerAccess === "ready" ? "pass" : providerAccess === "unknown" ? "unknown" : "fail",
  });

  checks.push({
    id: "tunnel",
    title: "Tunnel service",
    detail: tunnelConnected
      ? "Running and reporting connected."
      : health
        ? `Not running (${health.tunnel_status ?? "no status"}).`
        : "No tunnel selected.",
    status: tunnelConnected ? "pass" : "fail",
  });

  // Handshake age needs administrator, so absent is the ordinary case rather than a failure.
  const handshakeAge = health?.handshake_age_seconds ?? null;
  checks.push({
    id: "handshake",
    title: "Handshake",
    detail: !tunnelConnected
      ? "No active tunnel."
      : handshakeAge === null
        ? "Cannot be read without administrator; the tunnel itself is unaffected."
        : `Last handshake ${handshakeAge}s ago.`,
    status: !tunnelConnected
      ? "fail"
      : handshakeAge === null
        ? "unknown"
        : handshakeAge <= 180
          ? "pass"
          : "warn",
  });

  const egress = describeEgress({
    tunnelConnected,
    observedEgress: health?.external_ipv4 ?? null,
    serverIp: health?.endpoint_ip ?? null,
  });
  checks.push({
    id: "egress",
    title: "Egress address",
    detail: egress.caption,
    status: egress.tone === "positive" ? "pass" : egress.tone === "warning" ? "fail" : tunnelConnected ? "warn" : "unknown",
  });

  // The tunnel is IPv4-only, so reachable IPv6 while connected is traffic going around it.
  const ipv6 = describeIpv6({ tunnelConnected, observedIpv6: health?.external_ipv6 ?? null });
  if (ipv6) {
    checks.push({
      id: "ipv6",
      title: "IPv6 leak",
      detail: ipv6.caption,
      status: ipv6.tone === "warning" ? "fail" : ipv6.tone === "positive" ? "pass" : "unknown",
    });
  }

  return checks;
}

/**
 * What the tunnel service was actually handed, for the Tunnel Configuration card.
 *
 * The private key is already replaced with a redaction marker by the backend. Everything else —
 * `Address`, `DNS`, `AllowedIPs`, `Endpoint`, `PersistentKeepalive` — is shown, because verifying
 * that a settings change reached the running tunnel is the entire point of the card.
 */
export function describeTunnelConfig(diagnostics: DiagnosticsBundle | null): {
  available: boolean;
  tunnelName: string | null;
  configText: string | null;
  sourcePath: string | null;
  emptyReason: string;
} {
  const debug = diagnostics?.wireguard_debug;
  const configText = debug?.last_import_config_redacted ?? null;

  return {
    available: Boolean(configText),
    tunnelName: debug?.last_import_tunnel_name ?? null,
    configText,
    sourcePath: debug?.last_import_config_temp_path ?? null,
    emptyReason: diagnostics
      ? "No tunnel config has been rendered yet. Connect to a server and it appears here."
      : "Waiting for the first diagnostics read.",
  };
}

/** How long ago a drop happened, for the alert copy. */
export function formatDropAge(observedAtEpochMs: number, now: number): string {
  const seconds = Math.max(0, Math.floor((now - observedAtEpochMs) / 1000));
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  return `${Math.floor(minutes / 60)}h ago`;
}
