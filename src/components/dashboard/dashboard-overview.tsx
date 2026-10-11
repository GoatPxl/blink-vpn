"use client";

import { PixelIcon } from "@/components/ui/pixel-icon";
import { DashboardEmptyState } from "@/components/dashboard-empty-state";
import { Button, ConnectionTimer, PageChrome, formatHandshakeAge, type AppView, type HealthPanelData, type RecoveryState, type DashboardBusyAction, type TunnelRecord, type VpnNodeRecord } from "./shared";
import { describeEgress, describeIpv6, handshakeFreshness, type EgressTone } from "@/lib/connection-display";
import type { ConnectAttempt } from "./dashboard-provider";
import type { ReactNode } from "react";

/** Maps an egress verdict onto the theme's colour tokens. */
const EGRESS_TONE_VAR: Record<EgressTone, string> = {
  positive: "var(--secondary)",
  warning: "var(--tertiary)",
  muted: "var(--on-surface-variant)",
};

/**
 * WireGuard's cipher suite is fixed by the protocol — there is no negotiation and no backend
 * field reporting it, which is why this is a constant rather than a value read from a snapshot.
 *
 * It previously read "AES-256", which WireGuard does not use at all.
 */
const WIREGUARD_SUITE = "ChaCha20-Poly1305";
const WIREGUARD_SUITE_DETAIL = "Curve25519 · BLAKE2s";

/** Handshakes refresh on the keepalive interval, so this is the window a live peer stays within. */
const HANDSHAKE_FRESH_WINDOW_SECONDS = 180;

type ThroughputSample = {
  timestamp: number;
  downloadBytesPerSecond: number;
  uploadBytesPerSecond: number;
};

function formatRate(bytesPerSecond: number): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) {
    return "0 B/s";
  }

  const units = ["B/s", "KB/s", "MB/s", "GB/s"];
  let value = bytesPerSecond;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  const digits = value >= 100 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(digits)} ${units[unitIndex]}`;
}

function buildLinePath(values: number[], maxValue: number, width: number, height: number, paddingY: number): string {
  if (values.length === 0) {
    return "";
  }

  const safeMax = Math.max(1, maxValue);
  const innerHeight = Math.max(1, height - (paddingY * 2));

  if (values.length === 1) {
    const y = height - paddingY - ((values[0] / safeMax) * innerHeight);
    return `M0,${y.toFixed(2)} L${width},${y.toFixed(2)}`;
  }

  return values
    .map((value, index) => {
      const x = (index / (values.length - 1)) * width;
      const y = height - paddingY - ((value / safeMax) * innerHeight);
      return `${index === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");
}

/**
 * Re-polls the health snapshot on click.
 *
 * This used to be a bare decorative SVG with no handler — it looked actionable and did nothing.
 * Polling never elevates, so this cannot raise a UAC prompt.
 */
function RefreshButton({
  label,
  busy,
  onRefresh,
}: {
  label: string;
  busy: boolean;
  onRefresh: () => Promise<void>;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={busy}
      onClick={() => { void onRefresh(); }}
      className="rounded-full p-1 text-[var(--on-surface-variant)] transition hover:bg-[color:rgba(255,255,255,0.06)] hover:text-[var(--on-surface)] disabled:opacity-50"
    >
      <PixelIcon name="refresh" className={`h-4 w-4 ${busy ? "animate-spin" : ""}`} />
    </button>
  );
}

