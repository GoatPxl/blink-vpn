"use client";

import { useMemo, useState } from "react";

import { AppIcon, Button, Card, PageChrome, useNow, type DashboardBusyAction, type DiagnosticsBundle } from "./shared";
import {
  ACTIVITY_TABS,
  describeRecovery,
  describeRunHeadline,
  failingStep,
  filterActivity,
  formatRelativeTime,
  formatStepTime,
  groupActivity,
  selectUnresolvedFailure,
  type ActivityOutcome,
  type ActivityRun,
  type ActivityTab,
} from "@/lib/activity-display";
import type { AppLocation, VpnNodeRecord } from "@/lib/tauri";

const OUTCOME_STYLES: Record<ActivityOutcome, { dot: string; text: string }> = {
  success: { dot: "bg-[var(--secondary)]", text: "text-[var(--secondary)]" },
  error: { dot: "bg-[#ff9b9b]", text: "text-[#ff9b9b]" },
  info: { dot: "bg-[var(--on-surface-variant)]", text: "text-[var(--on-surface-variant)]" },
};

function RunEntry({ run, headline, defaultOpen }: { run: ActivityRun; headline: string; defaultOpen: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  const style = OUTCOME_STYLES[run.outcome];

  return (
    <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-[color:rgba(255,255,255,0.03)]"
      >
        <span className={`h-2 w-2 shrink-0 rounded-full ${style.dot}`} />
        <span className={`flex-1 text-sm ${run.outcome === "error" ? style.text : "text-[var(--on-surface)]"}`}>
          {headline}
        </span>
        <span className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">
          {run.steps.length} {run.steps.length === 1 ? "step" : "steps"}
        </span>
        <AppIcon
          name="chevron"
          className={`h-4 w-4 text-[var(--on-surface-variant)] transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>

      {open ? (
        <div className="border-t border-[color:var(--outline-variant)] px-4 py-3 font-mono text-[12px] leading-6">
          {run.steps.map((step) => (
            <div key={step.id} className="flex gap-3">
              <span className="w-[76px] shrink-0 text-[var(--on-surface-variant)]">{formatStepTime(step.createdAt)}</span>
              <span className={`w-[140px] shrink-0 truncate ${OUTCOME_STYLES[step.outcome].text}`} title={step.stage}>
                {step.stage}
              </span>
              <span className="flex-1 break-all text-[var(--on-surface)]">{step.detail}</span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

export function ActivityView({
  diagnostics,
  diagnosticsExportPath,
  locations,
  nodes,
  busyAction,
  onCopyDiagnostics,
  onExportDiagnostics,
  onRevealLocation,
  onCopyText,
  onRecoverNode,
}: {
  diagnostics: DiagnosticsBundle | null;
  diagnosticsExportPath: string | null;
  locations: AppLocation[];
  nodes: VpnNodeRecord[];
  busyAction: DashboardBusyAction;
  onCopyDiagnostics: () => Promise<void>;
  onExportDiagnostics: () => Promise<void>;
  onRevealLocation: (key: string) => Promise<void>;
  onCopyText: (value: string, label: string) => Promise<void>;
  onRecoverNode: ((nodeId: string) => Promise<void>) | null;
}) {
  const [tab, setTab] = useState<ActivityTab>("all");
  // One clock for the page. Every run shows a relative time, and giving each its own `useNow` would
  // leave one interval per row — up to two hundred timers to update text that changes by the minute.
  const now = useNow();

  const runs = useMemo(() => groupActivity(diagnostics?.activity), [diagnostics?.activity]);
  const visible = useMemo(() => filterActivity(runs, tab), [runs, tab]);
  const existingNodeIds = useMemo(() => new Set(nodes.map((node) => node.id)), [nodes]);
  const latestFailure = useMemo(
    () => selectUnresolvedFailure(runs, existingNodeIds),
    [existingNodeIds, runs],
  );
  const failure = latestFailure ? failingStep(latestFailure) : null;
  const recovery = latestFailure ? describeRecovery(latestFailure) : null;
  const busy = busyAction !== null;

  return (
    <PageChrome>
      {/* The reason this page exists. A failed provision used to leave nothing on screen that said
          what went wrong — the cause was attached to the error and never shown. */}
      {latestFailure && failure ? (
        <div className="rounded-[20px] border border-[color:rgba(255,155,155,0.4)] bg-[color:rgba(255,155,155,0.09)] px-5 py-4">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-[#ff9b9b]">
                <AppIcon name="shield" className="h-4 w-4" />
                <span className="text-[11px] font-semibold uppercase tracking-[0.24em]">Last failure</span>
                <span className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">
                  {formatRelativeTime(latestFailure.latestAt, now) ?? "unknown time"}
                </span>
              </div>
              <p className="mt-2 max-w-3xl break-words text-sm text-[var(--on-surface)]">{failure.detail}</p>
              {recovery ? (
                <p className="mt-2 max-w-3xl text-sm text-[var(--on-surface-variant)]">{recovery.label}</p>
              ) : null}
            </div>
            {recovery?.action === "cleanup" && latestFailure.nodeId && onRecoverNode ? (
              <Button
                tone="danger"
                disabled={busy}
                onClick={() => void onRecoverNode(latestFailure.nodeId as string)}
              >
                {busyAction === "force-cleanup" ? "Cleaning up" : "Force Cleanup"}
              </Button>
            ) : null}
          </div>
        </div>
      ) : null}

      <Card
        title="History"
        description="Everything the app has done on this machine, kept across restarts."
        action={
          <div className="flex flex-wrap items-center gap-2">
            {ACTIVITY_TABS.map((option) => (
              <button
                key={option.id}
                type="button"
                onClick={() => setTab(option.id)}
                aria-pressed={tab === option.id}
                className={`rounded-[10px] border px-3 py-1.5 text-xs transition-colors ${tab === option.id
                  ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]"
                  : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)] hover:text-[var(--on-surface)]"}`}
              >
                {option.label}
              </button>
            ))}
          </div>
        }
      >
        {visible.length === 0 ? (
          <div className="rounded-[10px] border border-dashed border-[color:var(--outline-variant)] px-4 py-6 text-sm text-[var(--on-surface-variant)]">
            {diagnostics === null
              ? "Loading history."
              : tab === "failures"
                ? "Nothing has failed. That is the intended state."
                : runs.length === 0
                  ? "Nothing recorded yet. Provisioning a server or connecting will show up here."
                  : "No entries in this category."}
          </div>
        ) : (
          <div className="space-y-2">
            {visible.map((run, index) => (
              // The newest run is opened by default when it failed: it is what the user came to read.
              <RunEntry
                key={run.id}
                run={run}
                headline={describeRunHeadline(run, now)}
                defaultOpen={index === 0 && run.outcome === "error"}
              />
            ))}
          </div>
        )}
      </Card>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
        <Card title="Report a problem" description="Export a redacted snapshot to attach to a bug report.">
          <p className="text-sm leading-6 text-[var(--on-surface-variant)]">
            The bundle carries provider, node and tunnel records plus this history. API tokens, private
            keys and credential references are stripped before it is written.
          </p>

          <div className="mt-4 flex flex-wrap items-center gap-2">
            <Button onClick={() => void onExportDiagnostics()} disabled={busy}>
              {busyAction === "export-diagnostics" ? "Exporting" : "Export bundle"}
            </Button>
            <Button tone="secondary" onClick={() => void onCopyDiagnostics()} disabled={busy || !diagnostics}>
              {busyAction === "copy-diagnostics" ? "Copying" : "Copy to clipboard"}
            </Button>
          </div>

          {diagnosticsExportPath ? (
            <div className="mt-4 rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3">
              <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Last export</div>
              <div className="mt-1 break-all font-mono text-[12px] text-[var(--on-surface)]">{diagnosticsExportPath}</div>
            </div>
          ) : null}

          <details className="mt-4 rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]">
            <summary className="cursor-pointer px-4 py-3 text-sm text-[var(--on-surface-variant)]">
              Raw bundle
            </summary>
            <pre className="max-h-[360px] overflow-auto border-t border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.96)] p-4 font-mono text-[12px] leading-6 text-[var(--on-surface)]">
              {diagnostics ? JSON.stringify(diagnostics, null, 2) : "Loading."}
            </pre>
          </details>
        </Card>

        <Card title="Where things are" description="The folders this app writes to on your machine.">
          {locations.length === 0 ? (
            <div className="rounded-[10px] border border-dashed border-[color:var(--outline-variant)] px-4 py-6 text-sm text-[var(--on-surface-variant)]">
              Locations are still loading.
            </div>
          ) : (
            <div className="space-y-2">
              {locations.map((location) => (
                <div
                  key={location.key}
                  className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3"
                >
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="text-sm font-medium text-[var(--on-surface)]">{location.label}</div>
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        onClick={() => void onCopyText(location.path, `${location.label} path`)}
                        className="rounded-[8px] border border-[color:var(--outline-variant)] px-2.5 py-1 text-xs text-[var(--on-surface-variant)] transition-colors hover:text-[var(--on-surface)]"
                      >
                        Copy path
                      </button>
                      <button
                        type="button"
                        onClick={() => void onRevealLocation(location.key)}
                        disabled={!location.exists}
                        title={location.exists ? undefined : "Nothing has been written here yet."}
                        className="rounded-[8px] border border-[color:var(--outline-variant)] px-2.5 py-1 text-xs text-[var(--on-surface-variant)] transition-colors hover:text-[var(--on-surface)] disabled:cursor-not-allowed disabled:opacity-40"
                      >
                        Open
                      </button>
                    </div>
                  </div>
                  <p className="mt-1 text-xs text-[var(--on-surface-variant)]">{location.description}</p>
                  <div className="mt-2 break-all font-mono text-[11px] text-[var(--on-surface-variant)]">
                    {location.path}
                  </div>
                </div>
              ))}
            </div>
          )}
        </Card>
      </div>
    </PageChrome>
  );
}
