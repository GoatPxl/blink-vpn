"use client";

import { DashboardEmptyState } from "@/components/dashboard-empty-state";
import { AppIcon, Button, PageChrome, formatHandshakeAge, type AppView, type HealthPanelData, type RecoveryState, type DashboardBusyAction, type TunnelRecord, type VpnNodeRecord } from "./shared";

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

export function DashboardOverview({
  activeView,
  noServerConfigured,
  activeNode,
  connectedTunnel,
  health,
  canConnect,
  throughputSamples,
  recovery,
  busyAction,
  onConnectPrimary,
  onConnectTunnel,
  onDisconnectTunnel,
  onRefresh,
}: {
  activeView: AppView;
  noServerConfigured: boolean;
  activeNode: VpnNodeRecord | null;
  connectedTunnel: TunnelRecord | null;
  health: HealthPanelData | null;
  canConnect: boolean;
  throughputSamples: ThroughputSample[];
  recovery: RecoveryState | null;
  busyAction: DashboardBusyAction;
  onConnectPrimary: () => Promise<void>;
  onConnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onDisconnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onRefresh: () => Promise<void>;
}) {
  if (noServerConfigured && activeView === "dashboard") {
    return <DashboardEmptyState />;
  }

  if (!(noServerConfigured && activeView === "dashboard") && activeView === "dashboard") {
    const tunnelConnected = connectedTunnel?.status.toLowerCase() === "connected";
    const statusLabel = recovery
      ? "Status: Attention"
      : tunnelConnected
        ? "Status: Secure"
        : connectedTunnel
          ? "Status: Disconnected"
          : "Status: Idle";
    const isConnectBusy = busyAction === "connect-tunnel" || busyAction === "create-tunnel";
    const buttonLabel = connectedTunnel
      ? canConnect
        ? (isConnectBusy ? "Connecting" : "Connect")
        : (busyAction === "disconnect-tunnel" ? "Disconnecting" : "Disconnect")
      : (isConnectBusy ? "Connecting" : "Connect");
    const buttonTone = connectedTunnel && !canConnect ? "danger" : "primary";
    const latestThroughput = throughputSamples[throughputSamples.length - 1] ?? null;
    const graphPoints = throughputSamples.length > 0
      ? throughputSamples
      : Array.from({ length: 12 }, (_, index) => ({
        timestamp: index,
        downloadBytesPerSecond: 0,
        uploadBytesPerSecond: 0,
      }));
    const downloadSeries = graphPoints.map((point) => point.downloadBytesPerSecond);
    const uploadSeries = graphPoints.map((point) => point.uploadBytesPerSecond);
    const graphMax = Math.max(1, ...downloadSeries, ...uploadSeries);
    const downloadPath = buildLinePath(downloadSeries, graphMax, 1000, 180, 10);
    const uploadPath = buildLinePath(uploadSeries, graphMax, 1000, 180, 10);
    const signalStrength = health?.handshake_age_seconds === null || health?.handshake_age_seconds === undefined
      ? 14
      : Math.max(8, Math.min(100, 100 - (Math.min(health.handshake_age_seconds, 240) / 2.4)));

    return (
      <PageChrome>
        <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_340px]">
          <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-6 py-8">
            <div className="flex items-start justify-between gap-4">
              <div>
                <div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-[0.26em] text-[var(--secondary)]">
                  <span className="h-2.5 w-2.5 rounded-full bg-[var(--secondary)] shadow-[0_0_0_6px_rgba(78,222,163,0.12)]" />
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

                  if (connectedTunnel) {
                    if (canConnect) {
                      void onConnectTunnel(connectedTunnel);
                    } else {
                      void onDisconnectTunnel(connectedTunnel);
                    }
                    return;
                  }
                  void onConnectPrimary();
                }}
                disabled={busyAction !== null || !activeNode}
              >
                <AppIcon name="power" className="h-4 w-4" />
                {buttonLabel}
              </Button>
            </div>

            <div className="mt-24 grid grid-cols-2 gap-6">
              <div>
                <div className="text-[11px] uppercase tracking-[0.22em] text-[var(--on-surface-variant)]">Connection Time</div>
                <div className="mt-1 font-mono text-[22px] font-semibold text-[var(--on-surface)]">{health ? "Live" : "--:--:--"}</div>
              </div>
              <div>
                <div className="text-[11px] uppercase tracking-[0.22em] text-[var(--on-surface-variant)]">Encryption Protocol</div>
                <div className="mt-1 flex items-center gap-2 text-[22px] font-semibold text-[var(--on-surface)]"><AppIcon name="lock" className="h-5 w-5 text-[var(--primary)]" />WireGuard AES-256</div>
              </div>
            </div>
          </div>

          <div className="grid gap-3">
            <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-5 py-4">
              <div className="flex items-start justify-between">
                <div>
                  <div className="text-sm text-[var(--on-surface-variant)]">Public IP Address</div>
                  <div className="mt-3 text-[18px] font-semibold tracking-tight text-[var(--on-surface)]">{health?.external_ipv4 ?? activeNode?.public_ipv4 ?? "n/a"}</div>
                </div>
                <AppIcon name="refresh" className="h-4 w-4 text-[var(--on-surface-variant)]" />
              </div>
              <div className="mt-8 flex items-center gap-2 text-[12px] text-[var(--secondary)]"><span className="h-2 w-2 rounded-full bg-[var(--secondary)]" />{tunnelConnected ? "Egress IP observed through tunnel" : "Connect to verify egress IP"}</div>
            </div>

            <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-5 py-4">
              <div className="flex items-start justify-between">
                <div>
                  <div className="text-sm text-[var(--on-surface-variant)]">Latest Handshake</div>
                  <div className="mt-3 text-[18px] font-semibold tracking-tight text-[var(--on-surface)]">{health?.tunnel_status ?? "Pending"}</div>
                  <div className="mt-1 text-xs text-[var(--on-surface-variant)]">Last handshake: {formatHandshakeAge(health?.handshake_age_seconds)}</div>
                </div>
                <AppIcon name="refresh" className="h-4 w-4 text-[var(--on-surface-variant)]" />
              </div>
              <div className="mt-5 h-1.5 overflow-hidden rounded-full bg-[color:rgba(255,255,255,0.07)]"><div className="h-full rounded-full bg-[var(--primary)]" style={{ width: `${signalStrength.toFixed(0)}%` }} /></div>
              <div className="mt-1 text-right text-[10px] text-[var(--on-surface-variant)]">Signal Strength: {tunnelConnected ? "Live" : "Pending"}</div>
            </div>
          </div>
        </div>

        <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-6 py-5">
          <div className="flex items-center justify-between gap-4">
            <h3 className="text-[18px] font-semibold tracking-tight text-[var(--on-surface)]">Network Throughput (60s)</h3>
            <div className="flex items-center gap-4 text-[11px] text-[var(--on-surface-variant)]">
              <span className="flex items-center gap-2"><span className="h-2.5 w-2.5 rounded-full bg-[var(--primary)]" />DL: {formatRate(latestThroughput?.downloadBytesPerSecond ?? 0)}</span>
              <span className="flex items-center gap-2"><span className="h-2.5 w-2.5 rounded-full bg-[var(--tertiary)]" />UL: {formatRate(latestThroughput?.uploadBytesPerSecond ?? 0)}</span>
            </div>
          </div>
          <div className="mt-5 h-[180px] rounded-[8px] border border-[color:rgba(255,255,255,0.03)] bg-[linear-gradient(180deg,rgba(11,14,21,0.96),rgba(11,14,21,0.96))] px-2 py-4">
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
          </div>
        </div>
      </PageChrome>
    );
  }

  return null;
}
