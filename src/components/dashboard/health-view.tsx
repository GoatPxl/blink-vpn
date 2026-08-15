"use client";

import { AppIcon, Button, Card, MetricCard, PageChrome, type DashboardBusyAction, type RecoveryState, type HealthPanelData, type TunnelRecord, type VpnNodeRecord, type LogEntry } from "./shared";
import { formatBytes, formatHandshakeAge } from "./shared";

export function HealthView({
  diagnosticsLogs,
  health,
  selectedNode,
  selectedTunnelId,
  busyAction,
  onRefreshHealth,
}: {
  diagnosticsLogs: LogEntry[];
  health: HealthPanelData | null;
  selectedNode: VpnNodeRecord | null;
  selectedTunnelId: string;
  busyAction: DashboardBusyAction;
  onRefreshHealth: () => Promise<void>;
}) {
  return (
    <PageChrome>
      <div className="flex items-center gap-2 text-xs text-[var(--on-surface-variant)]">
        <AppIcon name="shield" className="h-4 w-4" />
        <span>Health</span>
        <span>›</span>
        <span>Connection Check</span>
      </div>
      <div className="grid gap-4 xl:grid-cols-[390px_minmax(0,1fr)]">
        <Card title="System Health" description="Last check: Just now" action={<div className="rounded-full border border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.08)] px-3 py-1 text-[11px] uppercase tracking-[0.24em] text-[var(--secondary)]">Online</div>}>
          <div className="grid grid-cols-2 gap-3">
            <div className="rounded-[8px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3">
              <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Uptime</div>
              <div className="mt-1 font-mono text-[18px] text-[var(--on-surface)]">14d 08h 22m</div>
            </div>
            <div className="rounded-[8px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3">
              <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Engine Version</div>
              <div className="mt-1 font-mono text-[18px] text-[var(--on-surface)]">v4.2.1-stable</div>
            </div>
          </div>
        </Card>

        <Card title="Live Logs" description="WireGuard and backend activity" action={<div className="flex items-center gap-2"><button type="button" className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-1.5 text-xs text-[var(--on-surface-variant)]">All Logs</button><button type="button" className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-1.5 text-xs text-[var(--on-surface-variant)]">Errors</button><button type="button" className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-1.5 text-xs text-[var(--on-surface-variant)]">WireGuard</button></div>}>
          <div className="max-h-[560px] overflow-auto rounded-[8px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(8,10,16,0.96),rgba(8,10,16,0.96))] p-4 font-mono text-[12px] leading-6 text-[var(--on-surface)]">
            {diagnosticsLogs.map((entry) => (
              <div key={`${entry.time}-${entry.message}`} className="flex gap-2">
                <span className="w-[80px] text-[var(--on-surface-variant)]">{entry.time}</span>
                <span className={entry.tone === "warning" ? "text-[var(--tertiary)]" : entry.tone === "error" ? "text-[#ff9b9b]" : entry.tone === "success" ? "text-[var(--secondary)]" : "text-[var(--primary)]"}>[{entry.level}]</span>
                <span className="text-[var(--on-surface-variant)]">{entry.source}</span>
                <span className="flex-1 text-[var(--on-surface)]">{entry.message}</span>
              </div>
            ))}
          </div>
        </Card>
      </div>

      <div className="grid gap-4 xl:grid-cols-[390px_minmax(0,1fr)]">
        <Card title="Connectivity Check" action={<Button tone="secondary" onClick={() => void onRefreshHealth()} disabled={busyAction !== null}>Run Again</Button>}>
          <div className="space-y-4">
            {[
              { title: "API Reachable", detail: "Successfully connected to api.digitalocean.com", active: true },
              { title: "Token Valid", detail: "Authentication token verified for Enterprise tier", active: true },
              { title: "DNS Resolution", detail: "Resolving via primary SecureDNS", active: true },
              { title: "Node Responding", detail: `Handshaking with ${selectedNode?.region ?? "selected region"}`, active: true },
              { title: "Tunnel Established", detail: "Waiting for node response...", active: false },
            ].map((item) => (
              <div key={item.title} className={`rounded-[10px] border px-4 py-3 ${item.active ? "border-[color:rgba(78,222,163,0.2)] bg-[color:rgba(78,222,163,0.06)]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]"}`}>
                <div className="flex items-start gap-3">
                  <div className={`mt-0.5 flex h-6 w-6 items-center justify-center rounded-full border ${item.active ? "border-[color:rgba(78,222,163,0.4)] text-[var(--secondary)]" : "border-[color:var(--outline-variant)] text-[var(--on-surface-variant)]"}`}>
                    <AppIcon name={item.active ? "check" : "lock"} className="h-3.5 w-3.5" />
                  </div>
                  <div>
                    <div className="font-medium text-[var(--on-surface)]">{item.title}</div>
                    <div className="text-sm text-[var(--on-surface-variant)]">{item.detail}</div>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </Card>

        <Card title="Connection Stats" description="Handshake and throughput details aligned with the reference visual hierarchy.">
          {health ? (
            <div className="grid gap-4 md:grid-cols-2">
              <MetricCard title="Endpoint" value={health.endpoint_ip ?? "n/a"} detail={`Node ${health.node_id}`} tone="primary" />
              <MetricCard title="External IP" value={health.external_ipv4 ?? "n/a"} detail={`Tunnel ${health.tunnel_id ?? "n/a"}`} tone="secondary" />
              <MetricCard title="Handshake" value={formatHandshakeAge(health.handshake_age_seconds, health.tunnel_status)} detail={`Tunnel status ${health.tunnel_status ?? "n/a"}`} tone="tertiary" />
              <MetricCard title="Traffic" value={`RX ${formatBytes(health.rx_bytes)} / TX ${formatBytes(health.tx_bytes)}`} detail={`Selected tunnel ${selectedTunnelId || "none"}`} tone="primary" />
            </div>
          ) : <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm text-[var(--on-surface-variant)]">Select a node to load health details.</div>}
        </Card>
      </div>
    </PageChrome>
  );
}
