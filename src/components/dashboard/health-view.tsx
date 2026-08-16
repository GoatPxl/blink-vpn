"use client";

import { AppIcon, Button, Card, ConnectionTimer, MetricCard, PageChrome, formatBytes, formatHandshakeAge, useNow, type DashboardBusyAction, type HealthPanelData, type VpnNodeRecord } from "./shared";
import { formatDropAge, type CheckStatus, type ConnectivityCheck, type LogLine, type LogTab } from "@/lib/health-display";
import type { WireGuardReadiness } from "@/lib/tauri";

const STATUS_STYLES: Record<CheckStatus, { border: string; text: string; icon: string }> = {
  pass: { border: "border-[color:rgba(78,222,163,0.3)] bg-[color:rgba(78,222,163,0.07)]", text: "text-[var(--secondary)]", icon: "check" },
  fail: { border: "border-[color:rgba(255,155,155,0.35)] bg-[color:rgba(255,155,155,0.07)]", text: "text-[#ff9b9b]", icon: "lock" },
  warn: { border: "border-[color:rgba(255,185,95,0.3)] bg-[color:rgba(255,185,95,0.07)]", text: "text-[var(--tertiary)]", icon: "lock" },
  unknown: { border: "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]", text: "text-[var(--on-surface-variant)]", icon: "lock" },
};

const LOG_TABS: Array<{ id: LogTab; label: string }> = [
  { id: "all", label: "All Logs" },
  { id: "errors", label: "Errors" },
  { id: "wireguard", label: "WireGuard" },
];

const LOG_TONE_CLASS: Record<LogLine["tone"], string> = {
  error: "text-[#ff9b9b]",
  warning: "text-[var(--tertiary)]",
  success: "text-[var(--secondary)]",
  info: "text-[var(--primary)]",
  neutral: "text-[var(--on-surface-variant)]",
};

/**
 * How long ago the tunnel dropped, kept current.
 *
 * Mounted only when there is a drop, so its clock starts fresh and nothing ticks otherwise.
 */
function DropAge({ observedAtEpochMs }: { observedAtEpochMs: number }) {
  const now = useNow();
  return <>{formatDropAge(observedAtEpochMs, now)}</>;
}

