"use client";

import { SettingsView } from "@/components/dashboard/settings-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { VIEW_ROUTE } from "@/components/dashboard/shared";
import { useRouter } from "next/navigation";
import { useState } from "react";

export default function DashboardSettingsPage() {
  const router = useRouter();
  const [apiToken, setApiToken] = useState("");
  const [dockerFauxToken, setDockerFauxToken] = useState("");
  const {
    busyAction,
    configuredProvider,
    deleteProvider,
    dockerDevEnabled,
    dockerFauxTokenConfigured,
    connectedTunnel,
    nodes,
    providerId,
    providers,
    saveWireGuardConnectionMode,
    selectProviderId,
    saveDockerFauxToken,
    saveToken,
    suspendProvider,
    resumeProvider,
    validateDockerReadiness,
    validateToken,
    wireGuardConnectionMode,
  } = useDashboard();

  async function handleSaveToken() {
    await saveToken(apiToken);
    setApiToken("");
  }

  async function handleSaveDockerFauxToken() {
    await saveDockerFauxToken(dockerFauxToken);
    setDockerFauxToken("");
  }

  return (
    <SettingsView
      dockerDevEnabled={dockerDevEnabled}
      dockerFauxToken={dockerFauxToken}
      setDockerFauxToken={setDockerFauxToken}
      dockerFauxTokenConfigured={dockerFauxTokenConfigured}
      wireGuardConnectionMode={wireGuardConnectionMode}
      wireGuardModeLocked={connectedTunnel?.status.toLowerCase() === "connected"}
      onChangeWireGuardConnectionMode={saveWireGuardConnectionMode}
      configuredProvider={configuredProvider}
      providerId={providerId}
      providers={providers}
      nodes={nodes}
      onSelectProvider={selectProviderId}
      apiToken={apiToken}
      setApiToken={setApiToken}
      onSaveToken={handleSaveToken}
      onSaveDockerFauxToken={handleSaveDockerFauxToken}
      onValidateToken={validateToken}
      onValidateDockerReadiness={validateDockerReadiness}
      onSuspendProvider={suspendProvider}
      onResumeProvider={resumeProvider}
      onDeleteProvider={deleteProvider}
      busyAction={busyAction}
      onNavigateToView={(view) => router.push(VIEW_ROUTE[view])}
    />
  );
}
