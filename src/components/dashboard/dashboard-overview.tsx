"use client";

import { DashboardEmptyState } from "@/components/dashboard-empty-state";
import { AppIcon, Button, Card, MetricCard, PageChrome, type AppView, type HealthPanelData, type RecoveryState, type Toast, type DashboardBusyAction, type ProviderAccessState, type TunnelRecord, type VpnNodeRecord } from "./shared";

export function DashboardOverview({
  activeView,
  noServerConfigured,
  activeNode,
  connectedTunnel,
  health,
  canConnect,
  recovery,
  busyAction,
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
  recovery: RecoveryState | null;
  busyAction: DashboardBusyAction;
  onConnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onDisconnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onRefresh: () => Promise<void>;
}) {
  if (noServerConfigured && activeView === "dashboard") {
    return <DashboardEmptyState />;
  }

  if (!(noServerConfigured && activeView === "dashboard") && activeView === "dashboard") {
    return (
      <PageChrome>
        <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_340px]">
          <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-6 py-8">
            <div className="flex items-start justify-between gap-4">
              <div>
                <div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-[0.26em] text-[var(--secondary)]">
                  <span className="h-2.5 w-2.5 rounded-full bg-[var(--secondary)] shadow-[0_0_0_6px_rgba(78,222,163,0.12)]" />
                  <span>Status: Secure</span>
                </div>
                <h2 className="mt-3 text-[56px] font-semibold leading-none tracking-tight text-[var(--on-surface)]">{activeNode?.name ?? "No active node"}</h2>
                <p className="mt-2 text-sm text-[var(--on-surface-variant)]">Server: {activeNode ? `${activeNode.external_ref ?? activeNode.id}` : "n/a"}</p>
              </div>
              <Button tone={canConnect ? "primary" : "danger"} onClick={() => (canConnect && connectedTunnel ? void onConnectTunnel(connectedTunnel) : connectedTunnel ? void onDisconnectTunnel(connectedTunnel) : void onRefresh())} disabled={busyAction !== null || !connectedTunnel}>
                <AppIcon name="power" className="h-4 w-4" />
                {canConnect ? (busyAction === "connect-tunnel" ? "Connecting" : "Connect") : connectedTunnel ? (busyAction === "disconnect-tunnel" ? "Disconnecting" : "Disconnect") : "Refresh"}
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
              <div className="mt-8 flex items-center gap-2 text-[12px] text-[var(--secondary)]"><span className="h-2 w-2 rounded-full bg-[var(--secondary)]" />IP successfully masked</div>
            </div>

            <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-5 py-4">
              <div className="flex items-start justify-between">
                <div>
                  <div className="text-sm text-[var(--on-surface-variant)]">Latest Handshake</div>
                  <div className="mt-3 text-[18px] font-semibold tracking-tight text-[var(--on-surface)]">{health?.tunnel_status ?? "Pending"}</div>
                  <div className="mt-1 text-xs text-[var(--on-surface-variant)]">Latency: {health?.handshake_age_seconds ?? 14}ms</div>
                </div>
                <AppIcon name="refresh" className="h-4 w-4 text-[var(--on-surface-variant)]" />
              </div>
              <div className="mt-5 h-1.5 overflow-hidden rounded-full bg-[color:rgba(255,255,255,0.07)]"><div className="h-full w-[78%] rounded-full bg-[var(--primary)]" /></div>
              <div className="mt-1 text-right text-[10px] text-[var(--on-surface-variant)]">Signal Strength: Excellent</div>
            </div>
          </div>
        </div>

        <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-6 py-5">
          <div className="flex items-center justify-between gap-4">
            <h3 className="text-[18px] font-semibold tracking-tight text-[var(--on-surface)]">Network Throughput (60s)</h3>
            <div className="flex items-center gap-4 text-[11px] text-[var(--on-surface-variant)]">
              <span className="flex items-center gap-2"><span className="h-2.5 w-2.5 rounded-full bg-[var(--primary)]" />DL: 45.2 MB/s</span>
              <span className="flex items-center gap-2"><span className="h-2.5 w-2.5 rounded-full bg-[var(--tertiary)]" />UL: 12.8 MB/s</span>
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
              <path d="M0,108 L100,96 L200,116 L300,74 L400,92 L500,58 L600,82 L700,46 L800,68 L900,52 L1000,48" fill="none" stroke="rgba(171,199,255,0.95)" strokeWidth="3" />
              <path d="M0,136 L100,142 L200,128 L300,152 L400,120 L500,134 L600,112 L700,126 L800,100 L900,114 L1000,110" fill="none" stroke="rgba(255,185,95,0.9)" strokeWidth="2.5" />
            </svg>
          </div>
        </div>
      </PageChrome>
    );
  }

  return null;
}
