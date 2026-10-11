"use client";

import { PixelIcon } from "@/components/ui/pixel-icon";
import { UNDO_WINDOW_MS, useDashboard } from "./dashboard-provider";
import { Button, useNow } from "./shared";

const SEGMENTS = 6;

/**
 * "Powering off Frankfurt… Undo · 4": the one confirmation a destructive action gets.
 *
 * Six segments drain one a second. Undo cancels before anything reaches the provider; letting it
 * run out is the confirmation.
 */
export function RemovalToast() {
  const { pendingRemoval, undoRemoval } = useDashboard();
  const now = useNow(pendingRemoval !== null);

  if (!pendingRemoval) return null;

  const remainingMs = Math.max(0, pendingRemoval.runsAt - now);
  const seconds = Math.ceil(remainingMs / 1000);
  const lit = Math.ceil((remainingMs / UNDO_WINDOW_MS) * SEGMENTS);

  return (
    <div role="status" data-testid="removal-toast" className="flex items-center gap-4 border-2 border-bone bg-panel p-[14px] shadow-hard-alert">
      <PixelIcon name="power" className="h-7 w-7 shrink-0 text-alert" />
      <div className="min-w-0 flex-1">
        <div className="text-[15px] font-semibold text-bone">{pendingRemoval.title}</div>
        <div className="mt-0.5 text-[12.5px] text-stone">{pendingRemoval.detail}</div>
        <div aria-hidden="true" className="mt-2 grid grid-cols-6 gap-[3px]">
          {Array.from({ length: SEGMENTS }, (_, index) => (
            <span key={index} className={`h-1.5 ${index < lit ? "bg-alert" : "bg-track"}`} />
          ))}
        </div>
      </div>
      <Button tone="ghost" size="sm" className="!border-lime !text-lime" onClick={undoRemoval}>
        Undo · {seconds}
      </Button>
    </div>
  );
}
