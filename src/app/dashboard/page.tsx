"use client";

import { DashboardOverview } from "@/components/dashboard/dashboard-overview";
import { useDashboard } from "@/components/dashboard/dashboard-provider";

export default function DashboardPage() {
  const {
    activeNode,
    busyAction,
    canConnect,
    connectedTunnel,
    health,
    loadDashboard,
    noServerConfigured,
    recovery,
    connectTunnel,
    disconnectTunnel,
  } = useDashboard();

  return (
    <DashboardOverview
      activeView="dashboard"
      noServerConfigured={noServerConfigured}
      activeNode={activeNode}
      connectedTunnel={connectedTunnel}
      health={health}
      canConnect={canConnect}
      recovery={recovery}
      busyAction={busyAction}
      onConnectTunnel={connectTunnel}
      onDisconnectTunnel={disconnectTunnel}
      onRefresh={loadDashboard}
    />
  );
}
