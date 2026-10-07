"use client";

import { formatElapsedMs } from "@/lib/connect-progress-display";
import { describeProvisionProgress, describeRemovalProgress } from "@/lib/progress-estimate";
import { useDashboard, type OperationAttempt } from "./dashboard-provider";
import { AppIcon, ProgressBar, useNow } from "./shared";

const HEADLINE: Record<OperationAttempt["kind"], string> = {
  provision: "Provisioning server",
  destroy: "Destroying server",
  "force-cleanup": "Forcing cleanup",
};

/**
 * Progress for a provision, destroy or force cleanup.
 *
 * Rendered from the shell rather than a page, because these outlive navigation: a finished
 * provision sends the user to the server list, and a destroy started from a details drawer keeps
 * running after the drawer closes. Bottom-right, so it does not cover the connect card bottom-left
 * or the toasts top-right.
 */
export function OperationProgress() {
  const { operationAttempt } = useDashboard();
  const now = useNow(operationAttempt !== null);

  if (!operationAttempt) return null;

  const report =
    operationAttempt.kind === "provision"
      ? describeProvisionProgress({
          status: operationAttempt.status,
          startedAt: operationAttempt.startedAt,
          bootstrappingSince: operationAttempt.bootstrappingSince,
          now,
          providerName: operationAttempt.providerName,
        })
      : describeRemovalProgress({
          kind: operationAttempt.kind,
          nodeName: operationAttempt.nodeName,
          providerName: operationAttempt.providerName,
          startedAt: operationAttempt.startedAt,
          now,
        });

  const tone = operationAttempt.kind === "provision" ? "var(--primary)" : "var(--tertiary)";

  return (
    <div
      data-testid="operation-progress"
      data-kind={operationAttempt.kind}
      className="fixed bottom-4 right-4 z-50 w-[min(92vw,24rem)] rounded-2xl border border-[color:var(--outline-variant)] bg-[color:rgba(15,18,26,0.98)] px-5 py-4 shadow-[var(--shadow-elevated)]"
    >
      <div className="mb-3 flex items-center gap-2" style={{ color: tone }}>
        <AppIcon name="server" className="h-4 w-4" />
        <span className="truncate text-[11px] font-semibold uppercase tracking-[0.24em]">
          {HEADLINE[operationAttempt.kind]} · {operationAttempt.nodeName}
        </span>
      </div>
      <ProgressBar
        percent={report.percent}
        line={report.line}
        tone={tone}
        trailing={formatElapsedMs(operationAttempt.startedAt, now)}
      />
    </div>
  );
}
