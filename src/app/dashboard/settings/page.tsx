"use client";

import { SettingsView } from "@/components/dashboard/settings-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import type { ProviderAccess } from "@/lib/settings-display";
import type { ProviderRecord } from "@/lib/tauri";
import { useCallback } from "react";

export default function DashboardSettingsPage() {
  const {
    busyAction,
    connectedTunnel,
    connectTunnel,
    deleteProvider,
    dockerDevEnabled,
    nodes,
    providerAccessState,
    providerId,
    providers,
    resumeProvider,
    saveToken,
    saveTunnelSettings,
    selectProviderId,
    suspendProvider,
    tunnelConnected,
    tunnelSettings,
    validateToken,
    wireguardReadiness,
  } = useDashboard();

  // The key is never held in this component: it goes straight from the modal to the backend, so
  // there is no plaintext copy sitting in page state.
  const handleSaveKey = useCallback(async (targetProviderId: string, key: string) => {
    return saveToken(key, targetProviderId);
  }, [saveToken]);

  const handleRevalidate = useCallback(async (provider: ProviderRecord) => {
    // Validation reads the stored credential for the selected provider, so point it at this row
    // first — otherwise it would re-check whichever provider happened to be selected.
    selectProviderId(provider.id);
    await validateToken();
  }, [selectProviderId, validateToken]);

  const handleReconnect = useCallback(async () => {
    if (connectedTunnel) {
      await connectTunnel(connectedTunnel);
    }
  }, [connectTunnel, connectedTunnel]);

  return (
    <SettingsView
      dockerDevEnabled={dockerDevEnabled}
      providerId={providerId}
      providers={providers}
      nodes={nodes}
      providerAccessState={providerAccessState as ProviderAccess}
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
    />
  );
}
