"use client";

import { AppIcon, Button, Card, PageChrome, type DashboardBusyAction, type DiagnosticsBundle, type RecoveryState } from "./shared";

export function DiagnosticsView({
  diagnostics,
  diagnosticsExportPath,
  recovery,
  busyAction,
  onCopyDiagnostics,
  onExportDiagnostics,
}: {
  diagnostics: DiagnosticsBundle | null;
  diagnosticsExportPath: string;
  recovery: RecoveryState | null;
  busyAction: DashboardBusyAction;
  onCopyDiagnostics: () => Promise<void>;
  onExportDiagnostics: () => Promise<void>;
}) {
  return (
    <PageChrome>
      <div className="mx-auto w-full max-w-6xl space-y-4">
        <Card title="Diagnostics Bundle" description="Review the latest redacted health and lifecycle bundle for this device.">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[color:var(--outline-variant)] pb-4">
            <div>
              <div className="text-sm text-[var(--on-surface-variant)]">Last export</div>
              <div className="mt-1 font-mono text-[13px] text-[var(--on-surface)]">{diagnosticsExportPath}</div>
            </div>
            <div className="flex items-center gap-2">
              <Button tone="secondary" onClick={() => void onCopyDiagnostics()} disabled={busyAction !== null || !diagnostics}>Copy</Button>
              <Button onClick={() => void onExportDiagnostics()} disabled={busyAction !== null}>Export</Button>
            </div>
          </div>
          <div className="mt-4 overflow-hidden rounded-[18px] border border-[color:var(--outline-variant)]">
            <pre className="max-h-[560px] overflow-auto bg-[color:rgba(8,10,16,0.96)] p-4 text-[12px] leading-6 text-[var(--on-surface)]">
              {diagnostics ? JSON.stringify(diagnostics, null, 2) : "Loading diagnostics bundle..."}
            </pre>
          </div>
          {recovery ? <div className="mt-4 rounded-[16px] border border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] p-4 text-sm text-[var(--tertiary)]">{recovery.title}: {recovery.message}</div> : null}
        </Card>
      </div>
    </PageChrome>
  );
}