export function DashboardOverview({
  activeView,
  noServerConfigured,
  activeNode,
  connectedTunnel,
  health,
  tunnelConnected,
  throughputSamples,
  recovery,
  busyAction,
  connectAttempt,
  onConnectPrimary,
  onConnectTunnel,
  onDisconnectTunnel,
  onForceDisconnectNodeService,
  onRefresh,
  onRefreshHealth,
  devices,
  emptyState,
}: {
  activeView: AppView;
  noServerConfigured: boolean;
  activeNode: VpnNodeRecord | null;
  connectedTunnel: TunnelRecord | null;
  health: HealthPanelData | null;
  tunnelConnected: boolean;
  throughputSamples: ThroughputSample[];
  recovery: RecoveryState | null;
  busyAction: DashboardBusyAction;
  connectAttempt: ConnectAttempt | null;
  onConnectPrimary: () => Promise<void>;
  onConnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onDisconnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onForceDisconnectNodeService: (node: VpnNodeRecord) => Promise<void>;
  onRefresh: () => Promise<void>;
  onRefreshHealth: () => Promise<void>;
  /** The mobile devices card, rendered beneath the overview. */
  devices?: ReactNode;
  /** Replaces the default first-run prompt when no server exists. */
  emptyState?: ReactNode;
}) {
  if (noServerConfigured && activeView === "home") {
    return emptyState ?? <DashboardEmptyState />;
  }

  if (!(noServerConfigured && activeView === "home") && activeView === "home") {
    // Every connection widget reads the same live value, so the headline can no longer contradict
    // the handshake card beside it.
    // A connect in flight outranks everything else here: the user's internet is down during it, so
    // it is the one thing worth saying. It must not read as "Idle" — that is what made losing
    // connectivity look like a fault rather than a step.
    const isConnecting = connectAttempt !== null;
    const statusLabel = isConnecting
      ? "Status: Connecting"
      : recovery
        ? "Status: Attention"
        : tunnelConnected
          ? "Status: Secure"
          : connectedTunnel
            ? "Status: Disconnected"
            : "Status: Idle";
    const statusTone = isConnecting
      ? "var(--tertiary)"
      : recovery
        ? "var(--tertiary)"
        : tunnelConnected
          ? "var(--secondary)"
          : "var(--on-surface-variant)";
    const buttonLabel = isConnecting
      ? "Connecting"
      : tunnelConnected
        ? (busyAction === "disconnect-tunnel" ? "Disconnecting" : "Disconnect")
        : "Connect";
    const buttonTone = tunnelConnected ? "destructive" : "primary";

    // Known only when this app established the session. A tunnel already running at startup has
    // no recorded start, and inventing one would render a timer counting from an arbitrary moment.
    const sessionStartedAt = tunnelConnected ? health?.connected_at ?? null : null;

    const latestThroughput = throughputSamples[throughputSamples.length - 1] ?? null;
    const hasThroughput = throughputSamples.length > 1;
    const downloadSeries = throughputSamples.map((point) => point.downloadBytesPerSecond);
    const uploadSeries = throughputSamples.map((point) => point.uploadBytesPerSecond);
    const graphMax = Math.max(1, ...downloadSeries, ...uploadSeries);
    const downloadPath = buildLinePath(downloadSeries, graphMax, 1000, 180, 10);
    const uploadPath = buildLinePath(uploadSeries, graphMax, 1000, 180, 10);
    // Measured from the samples themselves rather than assumed from the poll interval, which
    // drifts whenever a poll runs slow.
    const windowSeconds = hasThroughput
      ? Math.round((throughputSamples[throughputSamples.length - 1].timestamp - throughputSamples[0].timestamp) / 1000)
      : 0;

    // Handshake age needs administrator, so it is genuinely absent on most runs. Null means no
    // bar at all rather than a placeholder width.
    const freshness = handshakeFreshness(health?.handshake_age_seconds, HANDSHAKE_FRESH_WINDOW_SECONDS);

    const egress = describeEgress({
      tunnelConnected,
      observedEgress: health?.external_ipv4 ?? null,
      serverIp: health?.endpoint_ip ?? activeNode?.public_ipv4 ?? null,
    });
    const egressTone = EGRESS_TONE_VAR[egress.tone];
    const ipv6 = describeIpv6({ tunnelConnected, observedIpv6: health?.external_ipv6 ?? null });
    const isHealthRefreshing = busyAction === "refresh-health";

    return (
      <PageChrome>
        <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_340px]">
          <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-6 py-8">
            <div className="flex items-start justify-between gap-4">
              <div>
                <div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-[0.26em]" style={{ color: statusTone }}>
                  <span className="h-2.5 w-2.5 rounded-full" style={{ backgroundColor: statusTone }} />
                  <span>{statusLabel}</span>
                </div>
                <h2 className="mt-3 text-[56px] font-semibold leading-none tracking-tight text-[var(--on-surface)]">{activeNode?.name ?? "No active node"}</h2>
                <p className="mt-2 text-sm text-[var(--on-surface-variant)]">Server: {activeNode ? `${activeNode.external_ref ?? activeNode.id}` : "n/a"}</p>
              </div>
              <Button
                tone={buttonTone}
                onClick={() => {
                  if (!activeNode) {
                    void onRefresh();
                    return;
                  }

                  if (tunnelConnected) {
                    // The label says Disconnect, so the click must never connect. With no tunnel
                    // record loaded yet, the node-wide stop is the only handle available.
                    void (connectedTunnel
                      ? onDisconnectTunnel(connectedTunnel)
                      : onForceDisconnectNodeService(activeNode));
                    return;
                  }
                  if (connectedTunnel) {
                    void onConnectTunnel(connectedTunnel);
                    return;
                  }
                  void onConnectPrimary();
                }}
                disabled={busyAction !== null || isConnecting || !activeNode}
              >
                <PixelIcon name="power" className="h-4 w-4" />
                {buttonLabel}
              </Button>
            </div>

            <div className="mt-24 grid grid-cols-2 gap-6">
              <div>
                <div className="text-[11px] uppercase tracking-[0.22em] text-[var(--on-surface-variant)]">Connection Time</div>
                <div className="mt-1 font-mono text-[22px] font-semibold text-[var(--on-surface)]">
                  {sessionStartedAt ? <ConnectionTimer startedAt={sessionStartedAt} /> : "--:--:--"}
                </div>
                {tunnelConnected && sessionStartedAt === null ? (
                  <div className="mt-1 text-[11px] text-[var(--on-surface-variant)]" title="This tunnel was already running when the app started, so its start time is not known.">
                    Started before this session
                  </div>
                ) : null}
              </div>
              <div>
                <div className="text-[11px] uppercase tracking-[0.22em] text-[var(--on-surface-variant)]">Encryption Protocol</div>
                <div className="mt-1 flex items-center gap-2 text-[22px] font-semibold text-[var(--on-surface)]"><PixelIcon name="key" className="h-5 w-5 text-[var(--primary)]" />{WIREGUARD_SUITE}</div>
                <div className="mt-1 text-[11px] text-[var(--on-surface-variant)]">WireGuard · {WIREGUARD_SUITE_DETAIL}</div>
              </div>
            </div>
          </div>

          <div className="grid gap-3">
            <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-5 py-4">
              <div className="flex items-start justify-between">
                <div>
                  <div className="text-sm text-[var(--on-surface-variant)]">My IP Address</div>
                  <div className="mt-3 text-[18px] font-semibold tracking-tight text-[var(--on-surface)]">{egress.value}</div>
                </div>
                <RefreshButton label="Re-check my IP address" busy={isHealthRefreshing} onRefresh={onRefreshHealth} />
              </div>
              <div className="mt-3 flex items-center gap-2 text-[12px]" style={{ color: egressTone }}>
                <span className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: egressTone }} />
                {egress.caption}
              </div>
              {ipv6 ? (
                <div className="mt-4 border-t border-[color:var(--outline-variant)] pt-3">
                  <div className="text-[11px] uppercase tracking-[0.22em] text-[var(--on-surface-variant)]">IPv6</div>
                  <div className="mt-1 truncate text-[13px] font-medium text-[var(--on-surface)]" title={ipv6.value}>{ipv6.value}</div>
                  <div className="mt-1 flex items-center gap-2 text-[11px]" style={{ color: EGRESS_TONE_VAR[ipv6.tone] }}>
                    <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ backgroundColor: EGRESS_TONE_VAR[ipv6.tone] }} />
                    {ipv6.caption}
                  </div>
                </div>
              ) : null}
            </div>

            <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-5 py-4">
              <div className="flex items-start justify-between">
                <div>
                  <div className="text-sm text-[var(--on-surface-variant)]">Latest Handshake</div>
                  <div className="mt-3 text-[18px] font-semibold tracking-tight text-[var(--on-surface)]">{health?.tunnel_status ?? "Pending"}</div>
                  <div className="mt-1 text-xs text-[var(--on-surface-variant)]">Last handshake: {formatHandshakeAge(health?.handshake_age_seconds, health?.tunnel_status)}</div>
                </div>
                <RefreshButton label="Refresh handshake state" busy={isHealthRefreshing} onRefresh={onRefreshHealth} />
              </div>
              {freshness === null ? (
                <div
                  className="mt-5 text-[11px] text-[var(--on-surface-variant)]"
                  title="Handshake age comes from the WireGuard tunnel service's pipe, which is restricted to administrators. The tunnel itself is unaffected."
                >
                  {tunnelConnected ? "Handshake age requires administrator" : "No active handshake"}
                </div>
              ) : (
                <>
                  <div className="mt-5 h-1.5 overflow-hidden rounded-full bg-[color:rgba(255,255,255,0.07)]">
                    <div className="h-full rounded-full bg-[var(--primary)]" style={{ width: `${freshness.toFixed(0)}%` }} />
                  </div>
                  <div className="mt-1 text-right text-[10px] text-[var(--on-surface-variant)]">Handshake freshness</div>
                </>
              )}
            </div>
          </div>
        </div>

        <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-6 py-5">
          <div className="flex items-center justify-between gap-4">
            <h3 className="text-[18px] font-semibold tracking-tight text-[var(--on-surface)]">
              Network Throughput{windowSeconds > 0 ? ` (${windowSeconds}s)` : ""}
            </h3>
            {hasThroughput ? (
              <div className="flex items-center gap-4 text-[11px] text-[var(--on-surface-variant)]">
                <span className="flex items-center gap-2"><span className="h-2.5 w-2.5 rounded-full bg-[var(--primary)]" />DL: {formatRate(latestThroughput?.downloadBytesPerSecond ?? 0)}</span>
                <span className="flex items-center gap-2"><span className="h-2.5 w-2.5 rounded-full bg-[var(--tertiary)]" />UL: {formatRate(latestThroughput?.uploadBytesPerSecond ?? 0)}</span>
              </div>
            ) : null}
          </div>
          <div className="relative mt-5 h-[180px] rounded-[8px] border border-[color:rgba(255,255,255,0.03)] bg-[linear-gradient(180deg,rgba(11,14,21,0.96),rgba(11,14,21,0.96))] px-2 py-4">
            {hasThroughput ? (
              <>
                {/* Absolute scale, so the shape of the line means something. */}
                <div className="pointer-events-none absolute left-3 top-2 text-[10px] text-[var(--on-surface-variant)]">{formatRate(graphMax)}</div>
                <div className="pointer-events-none absolute bottom-2 left-3 text-[10px] text-[var(--on-surface-variant)]">0 B/s</div>
                <svg viewBox="0 0 1000 180" className="h-full w-full text-[var(--on-surface-variant)]" preserveAspectRatio="none" aria-hidden="true">
                  <g opacity="0.16" stroke="currentColor">
                    <path d="M0 28H1000" />
                    <path d="M0 68H1000" />
                    <path d="M0 108H1000" />
                    <path d="M0 148H1000" />
                  </g>
                  <path d={downloadPath} fill="none" stroke="rgba(171,199,255,0.95)" strokeWidth="3" />
                  <path d={uploadPath} fill="none" stroke="rgba(255,185,95,0.9)" strokeWidth="2.5" />
                </svg>
              </>
            ) : (
              // An empty chart, rather than a flat zero line: no measurement is a different claim
              // from a measured zero, and the two used to look identical.
              <div className="flex h-full items-center justify-center text-[12px] text-[var(--on-surface-variant)]">
                {tunnelConnected ? "Sampling throughput…" : "Connect to measure throughput"}
              </div>
            )}
          </div>
        </div>
        {devices}
      </PageChrome>
    );
  }

  return null;
}
