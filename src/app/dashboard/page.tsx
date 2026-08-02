"use client";

import { DashboardOverview } from "@/components/dashboard/dashboard-overview";
import { useDashboard } from "@/components/dashboard/dashboard-provider";

export default function DashboardPage() {
  const {
    activeNode,
    busyAction,
    canConnect,
    connectedTunnel,
    connectPrimary,
    health,
    loadDashboard,
    noServerConfigured,
    recovery,
    connectTunnel,
    disconnectTunnel,
    throughputSamples,
  } = useDashboard();

  return (
    <DashboardOverview
      activeView="dashboard"
      noServerConfigured={noServerConfigured}
      activeNode={activeNode}
      connectedTunnel={connectedTunnel}
      health={health}
      canConnect={canConnect}
      throughputSamples={throughputSamples}
      recovery={recovery}
      busyAction={busyAction}
      onConnectPrimary={connectPrimary}
      onConnectTunnel={connectTunnel}
      onDisconnectTunnel={disconnectTunnel}
      onRefresh={loadDashboard}
    />
  );
}
