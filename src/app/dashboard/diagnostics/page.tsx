"use client";

import { DiagnosticsView } from "@/components/dashboard/diagnostics-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";

export default function DashboardDiagnosticsPage() {
  const { busyAction, copyDiagnostics, diagnostics, diagnosticsExportPath, exportDiagnostics, recovery } = useDashboard();

  return (
    <DiagnosticsView
      diagnostics={diagnostics}
      diagnosticsExportPath={diagnosticsExportPath}
      recovery={recovery}
      busyAction={busyAction}
      onCopyDiagnostics={copyDiagnostics}
      onExportDiagnostics={exportDiagnostics}
    />
  );
}
