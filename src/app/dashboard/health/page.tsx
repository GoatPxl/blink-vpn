"use client";

import { HealthView } from "@/components/dashboard/health-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import {
  buildConnectivityChecks,
  buildLogLines,
  describeKillSwitch,
  type LogTab,
  type ProviderAccess,
} from "@/lib/health-display";
import { useCallback, useEffect, useMemo, useState } from "react";

export default function DashboardHealthPage() {
  const {
    busyAction,
    connectedTunnel,
    connectTunnel,
    diagnostics,
    health,
    healthUpdatedAt,
    providerAccessState,
    refreshDiagnostics,
    refreshHealth,
    refreshWireGuardReadiness,
    selectedNode,
    selectedTunnelId,
    tunnelConnected,
    wireguardReadiness,
  } = useDashboard();

  const [logTab, setLogTab] = useState<LogTab>("all");

  // The diagnostics bundle is otherwise fetched only by loadDashboard, so the log panel would show
  // whatever was captured on mount and never move. Deliberately the silent refresher: a full
  // dashboard reload sets `busyAction`, which would disable Reconnect every time it fired.
  useEffect(() => {
    const intervalId = window.setInterval(() => {
      void refreshDiagnostics();
    }, 15_000);
    return () => window.clearInterval(intervalId);
  }, [refreshDiagnostics]);

  const killSwitch = useMemo(() => describeKillSwitch(health?.kill_switch), [health?.kill_switch]);

  const checks = useMemo(
    () => buildConnectivityChecks({
      readiness: wireguardReadiness,
      providerAccess: providerAccessState as ProviderAccess,
      health,
      tunnelConnected,
    }),
    [health, providerAccessState, tunnelConnected, wireguardReadiness],
  );

  const logLines = useMemo(() => buildLogLines(diagnostics, logTab), [diagnostics, logTab]);

  const handleRunChecks = useCallback(async () => {
    await Promise.all([refreshWireGuardReadiness(), refreshDiagnostics(), refreshHealth()]);
  }, [refreshDiagnostics, refreshHealth, refreshWireGuardReadiness]);

  const handleReconnect = useCallback(async () => {
    if (connectedTunnel) {
      await connectTunnel(connectedTunnel);
    }
  }, [connectTunnel, connectedTunnel]);

  return (
    <HealthView
      health={health}
      killSwitch={killSwitch}
      checks={checks}
      logLines={logLines}
      logTab={logTab}
      onSelectLogTab={setLogTab}
      readiness={wireguardReadiness}
      tunnelConnected={tunnelConnected}
      lastCheckedAt={healthUpdatedAt}
      selectedNode={selectedNode}
      selectedTunnelId={selectedTunnelId}
      busyAction={busyAction}
      onRunChecks={handleRunChecks}
      onReconnect={connectedTunnel ? handleReconnect : null}
    />
  );
}
