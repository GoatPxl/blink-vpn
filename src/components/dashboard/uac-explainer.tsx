"use client";

import { useState } from "react";

import { PixelIcon } from "@/components/ui/pixel-icon";
import { useDashboard } from "./dashboard-provider";
import { Button, Modal } from "./shared";

/**
 * Before a connect: say that Windows will ask for permission, and that the internet pauses.
 *
 * A UAC prompt out of nowhere reads as something suspicious happening, and the pause that follows
 * reads as a fault. Saying both first turns them into expected steps. Shown before the first ever
 * connect; after that only if the user un-ticks "don't explain this again".
 */
export function UacExplainer() {
  const { uacPrompt, answerUacPrompt } = useDashboard();
  const [dontExplainAgain, setDontExplainAgain] = useState(true);

  if (!uacPrompt) return null;

  return (
    <Modal onClose={() => answerUacPrompt(false, false)} labelledBy="uac-explainer-title">
      <div className="flex items-start gap-5">
        <PixelIcon name="shield" className="mt-1 h-9 w-9 shrink-0 text-lime" />
        <div className="min-w-0">
          <h2 id="uac-explainer-title" className="font-display text-[20px] uppercase text-bone">
            One Windows prompt is coming
          </h2>
          <ol className="mt-4 space-y-3 text-[15px] text-stone">
            <li><span className="text-bone">Windows asks for permission</span> to start the tunnel. That is the WireGuard service, not Blink, asking.</li>
            <li><span className="text-bone">Your internet pauses for about 15 seconds</span> while the tunnel reaches {uacPrompt.place}.</li>
            <li><span className="text-bone">Then you&apos;re secure.</span> Blink says so only once data has come back through the tunnel.</li>
          </ol>
        </div>
      </div>

      <div className="mt-6 flex flex-wrap items-center justify-between gap-4 border-t-2 border-line pt-5">
        <label className="flex cursor-pointer items-center gap-3 text-[14px] text-stone">
          <input
            type="checkbox"
            checked={dontExplainAgain}
            onChange={(event) => setDontExplainAgain(event.target.checked)}
            className="h-5 w-5 accent-[var(--lime)]"
          />
          Don&apos;t explain this again
        </label>
        <div className="flex gap-3">
          <Button tone="quiet" onClick={() => answerUacPrompt(false, false)}>Not now</Button>
          <Button onClick={() => answerUacPrompt(true, dontExplainAgain)}>Continue</Button>
        </div>
      </div>
    </Modal>
  );
}
