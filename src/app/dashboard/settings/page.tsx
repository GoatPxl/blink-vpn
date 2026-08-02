"use client";

import { SettingsView } from "@/components/dashboard/settings-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { providerAccessBadge, VIEW_ROUTE } from "@/components/dashboard/shared";
import { useRouter } from "next/navigation";
import { useState } from "react";

export default function DashboardSettingsPage() {
  const router = useRouter();
  const [apiToken, setApiToken] = useState("");
  const {
    busyAction,
    configuredProvider,
    providerAccessState,
    providerFeedback,
    providerHasStoredToken,
    providerId,
    providerName,
    providerStatusMessage,
    saveToken,
    validateToken,
  } = useDashboard();

  const providerBadge = providerAccessBadge(providerAccessState);

  async function handleSaveToken() {
    await saveToken(apiToken);
    setApiToken("");
  }

  return (
    <SettingsView
      providerAccessState={providerAccessState}
      providerStatusMessage={providerStatusMessage}
      providerFeedback={providerFeedback}
      providerHasStoredToken={providerHasStoredToken}
      providerBadge={providerBadge}
      configuredProvider={configuredProvider}
      providerId={providerId}
      providerName={providerName}
      apiToken={apiToken}
      setApiToken={setApiToken}
      onSaveToken={handleSaveToken}
      onValidateToken={validateToken}
      busyAction={busyAction}
      onNavigateToView={(view) => router.push(VIEW_ROUTE[view])}
    />
  );
}
