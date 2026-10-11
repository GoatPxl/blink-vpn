"use client";

import { DashboardOverview } from "@/components/dashboard/dashboard-overview";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { MobileDevicesCard } from "@/components/dashboard/mobile-devices-card";
import { useConnectivityChecks } from "@/components/dashboard/use-connectivity-checks";
import { DashboardEmptyState } from "@/components/dashboard-empty-state";

export default function DashboardPage() {
  const {
    activeNode,
    busyAction,
    connectAttempt,
    connectedTunnel,
    connectPrimary,
    connectTunnel,
    copyText,
    destroyNode,
    disconnectTunnel,
    exportDeviceConfig,
    forceDisconnectNodeService,
    health,
    loadDeviceConfig,
    nodes,
    providers,
    savedLocations,
    tunnelConnected,
    turnOnLocation,
  } = useDashboard();
  const checks = useConnectivityChecks();
  // With several servers and no default chosen there is no active node, but something is still
  // running and billing — "Off · nothing billing" would be false. Show the first one instead.
  const homeNode = activeNode ?? nodes[0] ?? null;

  if (!homeNode) {
    const hasVerifiedKey = providers.some(
      (provider) => provider.status === "active" && !!provider.token_ref && provider.last_validation_status !== "failed",
    );
    return (
      <DashboardEmptyState
        savedLocations={savedLocations}
        favourites={[]}
        busyAction={busyAction}
        hasVerifiedKey={hasVerifiedKey}
        onTurnOn={turnOnLocation}
      />
    );
  }

  return (
    <DashboardOverview
      activeNode={homeNode}
      connectedTunnel={connectedTunnel}
      health={health}
      tunnelConnected={tunnelConnected}
      busyAction={busyAction}
      connectAttempt={connectAttempt}
      checks={checks}
      onConnectPrimary={connectPrimary}
      onConnectTunnel={connectTunnel}
      onDisconnectTunnel={disconnectTunnel}
      onForceDisconnectNodeService={forceDisconnectNodeService}
      onPowerOff={destroyNode}
      devices={
        <MobileDevicesCard
          node={homeNode}
          busyAction={busyAction}
          onLoadDeviceConfig={loadDeviceConfig}
          onExportDeviceConfig={exportDeviceConfig}
          onCopyText={copyText}
        />
      }
    />
  );
}
