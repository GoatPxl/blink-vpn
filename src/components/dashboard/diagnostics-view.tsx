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
  const initRuns = diagnostics?.init_runs ?? [];
  const wireguardDebug = diagnostics?.wireguard_debug;

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
          <div className="mt-4 rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.7)] p-4">
            <div className="text-sm font-semibold text-[var(--on-surface)]">WireGuard Connect Trace</div>
            <div className="mt-1 text-xs text-[var(--on-surface-variant)]">
              Captures redacted tunnel config and recent WireGuard/SC commands used by connect/import flows.
            </div>
            {wireguardDebug ? (
              <div className="mt-3 space-y-2">
                <div className="rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-3 py-2">
                  <div className="text-[11px] uppercase tracking-[0.16em] text-[var(--on-surface-variant)]">Last Imported Tunnel</div>
                  <div className="mt-1 font-mono text-xs text-[var(--on-surface)]">{wireguardDebug.last_import_tunnel_name ?? "n/a"}</div>
                  <div className="mt-2 text-[11px] uppercase tracking-[0.16em] text-[var(--on-surface-variant)]">Temp Config Path</div>
                  <div className="mt-1 font-mono text-xs text-[var(--on-surface)]">{wireguardDebug.last_import_config_temp_path ?? "n/a"}</div>
                </div>
                <div className="rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-3 py-2">
                  <div className="text-[11px] uppercase tracking-[0.16em] text-[var(--on-surface-variant)]">Last Imported Config (Redacted)</div>
                  <pre className="mt-2 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-[color:rgba(8,10,16,0.92)] p-2 font-mono text-[12px] leading-5 text-[var(--on-surface)]">{wireguardDebug.last_import_config_redacted ?? "No config imported in this session yet."}</pre>
                </div>
                <div className="rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-3 py-2">
                  <div className="text-[11px] uppercase tracking-[0.16em] text-[var(--on-surface-variant)]">Recent Commands</div>
                  <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-[color:rgba(8,10,16,0.92)] p-2 font-mono text-[12px] leading-5 text-[var(--on-surface)]">{wireguardDebug.commands.length > 0 ? wireguardDebug.commands.join("\n") : "No WireGuard commands captured yet."}</pre>
                </div>
              </div>
            ) : (
              <div className="mt-3 rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-3 py-2 text-xs text-[var(--on-surface-variant)]">
                WireGuard debug data is not available yet. Run a Connect or Recover action and refresh diagnostics.
              </div>
            )}
          </div>
          <div className="mt-4 rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.7)] p-4">
            <div className="text-sm font-semibold text-[var(--on-surface)]">Init Output</div>
            <div className="mt-1 text-xs text-[var(--on-surface-variant)]">
              Shows per-run bootstrap output. In Docker development mode this appears as docker-init.
            </div>
            <div className="mt-3 max-h-64 space-y-2 overflow-auto">
              {initRuns.length === 0 ? (
                <div className="rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-3 py-2 text-xs text-[var(--on-surface-variant)]">
                  No init output captured yet. Run Install Host to generate cloud-init or docker-init entries.
                </div>
              ) : (
                initRuns.map((entry) => (
                  <div key={entry.id} className="rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-3 py-2">
                    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] uppercase tracking-[0.16em] text-[var(--on-surface-variant)]">
                      <span>{entry.init_kind}</span>
                      <span>{entry.stage}</span>
                      <span className={entry.outcome.toLowerCase() === "error" ? "text-[#ffd5d5]" : "text-[var(--secondary)]"}>{entry.outcome}</span>
                      <span>{entry.node_id}</span>
                    </div>
                    <div className="mt-1 text-xs text-[var(--on-surface-variant)]">{new Date(entry.created_at).toLocaleString()}</div>
                    <pre className="mt-2 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-[color:rgba(8,10,16,0.92)] p-2 font-mono text-[12px] leading-5 text-[var(--on-surface)]">{entry.output}</pre>
                  </div>
                ))
              )}
            </div>
          </div>
          {recovery ? <div className="mt-4 rounded-[16px] border border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] p-4 text-sm text-[var(--tertiary)]">{recovery.title}: {recovery.message}</div> : null}
        </Card>
      </div>
    </PageChrome>
  );
}
