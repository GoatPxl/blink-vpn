"use client";

import { PixelIcon, type PixelIconName } from "@/components/ui/pixel-icon";
import { Button, Panel, ConnectionTimer, MetricCard, PageChrome, formatBytes, formatHandshakeAge, useNow, type DashboardBusyAction, type HealthPanelData, type VpnNodeRecord } from "./shared";
import { formatDropAge, type CheckStatus, type ConnectivityCheck, type describeTunnelConfig } from "@/lib/health-display";
import type { WireGuardReadiness } from "@/lib/tauri";

type TunnelConfigDisplay = ReturnType<typeof describeTunnelConfig>;

const STATUS_STYLES: Record<CheckStatus, { border: string; text: string; icon: PixelIconName }> = {
  pass: { border: "border-[color:rgba(78,222,163,0.3)] bg-[color:rgba(78,222,163,0.07)]", text: "text-[var(--secondary)]", icon: "check" },
  fail: { border: "border-[color:rgba(255,155,155,0.35)] bg-[color:rgba(255,155,155,0.07)]", text: "text-[#ff9b9b]", icon: "alert" },
  warn: { border: "border-[color:rgba(255,185,95,0.3)] bg-[color:rgba(255,185,95,0.07)]", text: "text-[var(--tertiary)]", icon: "alert" },
  unknown: { border: "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]", text: "text-[var(--on-surface-variant)]", icon: "alert" },
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
  tunnelConfig,
  onCopyConfig,
  readiness,
  tunnelConnected,
  lastCheckedAt,
  selectedNode,
  selectedTunnelId,
  busyAction,
  reconnecting,
  onRunChecks,
  onReconnect,
}: {
  health: HealthPanelData | null;
  killSwitch: { headline: string; detail: string; status: CheckStatus };
  checks: ConnectivityCheck[];
  tunnelConfig: TunnelConfigDisplay;
  onCopyConfig: (() => Promise<void>) | null;
  readiness: WireGuardReadiness | null;
  tunnelConnected: boolean;
  lastCheckedAt: number | null;
  selectedNode: VpnNodeRecord | null;
  selectedTunnelId: string;
  busyAction: DashboardBusyAction;
  /** A connect started from this banner is still in flight. */
  reconnecting: boolean;
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
                <PixelIcon name="shield" className="h-4 w-4" />
                <span className="text-[11px] font-semibold uppercase tracking-[0.24em]">Tunnel dropped</span>
              </div>
              <p className="mt-2 max-w-2xl text-sm text-[var(--on-surface)]">
                The tunnel stopped without being asked to, <DropAge observedAtEpochMs={drop.observed_at_epoch_ms} />.
                Leak protection stopped with it, so traffic is using your normal connection until you reconnect.
              </p>
            </div>
            {onReconnect ? (
              <Button
                tone="primary"
                onClick={() => void onReconnect()}
                disabled={busyAction !== null || reconnecting}
              >
                {reconnecting ? "Reconnecting" : "Reconnect"}
              </Button>
            ) : null}
          </div>
        </div>
      ) : null}

      {/* The headline claim of the page, derived from the routes the running config carries. */}
      <div className={`rounded-[24px] border px-6 py-5 ${killSwitchStyle.border}`}>
        <div className="flex items-start gap-4">
          <div className={`mt-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-[color:var(--outline-variant)] ${killSwitchStyle.text}`}>
            <PixelIcon name="shield" className="h-5 w-5" />
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
        <Panel
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
        </Panel>

        {/* Sits beside the kill-switch verdict because AllowedIPs is what decides it: a config
            without a /0 route is exactly the case the banner above reports as unprotected. */}
        <Panel
          title="Tunnel Configuration"
          description="What the tunnel service was handed, in full."
          action={
            onCopyConfig ? (
              <Button tone="ghost" onClick={() => void onCopyConfig()}>Copy</Button>
            ) : null
          }
        >
          {tunnelConfig.available ? (
            <>
              <pre className="max-h-[320px] overflow-auto rounded-[8px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(8,10,16,0.96),rgba(8,10,16,0.96))] p-4 font-mono text-[12px] leading-6 text-[var(--on-surface)]">
                {tunnelConfig.configText}
              </pre>
              <div className="mt-3 flex flex-wrap gap-x-6 gap-y-1 text-[11px] text-[var(--on-surface-variant)]">
                {tunnelConfig.tunnelName ? <span>Tunnel: <span className="font-mono">{tunnelConfig.tunnelName}</span></span> : null}
                {tunnelConfig.sourcePath ? <span className="break-all">Source: <span className="font-mono">{tunnelConfig.sourcePath}</span></span> : null}
              </div>
            </>
          ) : (
            <div className="rounded-[10px] border border-dashed border-[color:var(--outline-variant)] px-4 py-6 text-sm text-[var(--on-surface-variant)]">
              {tunnelConfig.emptyReason}
            </div>
          )}
        </Panel>
      </div>

      <div className="grid gap-4 xl:grid-cols-[390px_minmax(0,1fr)]">
        <Panel
          title="Connectivity Check"
          description={failing > 0 ? `${failing} check${failing === 1 ? "" : "s"} failing.` : "All checks passing."}
          action={<Button tone="ghost" onClick={() => void onRunChecks()} disabled={busyAction !== null}>{busyAction === "refresh-health" ? "Checking" : "Run Again"}</Button>}
        >
          <div className="space-y-3">
            {checks.map((check) => {
              const style = STATUS_STYLES[check.status];
              return (
                <div key={check.id} className={`rounded-[10px] border px-4 py-3 ${style.border}`}>
                  <div className="flex items-start gap-3">
                    <div className={`mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-[color:var(--outline-variant)] ${style.text}`}>
                      <PixelIcon name={style.icon} className="h-3.5 w-3.5" />
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
        </Panel>

        <Panel title="Connection Stats" description="Live handshake, addressing, and throughput for the selected tunnel.">
          {health ? (
            <div className="grid gap-4 md:grid-cols-2">
              <MetricCard title="Endpoint" value={health.endpoint_ip ?? "n/a"} detail={`Node ${health.node_id}`} />
              <MetricCard title="Egress IP" value={health.external_ipv4 ?? "Not verified"} detail={health.external_ipv4 ? "Observed through the tunnel" : "Connect to verify"} />
              <MetricCard title="Handshake" value={formatHandshakeAge(health.handshake_age_seconds, health.tunnel_status)} detail={`Tunnel status ${health.tunnel_status ?? "n/a"}`} />
              <MetricCard title="Traffic" value={`RX ${formatBytes(health.rx_bytes)} / TX ${formatBytes(health.tx_bytes)}`} detail={`Selected tunnel ${selectedTunnelId || "none"}`} />
            </div>
          ) : (
            <div className="rounded-[10px] border border-dashed border-[color:var(--outline-variant)] px-4 py-6 text-sm text-[var(--on-surface-variant)]">
              {selectedNode ? "Waiting for the first health poll." : "Select a node to load health details."}
            </div>
          )}
        </Panel>
      </div>
    </PageChrome>
  );
}
