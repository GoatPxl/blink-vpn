"use client";

import Link from "next/link";

import { awaitingWindowsPermission, formatElapsedMs } from "@/lib/connect-progress-display";
import { describeRemovalProgress } from "@/lib/progress-estimate";
import { PixelIcon } from "@/components/ui/pixel-icon";
import { useDashboard } from "./dashboard-provider";
import { BlockProgress, Tag, useNow, VIEW_ROUTE } from "./shared";

/**
 * One line of ember at the top of the content, for work in flight that is not on screen.
 *
 * A build or a connect gets its full panel on Home; anywhere else it gets this, so the user can
 * keep doing what they were doing and still see that their internet is about to pause. A power-off
 * has no full panel and always shows here.
 */
export function OperationStrip({ onHome }: { onHome: boolean }) {
  const { connectAttempt, operationAttempt, nodes } = useDashboard();
  const now = useNow(connectAttempt !== null || operationAttempt !== null);

  if (connectAttempt && !onHome) {
    const node = nodes.find((candidate) => candidate.id === connectAttempt.nodeId);
    const waitingForWindows = awaitingWindowsPermission(connectAttempt);
    return (
      <Link
        href={VIEW_ROUTE.home}
        data-testid="connect-progress"
        data-phase={connectAttempt.snapshot?.phase ?? "starting"}
        className="flex items-center gap-5 border-2 border-ember bg-ember-ink px-6 py-3"
      >
        <Tag state="busy">{waitingForWindows ? "Waiting for Windows permission" : "Hold on"} · connecting to {node?.region_geo?.city ?? node?.name ?? connectAttempt.tunnelName}</Tag>
        <span className="flex-1 text-[14px] text-stone">
          {waitingForWindows ? "Approve the Windows prompt. Your internet works normally until then." : "Internet pauses until the handshake lands."}
        </span>
        {connectAttempt.serviceStartedAt !== null ? (
          <span data-testid="connect-progress-elapsed" className="font-mono text-[16px] text-ember tabular-nums">
            {formatElapsedMs(connectAttempt.serviceStartedAt, now)}
          </span>
        ) : null}
        <PixelIcon name="arrow-right" className="h-4 w-4 text-ember" />
      </Link>
    );
  }

  if (operationAttempt?.kind === "provision" && !onHome) {
    return (
      <Link
        href={VIEW_ROUTE.home}
        data-testid="operation-progress"
        data-kind="provision"
        className="flex items-center gap-5 border-2 border-ember bg-ember-ink px-6 py-3"
      >
        <Tag state="busy">Building {operationAttempt.place}</Tag>
        <span className="flex-1 truncate text-[14px] text-stone">{operationAttempt.nodeName} · you still have internet</span>
        <span className="font-mono text-[16px] text-ember tabular-nums">{formatElapsedMs(operationAttempt.startedAt, now)}</span>
        <PixelIcon name="arrow-right" className="h-4 w-4 text-ember" />
      </Link>
    );
  }

  if (operationAttempt && operationAttempt.kind !== "provision") {
    const report = describeRemovalProgress({
      kind: operationAttempt.kind,
      nodeName: operationAttempt.nodeName,
      providerName: operationAttempt.providerName,
      startedAt: operationAttempt.startedAt,
      now,
    });
    return (
      <div
        data-testid="operation-progress"
        data-kind={operationAttempt.kind}
        className="border-2 border-ember bg-ember-ink px-6 py-4"
      >
        <div className="mb-3 flex items-center justify-between gap-4">
          <Tag state="busy">{operationAttempt.kind === "destroy" ? "Powering off" : "Cleaning up"} · {operationAttempt.nodeName}</Tag>
          <span className="font-mono text-[14px] text-ember tabular-nums">{formatElapsedMs(operationAttempt.startedAt, now)}</span>
        </div>
        <BlockProgress percent={report.percent} blocks={40} tone="ember" height={12} line={report.line} />
      </div>
    );
  }

  return null;
}
