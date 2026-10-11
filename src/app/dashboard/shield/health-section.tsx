"use client";

import { HealthView } from "@/components/dashboard/health-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { useConnectivityChecks } from "@/components/dashboard/use-connectivity-checks";
import { describeKillSwitch, describeTunnelConfig } from "@/lib/health-display";
import { useCallback, useEffect, useMemo } from "react";

export function HealthSection() {
  const {
    busyAction,
    connectAttempt,
    connectedTunnel,
    connectTunnel,
    copyText,
    diagnostics,
    health,
    healthUpdatedAt,
    refreshDiagnostics,
    refreshHealth,
    refreshWireGuardReadiness,
    selectedNode,
    selectedTunnelId,
    tunnelConnected,
    wireguardReadiness,
  } = useDashboard();

  // The diagnostics bundle is otherwise fetched only by loadDashboard, so the config card would
  // show whatever was captured on mount and never move. Deliberately the silent refresher: a full
  // dashboard reload sets `busyAction`, which would disable Reconnect every time it fired.
  useEffect(() => {
    const intervalId = window.setInterval(() => {
      void refreshDiagnostics();
    }, 15_000);
    return () => window.clearInterval(intervalId);
  }, [refreshDiagnostics]);

  const killSwitch = useMemo(() => describeKillSwitch(health?.kill_switch), [health?.kill_switch]);

  const checks = useConnectivityChecks();

  const tunnelConfig = useMemo(() => describeTunnelConfig(diagnostics), [diagnostics]);

  const handleRunChecks = useCallback(async () => {
    await Promise.all([refreshWireGuardReadiness(), refreshDiagnostics(), refreshHealth()]);
  }, [refreshDiagnostics, refreshHealth, refreshWireGuardReadiness]);

  const handleReconnect = useCallback(async () => {
    if (connectedTunnel) {
      await connectTunnel(connectedTunnel);
    }
  }, [connectTunnel, connectedTunnel]);

  const handleCopyConfig = useCallback(async () => {
    if (tunnelConfig.configText) {
      await copyText(tunnelConfig.configText, "Tunnel config");
    }
  }, [copyText, tunnelConfig.configText]);

  return (
    <HealthView
      health={health}
      killSwitch={killSwitch}
      checks={checks}
      tunnelConfig={tunnelConfig}
      onCopyConfig={tunnelConfig.configText ? handleCopyConfig : null}
      readiness={wireguardReadiness}
      tunnelConnected={tunnelConnected}
      lastCheckedAt={healthUpdatedAt}
      selectedNode={selectedNode}
      selectedTunnelId={selectedTunnelId}
      busyAction={busyAction}
      reconnecting={connectAttempt !== null}
      onRunChecks={handleRunChecks}
      onReconnect={connectedTunnel ? handleReconnect : null}
    />
  );
}
