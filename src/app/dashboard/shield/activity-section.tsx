"use client";

import { useCallback, useEffect } from "react";

import { ActivityView } from "@/components/dashboard/activity-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";

/** Matches the Kill Switch page: often enough to feel live, rare enough not to hammer anything. */
const REFRESH_INTERVAL_MS = 15_000;

export function ActivitySection() {
  const {
    appLocations,
    busyAction,
    copyDiagnostics,
    copyText,
    diagnostics,
    diagnosticsExportPath,
    exportDiagnostics,
    forceCleanupNode,
    nodes,
    refreshDiagnostics,
    revealLocation,
  } = useDashboard();

  // The old page never re-fetched, so a connect made while it was open left no visible trace until
  // the user navigated away and back.
  useEffect(() => {
    const intervalId = window.setInterval(() => {
      void refreshDiagnostics();
    }, REFRESH_INTERVAL_MS);

    return () => window.clearInterval(intervalId);
  }, [refreshDiagnostics]);

  const handleRecoverNode = useCallback(async (nodeId: string) => {
    await forceCleanupNode(nodeId);
  }, [forceCleanupNode]);

  return (
    <ActivityView
      diagnostics={diagnostics}
      diagnosticsExportPath={diagnosticsExportPath}
      locations={appLocations}
      nodes={nodes}
      busyAction={busyAction}
      onCopyDiagnostics={copyDiagnostics}
      onExportDiagnostics={exportDiagnostics}
      onRevealLocation={revealLocation}
      onCopyText={copyText}
      onRecoverNode={handleRecoverNode}
    />
  );
}
