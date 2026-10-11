"use client";

import { useCallback, useEffect, useMemo } from "react";

import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { ShieldView } from "@/components/dashboard/shield-view";
import { useConnectivityChecks } from "@/components/dashboard/use-connectivity-checks";
import { describeKillSwitch, describeTunnelConfig } from "@/lib/health-display";

/** Often enough to feel live, rare enough not to hammer anything. */
const DIAGNOSTICS_REFRESH_MS = 15_000;

export default function ShieldPage() {
  const dashboard = useDashboard();
  const {
    connectTunnel,
    connectedTunnel,
    copyText,
    diagnostics,
    forceCleanupNode,
    health,
    refreshDiagnostics,
    refreshHealth,
    refreshWireGuardReadiness,
  } = dashboard;
  const checks = useConnectivityChecks();

  // The diagnostics bundle is otherwise fetched only by loadDashboard, so the config and the log
  // would show whatever was captured on mount and never move. Deliberately the silent refresher: a
  // full reload sets `busyAction`, which would disable Reconnect every time it fired.
  useEffect(() => {
    const intervalId = window.setInterval(() => void refreshDiagnostics(), DIAGNOSTICS_REFRESH_MS);
    return () => window.clearInterval(intervalId);
  }, [refreshDiagnostics]);

  const killSwitch = useMemo(() => describeKillSwitch(health?.kill_switch), [health?.kill_switch]);
  const tunnelConfig = useMemo(() => describeTunnelConfig(diagnostics), [diagnostics]);

  const handleRunChecks = useCallback(async () => {
    await Promise.all([refreshWireGuardReadiness(), refreshDiagnostics(), refreshHealth()]);
  }, [refreshDiagnostics, refreshHealth, refreshWireGuardReadiness]);

  const handleReconnect = useCallback(async () => {
    if (connectedTunnel) await connectTunnel(connectedTunnel);
  }, [connectTunnel, connectedTunnel]);

  const handleCopyConfig = useCallback(async () => {
    if (tunnelConfig.configText) await copyText(tunnelConfig.configText, "Tunnel config");
  }, [copyText, tunnelConfig.configText]);

  return (
    <ShieldView
      health={health}
      killSwitch={killSwitch}
      checks={checks}
      tunnelConfig={tunnelConfig}
      tunnelConnected={dashboard.tunnelConnected}
      tunnelSettings={dashboard.tunnelSettings}
      throughputSamples={dashboard.throughputSamples}
      readiness={dashboard.wireguardReadiness}
      lastCheckedAt={dashboard.healthUpdatedAt}
      node={dashboard.selectedNode}
      nodePeers={dashboard.nodePeers}
      nodes={dashboard.nodes}
      diagnostics={diagnostics}
      diagnosticsExportPath={dashboard.diagnosticsExportPath}
      locations={dashboard.appLocations}
      busyAction={dashboard.busyAction}
      reconnecting={dashboard.connectAttempt !== null}
      onRunChecks={handleRunChecks}
      onReconnect={connectedTunnel ? handleReconnect : null}
      onCopyConfig={tunnelConfig.configText ? handleCopyConfig : null}
      onCopyDiagnostics={dashboard.copyDiagnostics}
      onExportDiagnostics={dashboard.exportDiagnostics}
      onRevealLocation={dashboard.revealLocation}
      onCopyText={copyText}
      onRecoverNode={forceCleanupNode}
      onClaimDevice={dashboard.claimDevice}
      onLoadDeviceConfig={dashboard.loadDeviceConfig}
      onExportDeviceConfig={dashboard.exportDeviceConfig}
    />
  );
}
