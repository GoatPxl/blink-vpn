"use client";

import { DashboardOverview } from "@/components/dashboard/dashboard-overview";
import { useDashboard } from "@/components/dashboard/dashboard-provider";

export default function DashboardPage() {
  const {
    activeNode,
    busyAction,
    connectedTunnel,
    connectPrimary,
    health,
    loadDashboard,
    noServerConfigured,
    recovery,
    refreshHealth,
    connectTunnel,
    disconnectTunnel,
    throughputSamples,
    tunnelConnected,
  } = useDashboard();

  return (
    <DashboardOverview
      activeView="dashboard"
      noServerConfigured={noServerConfigured}
      activeNode={activeNode}
      connectedTunnel={connectedTunnel}
      health={health}
      tunnelConnected={tunnelConnected}
      throughputSamples={throughputSamples}
      recovery={recovery}
      busyAction={busyAction}
      onConnectPrimary={connectPrimary}
      onConnectTunnel={connectTunnel}
      onDisconnectTunnel={disconnectTunnel}
      onRefresh={loadDashboard}
      onRefreshHealth={refreshHealth}
    />
  );
}
