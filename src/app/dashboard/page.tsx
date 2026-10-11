"use client";

import { BuildPanel } from "@/components/dashboard/build-panel";
import { DashboardOverview } from "@/components/dashboard/dashboard-overview";
import { HoldOnPanel } from "@/components/dashboard/hold-on-panel";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { MobileDevicesCard } from "@/components/dashboard/mobile-devices-card";
import { useConnectivityChecks } from "@/components/dashboard/use-connectivity-checks";
import { DashboardEmptyState } from "@/components/dashboard-empty-state";

export default function DashboardPage() {
  const {
    activeNode,
    appPreferences,
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
    operationAttempt,
    providers,
    savedLocations,
    tunnelConnected,
    turnOnLocation,
  } = useDashboard();
  const checks = useConnectivityChecks();
  // With several servers and no default chosen there is no active node, but something is still
  // running and billing — "Off · nothing billing" would be false. Show the first one instead.
  const homeNode = activeNode ?? nodes[0] ?? null;

  // While a server builds or a tunnel comes up, that is the only thing Home is about.
  if (operationAttempt?.kind === "provision") {
    return <BuildPanel attempt={operationAttempt} />;
  }
  if (connectAttempt) {
    const connecting = nodes.find((node) => node.id === connectAttempt.nodeId);
    return <HoldOnPanel attempt={connectAttempt} place={connecting?.region_geo?.city ?? connecting?.name ?? connectAttempt.tunnelName} />;
  }

  if (!homeNode) {
    const hasVerifiedKey = providers.some(
      (provider) => provider.status === "active" && !!provider.token_ref && provider.last_validation_status !== "failed",
    );
    return (
      <DashboardEmptyState
        savedLocations={savedLocations}
        favourites={appPreferences?.favourites ?? []}
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