export function HealthView({
  health,
  killSwitch,
  checks,
  logLines,
  logTab,
  onSelectLogTab,
  readiness,
  tunnelConnected,
  lastCheckedAt,
  selectedNode,
  selectedTunnelId,
  busyAction,
  onRunChecks,
  onReconnect,
}: {
  health: HealthPanelData | null;
  killSwitch: { headline: string; detail: string; status: CheckStatus };
  checks: ConnectivityCheck[];
  logLines: LogLine[];
  logTab: LogTab;
  onSelectLogTab: (tab: LogTab) => void;
  readiness: WireGuardReadiness | null;
  tunnelConnected: boolean;
  lastCheckedAt: number | null;
  selectedNode: VpnNodeRecord | null;
  selectedTunnelId: string;
  busyAction: DashboardBusyAction;
  onRunChecks: () => Promise<void>;
  onReconnect: (() => Promise<void>) | null;
}) {
  const drop = health?.unexpected_drop ?? null;
  const killSwitchStyle = STATUS_STYLES[killSwitch.status];
  const failing = checks.filter((check) => check.status === "fail").length;

  return (
    <PageChrome>
      {/* A tunnel that stopped on its own took WireGuard's filters with it, so traffic is flowing
          unprotected right now. This is the one thing on the page that must not be missable. */}
      {drop ? (
        <div className="rounded-[20px] border border-[color:rgba(255,155,155,0.4)] bg-[color:rgba(255,155,155,0.09)] px-5 py-4">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div>
              <div className="flex items-center gap-2 text-[#ff9b9b]">
                <AppIcon name="shield" className="h-4 w-4" />
                <span className="text-[11px] font-semibold uppercase tracking-[0.24em]">Tunnel dropped</span>
              </div>
              <p className="mt-2 max-w-2xl text-sm text-[var(--on-surface)]">
                The tunnel stopped without being asked to, <DropAge observedAtEpochMs={drop.observed_at_epoch_ms} />.
                Leak protection stopped with it, so traffic is using your normal connection until you reconnect.
              </p>
            </div>
            {onReconnect ? (
              <Button tone="primary" onClick={() => void onReconnect()} disabled={busyAction !== null}>
                {busyAction === "connect-tunnel" ? "Reconnecting" : "Reconnect"}
              </Button>
            ) : null}
          </div>
        </div>
      ) : null}

      {/* The headline claim of the page, derived from the routes the running config carries. */}
      <div className={`rounded-[24px] border px-6 py-5 ${killSwitchStyle.border}`}>
        <div className="flex items-start gap-4">
          <div className={`mt-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-[color:var(--outline-variant)] ${killSwitchStyle.text}`}>
            <AppIcon name="shield" className="h-5 w-5" />
          </div>
          <div>
            <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Kill switch</div>
            <div className={`mt-1 text-[24px] font-semibold tracking-tight ${killSwitchStyle.text}`}>{killSwitch.headline}</div>
            <p className="mt-2 max-w-3xl text-sm leading-6 text-[var(--on-surface-variant)]">{killSwitch.detail}</p>
            {health?.kill_switch.routes.length ? (
              <p className="mt-2 font-mono text-[11px] text-[var(--on-surface-variant)]">
                Routes: {health.kill_switch.routes.join(", ")}
              </p>
            ) : null}
          </div>
        </div>
      </div>

      <div className="grid gap-4 xl:grid-cols-[390px_minmax(0,1fr)]">
        <Card
          title="System Health"
          description={lastCheckedAt ? `Last check: ${new Date(lastCheckedAt).toLocaleTimeString()}` : "Waiting for the first health poll."}
          action={
            <div className={`rounded-full border px-3 py-1 text-[11px] uppercase tracking-[0.24em] ${tunnelConnected ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.08)] text-[var(--secondary)]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)]"}`}>
              {tunnelConnected ? "Connected" : "Offline"}
            </div>
          }
        >
          <div className="grid grid-cols-2 gap-3">
            <div className="rounded-[8px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3">
              <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Session</div>
              <div className="mt-1 font-mono text-[18px] text-[var(--on-surface)]">
                {tunnelConnected && health?.connected_at
                  ? <ConnectionTimer startedAt={health.connected_at} />
                  : "--:--:--"}
              </div>
              {tunnelConnected && !health?.connected_at ? (
                <div className="mt-1 text-[11px] text-[var(--on-surface-variant)]">Started before this session</div>
              ) : null}
            </div>
            <div className="rounded-[8px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3">
              <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Versions</div>
              <div className="mt-1 font-mono text-[13px] leading-5 text-[var(--on-surface)]">
                <div title={readiness?.version ?? undefined} className="truncate">{readiness?.version ?? "WireGuard version unknown"}</div>
                <div className="text-[var(--on-surface-variant)]">App {readiness?.app_version ?? "n/a"}</div>
              </div>
            </div>
          </div>
        </Card>

        <Card
          title="Live Logs"
          description="Real WireGuard commands and provisioning runs, redacted."
          action={
            <div className="flex items-center gap-2">
              {LOG_TABS.map((tab) => (
                <button
                  key={tab.id}
                  type="button"
                  onClick={() => onSelectLogTab(tab.id)}
                  aria-pressed={logTab === tab.id}
                  className={`rounded-[10px] border px-3 py-1.5 text-xs transition-colors ${logTab === tab.id
                    ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]"
                    : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)] hover:text-[var(--on-surface)]"}`}
                >
                  {tab.label}
                </button>
              ))}
            </div>
          }
        >
          <div className="max-h-[320px] overflow-auto rounded-[8px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(8,10,16,0.96),rgba(8,10,16,0.96))] p-4 font-mono text-[12px] leading-6">
            {logLines.length === 0 ? (
              <div className="text-[var(--on-surface-variant)]">
                {logTab === "errors" ? "No failures recorded." : "No activity captured yet."}
              </div>
            ) : (
              logLines.map((line) => (
                <div key={line.id} className="flex gap-2">
                  <span className="w-[68px] shrink-0 text-[var(--on-surface-variant)]">{line.timestamp}</span>
                  <span className={`w-[56px] shrink-0 ${LOG_TONE_CLASS[line.tone]}`}>[{line.level}]</span>
                  <span className="w-[150px] shrink-0 truncate text-[var(--on-surface-variant)]" title={line.source}>{line.source}</span>
                  <span className="flex-1 break-all text-[var(--on-surface)]">{line.message}</span>
                </div>
              ))
            )}
          </div>
        </Card>
      </div>

      <div className="grid gap-4 xl:grid-cols-[390px_minmax(0,1fr)]">
        <Card
          title="Connectivity Check"
          description={failing > 0 ? `${failing} check${failing === 1 ? "" : "s"} failing.` : "All checks passing."}
          action={<Button tone="secondary" onClick={() => void onRunChecks()} disabled={busyAction !== null}>{busyAction === "refresh-health" ? "Checking" : "Run Again"}</Button>}
        >
          <div className="space-y-3">
            {checks.map((check) => {
              const style = STATUS_STYLES[check.status];
              return (
                <div key={check.id} className={`rounded-[10px] border px-4 py-3 ${style.border}`}>
                  <div className="flex items-start gap-3">
                    <div className={`mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-[color:var(--outline-variant)] ${style.text}`}>
                      <AppIcon name={style.icon} className="h-3.5 w-3.5" />
                    </div>
                    <div>
                      <div className="font-medium text-[var(--on-surface)]">{check.title}</div>
                      <div className="text-sm text-[var(--on-surface-variant)]">{check.detail}</div>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </Card>

        <Card title="Connection Stats" description="Live handshake, addressing, and throughput for the selected tunnel.">
          {health ? (
            <div className="grid gap-4 md:grid-cols-2">
              <MetricCard title="Endpoint" value={health.endpoint_ip ?? "n/a"} detail={`Node ${health.node_id}`} tone="primary" />
              <MetricCard title="Egress IP" value={health.external_ipv4 ?? "Not verified"} detail={health.external_ipv4 ? "Observed through the tunnel" : "Connect to verify"} tone="secondary" />
              <MetricCard title="Handshake" value={formatHandshakeAge(health.handshake_age_seconds, health.tunnel_status)} detail={`Tunnel status ${health.tunnel_status ?? "n/a"}`} tone="tertiary" />
              <MetricCard title="Traffic" value={`RX ${formatBytes(health.rx_bytes)} / TX ${formatBytes(health.tx_bytes)}`} detail={`Selected tunnel ${selectedTunnelId || "none"}`} tone="primary" />
            </div>
          ) : (
            <div className="rounded-[10px] border border-dashed border-[color:var(--outline-variant)] px-4 py-6 text-sm text-[var(--on-surface-variant)]">
              {selectedNode ? "Waiting for the first health poll." : "Select a node to load health details."}
            </div>
          )}
        </Card>
      </div>
    </PageChrome>
  );
}
