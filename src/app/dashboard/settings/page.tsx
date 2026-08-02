"use client";

import { SettingsView } from "@/components/dashboard/settings-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { providerAccessBadge, VIEW_ROUTE } from "@/components/dashboard/shared";
import { useRouter } from "next/navigation";
import { useState } from "react";

export default function DashboardSettingsPage() {
  const router = useRouter();
  const [apiToken, setApiToken] = useState("");
  const [dockerFauxToken, setDockerFauxToken] = useState("");
  const {
    busyAction,
    configuredProvider,
    dockerDevEnabled,
    dockerFauxTokenConfigured,
    dockerFauxTokenConfiguredAt,
    dockerReadiness,
    dockerReadinessMessage,
    dockerReadinessState,
    providerAccessState,
    providerFeedback,
    providerHasStoredToken,
    providerId,
    providerName,
    providerStatusMessage,
    saveDockerFauxToken,
    saveToken,
    validateDockerReadiness,
    validateToken,
  } = useDashboard();

  const providerBadge = providerAccessBadge(providerAccessState);

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
      providerAccessState={providerAccessState}
      providerStatusMessage={providerStatusMessage}
      providerFeedback={providerFeedback}
      dockerDevEnabled={dockerDevEnabled}
      dockerReadinessState={dockerReadinessState}
      dockerReadinessMessage={dockerReadinessMessage}
      dockerReadiness={dockerReadiness}
      dockerFauxToken={dockerFauxToken}
      setDockerFauxToken={setDockerFauxToken}
      dockerFauxTokenConfigured={dockerFauxTokenConfigured}
      dockerFauxTokenConfiguredAt={dockerFauxTokenConfiguredAt}
      providerHasStoredToken={providerHasStoredToken}
      providerBadge={providerBadge}
      configuredProvider={configuredProvider}
      providerId={providerId}
      providerName={providerName}
      apiToken={apiToken}
      setApiToken={setApiToken}
      onSaveToken={handleSaveToken}
      onSaveDockerFauxToken={handleSaveDockerFauxToken}
      onValidateToken={validateToken}
      onValidateDockerReadiness={validateDockerReadiness}
      busyAction={busyAction}
      onNavigateToView={(view) => router.push(VIEW_ROUTE[view])}
    />
  );
}
