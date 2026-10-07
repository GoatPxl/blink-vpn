"use client";

import { DashboardOverview } from "@/components/dashboard/dashboard-overview";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { MobileDevicesCard } from "@/components/dashboard/mobile-devices-card";
import { DashboardEmptyState } from "@/components/dashboard-empty-state";

export default function DashboardPage() {
  const {
    activeNode,
    busyAction,
    connectAttempt,
    connectedTunnel,
    connectPrimary,
    copyText,
    exportDeviceConfig,
    health,
    loadDeviceConfig,
    savedLocations,
    turnOnLocation,
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
      connectAttempt={connectAttempt}
      onConnectPrimary={connectPrimary}
      onConnectTunnel={connectTunnel}
      onDisconnectTunnel={disconnectTunnel}
      onRefresh={loadDashboard}
      onRefreshHealth={refreshHealth}
      emptyState={
        <DashboardEmptyState
          savedLocations={savedLocations}
          busyAction={busyAction}
          onTurnOn={turnOnLocation}
        />
      }
      devices={
        activeNode ? (
          <MobileDevicesCard
            node={activeNode}
            busyAction={busyAction}
            onLoadDeviceConfig={loadDeviceConfig}
            onExportDeviceConfig={exportDeviceConfig}
            onCopyText={copyText}
          />
        ) : null
      }
    />
  );
}
