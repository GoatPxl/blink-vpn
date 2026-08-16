"use client";

import {
  CONNECT_STEPS,
  CONNECT_STEP_LABEL,
  connectStepStates,
  deriveConnectStep,
  describeConnectProgress,
  formatElapsedMs,
} from "@/lib/connect-progress-display";
import { useDashboard } from "./dashboard-provider";
import { AppIcon, Button, useNow } from "./shared";

/** Tone tokens, matching the treatment the rest of the dashboard uses for each meaning. */
const TONE_VAR = {
  info: "var(--primary)",
  warning: "var(--tertiary)",
  positive: "var(--secondary)",
} as const;

/**
 * What is happening during the seconds the user has no internet.
 *
 * This card is the actual fix for the reported problem. The outage was always there — the WireGuard
 * adapter takes the default route the instant its service starts, and the peer does not answer for
 * some seconds after that. What was missing was any acknowledgement of it: the app announced
 * "Connected", the internet stopped working, and the window stopped repainting.
 *
 * Rendered from the shell rather than a page, because a connect survives navigation — the user may
 * start one from the Nodes table and end up on the dashboard while it runs.
 */
export function ConnectProgress() {
  const { connectAttempt, cancelConnect, busyAction } = useDashboard();
  // Only ticks while an attempt is live.
  const now = useNow(connectAttempt !== null);

  if (!connectAttempt) return null;

  const phase = connectAttempt.snapshot?.phase ?? null;
  const elapsedSeconds = Math.max(0, Math.floor((now - connectAttempt.startedAt) / 1000));
  const copy = describeConnectProgress({ phase, elapsedSeconds });
  const step = deriveConnectStep({
    creatingTunnel: connectAttempt.creatingTunnel,
    snapshot: connectAttempt.snapshot,
  });
  const stepStates = connectStepStates(step, phase);
  const tone = TONE_VAR[copy.tone];

  return (
    <div
      data-testid="connect-progress"
      data-phase={phase ?? "starting"}
      // Bottom-left, so it does not sit under the toast region top-right.
      className="fixed bottom-4 left-4 z-50 w-[min(92vw,26rem)] rounded-2xl border border-[color:var(--outline-variant)] bg-[color:rgba(15,18,26,0.98)] px-5 py-4 shadow-[var(--shadow-elevated)]"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-2" style={{ color: tone }}>
          <AppIcon name="shield" className="h-4 w-4" />
          <span className="text-[11px] font-semibold uppercase tracking-[0.24em]">
            {copy.headline}
          </span>
        </div>
        <span
          data-testid="connect-progress-elapsed"
          className="font-mono text-xs tabular-nums text-[var(--on-surface-variant)]"
        >
          {formatElapsedMs(connectAttempt.startedAt, now)}
        </span>
      </div>

      <p className="mt-2 text-sm leading-6 text-[var(--on-surface-variant)]">{copy.detail}</p>

      <ol className="mt-4 space-y-2">
        {CONNECT_STEPS.map((candidate) => {
          const state = stepStates[candidate];
          const color =
            state === "done"
              ? "var(--secondary)"
              : state === "active"
                ? tone
                : state === "failed"
                  ? "var(--tertiary)"
                  : "var(--on-surface-variant)";

          return (
            <li key={candidate} className="flex items-center gap-2.5 text-xs" style={{ color }}>
              <span className="flex h-4 w-4 shrink-0 items-center justify-center">
                {state === "done" ? (
                  <AppIcon name="check" className="h-3.5 w-3.5" />
                ) : state === "active" ? (
                  <AppIcon name="refresh" className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <span
                    className="h-1.5 w-1.5 rounded-full"
                    style={{ backgroundColor: "currentColor", opacity: state === "pending" ? 0.4 : 1 }}
                  />
                )}
              </span>
              <span className={state === "pending" ? "opacity-60" : undefined}>
                {CONNECT_STEP_LABEL[candidate]}
              </span>
            </li>
          );
        })}
      </ol>

      {/* Offered only once waiting has stopped looking normal. Shown earlier it would read as an
          invitation to abandon a connect that is progressing fine — and cancelling means tearing
          the tunnel down, which is not free. */}
      {copy.showDisconnectEscape ? (
        <div className="mt-4 flex justify-end">
          <Button
            tone="secondary"
            onClick={() => void cancelConnect()}
            disabled={busyAction !== null}
            className="px-3 py-2 text-xs"
          >
            {busyAction === "disconnect-tunnel" ? "Cancelling" : "Cancel and restore internet"}
          </Button>
        </div>
      ) : null}
    </div>
  );
}
