"use client";

import { useEffect, useSyncExternalStore } from "react";

import { DashboardOverview } from "@/components/dashboard/dashboard-overview";
import { WaitScreen } from "@/components/dashboard/wait-screen";
import { closeSession, getSession, keepWaitScreen, subscribeSession } from "@/lib/game/session";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { MobileDevicesCard } from "@/components/dashboard/mobile-devices-card";
import { useConnectivityChecks } from "@/components/dashboard/use-connectivity-checks";
import { DashboardEmptyState } from "@/components/dashboard-empty-state";
import { hasUsableKey } from "@/lib/settings-display";

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

  // While a server builds or a tunnel comes up — and for as long after as the game goes on — that
  // is the only thing Home is about.
  const session = useSyncExternalStore(subscribeSession, getSession, getSession);
  const waiting = operationAttempt?.kind === "provision" || connectAttempt !== null;
  const showWaitScreen = keepWaitScreen({ waiting, sessionOpen: session.open, gameOver: session.over });

  // Handing Home back ends the session, so the next wait starts a fresh game.
  useEffect(() => {
    if (!showWaitScreen && session.open) closeSession();
  }, [session.open, showWaitScreen]);

  if (showWaitScreen) {
    const waitNode = nodes.find((node) => node.id === (connectAttempt?.nodeId ?? homeNode?.id));
    const place = operationAttempt?.kind === "provision"
      ? operationAttempt.place
      : waitNode?.region_geo?.city ?? waitNode?.name ?? connectAttempt?.tunnelName ?? "";
    return <WaitScreen place={place} />;
  }

  if (!homeNode) {
    const hasVerifiedKey = providers.some(hasUsableKey);
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
