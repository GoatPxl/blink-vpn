"use client";

import { SettingsView } from "@/components/dashboard/settings-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import type { ProviderRecord } from "@/lib/tauri";
import { useCallback } from "react";

export default function DashboardSettingsPage() {
  const {
    busyAction,
    connectAttempt,
    connectedTunnel,
    connectTunnel,
    deleteProvider,
    dockerDevEnabled,
    nodes,
    providers,
    resumeProvider,
    saveToken,
    saveTunnelSettings,
    suspendProvider,
    tunnelConnected,
    tunnelSettings,
    validateProviderKey,
    validatingProviderId,
    wireguardReadiness,
  } = useDashboard();

  // The credential is never held in this component: it goes straight from the modal to the backend,
  // so there is no plaintext copy sitting in page state.
  const handleSaveKey = useCallback(async (targetProviderId: string, credentials: Record<string, string>) => {
    return saveToken(credentials, targetProviderId);
  }, [saveToken]);

  const handleRevalidate = useCallback(async (provider: ProviderRecord) => {
    // Checks this provider and records the result against it. Notably does *not* change the
    // app-wide selected provider — doing so made the Verified badge jump between rows.
    await validateProviderKey(provider.id);
  }, [validateProviderKey]);

  const handleReconnect = useCallback(async () => {
    if (connectedTunnel) {
      await connectTunnel(connectedTunnel);
    }
  }, [connectTunnel, connectedTunnel]);

  return (
    <SettingsView
      dockerDevEnabled={dockerDevEnabled}
      providers={providers}
      nodes={nodes}
      validatingProviderId={validatingProviderId}
      onSaveKey={handleSaveKey}
      onRevalidate={handleRevalidate}
      onSuspendProvider={suspendProvider}
      onResumeProvider={resumeProvider}
      onDeleteProvider={deleteProvider}
      tunnelSettings={tunnelSettings}
      onSaveTunnelSettings={saveTunnelSettings}
      tunnelConnected={tunnelConnected}
      onReconnect={connectedTunnel ? handleReconnect : null}
      readiness={wireguardReadiness}
      busyAction={busyAction}
      reconnecting={connectAttempt !== null}
    />
  );
}
