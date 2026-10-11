"use client";

import { useState } from "react";

import {
  CONNECT_STEPS,
  CONNECT_STEP_LABEL,
  deriveConnectStep,
  describeConnectProgress,
  formatElapsedMs,
} from "@/lib/connect-progress-display";
import { estimateConnectProgress } from "@/lib/progress-estimate";
import { useDashboard, type ConnectAttempt } from "./dashboard-provider";
import { BlockProgress, Button, Panel, Tag, useNow } from "./shared";

/** When the first-time screen offers to roll back. Past a healthy handshake, well short of giving up. */
const ROLL_BACK_AFTER_SECONDS = 45;

/**
 * HOLD ON: the seconds the user has no internet while the tunnel shakes hands.
 *
 * The outage is real — the adapter takes the default route the instant its service starts, and the
 * peer answers some seconds later. This screen is what makes it a step instead of a fault. It flips
 * back to Home the moment `TunnelPhase` says inbound traffic has arrived (AGENTS.md §4); service
 * state alone never ends it. There is no game here, ever: the user should be watching this.
 *
 * Until the first connect has worked, it also lists the steps and offers a roll back at 45 s.
 */
export function HoldOnPanel({ attempt, place }: { attempt: ConnectAttempt; place: string }) {
  const { appPreferences, busyAction, cancelConnect } = useDashboard();
  const [explained, setExplained] = useState(false);
  const now = useNow();

  const phase = attempt.snapshot?.phase ?? null;
  const elapsedSeconds = Math.max(0, Math.floor((now - attempt.startedAt) / 1000));
  const copy = describeConnectProgress({ phase, elapsedSeconds });
  const step = deriveConnectStep({ creatingTunnel: attempt.creatingTunnel, snapshot: attempt.snapshot });
  const percent = estimateConnectProgress({
    step,
    elapsedMs: now - attempt.startedAt,
    done: phase === "connected" || phase === "degraded",
  });
  const firstTime = appPreferences !== null && !appPreferences.first_connect_done;
  const stalled = phase === "stalled";
  const rollBackReady = copy.showDisconnectEscape || elapsedSeconds >= ROLL_BACK_AFTER_SECONDS;
  const cancelling = busyAction === "disconnect-tunnel";

  const rollBack = (
    <Button tone="destructive" onClick={() => void cancelConnect()} disabled={!rollBackReady || cancelling}>
      {cancelling ? "Rolling back" : rollBackReady ? "Roll back" : `Roll back · ${ROLL_BACK_AFTER_SECONDS - elapsedSeconds}`}
    </Button>
  );

  return (
    <Panel
      tint={stalled ? "failed" : "busy"}
      className="flex flex-1 flex-col"
      bodyClassName="flex flex-1 flex-col items-center justify-center gap-8 px-8 py-12 text-center"
    >
      <div data-testid="connect-progress" data-phase={phase ?? "starting"} className="flex flex-col items-center gap-6">
        <Tag state={stalled ? "failed" : "busy"}>Connecting to {place}</Tag>
        <h1 className={`font-display text-[clamp(72px,11vw,160px)] uppercase leading-[0.9] ${stalled ? "text-alert" : "text-ember"}`}>
          {stalled ? "Still waiting" : "Hold on"}
        </h1>
        <span
          data-testid="connect-progress-elapsed"
          className={`font-mono text-[72px] font-bold leading-none tabular-nums ${stalled ? "text-alert" : "text-ember"}`}
        >
          {formatElapsedMs(attempt.startedAt, now)}
        </span>
        <p className="max-w-[760px] text-[clamp(18px,1.8vw,24px)] font-semibold text-bone">
          {stalled
            ? `Your server has not answered after ${elapsedSeconds} s. Your internet stays paused until it does.`
            : "Internet pauses for ~15 s while the tunnel shakes hands. Don't disconnect."}
        </p>
        <div className="w-full max-w-[760px]">
          <BlockProgress percent={percent} blocks={30} tone={stalled ? "alert" : "ember"} height={22} line={`${CONNECT_STEP_LABEL[step]}…`} />
        </div>
      </div>

      {firstTime ? (
        <div className="grid w-full max-w-[960px] grid-cols-2 gap-5 text-left max-[900px]:grid-cols-1">
          <div className="border-2 border-line bg-ink-2 p-6">
            <Tag>What&apos;s happening</Tag>
            <ol className="mt-4 space-y-3">
              {CONNECT_STEPS.map((candidate, index) => {
                const position = CONNECT_STEPS.indexOf(step);
                const state = index < position ? "done" : index === position ? "now" : "later";
                return (
                  <li key={candidate} className="flex items-center gap-3 text-[15px]">
                    <span aria-hidden="true" className={`h-3 w-3 shrink-0 ${state === "done" ? "bg-ember" : state === "now" ? "bg-ember-dim" : "bg-ember-track"}`} />
                    <span className={state === "later" ? "text-stone" : "text-bone"}>{CONNECT_STEP_LABEL[candidate]}</span>
                    {state === "done" ? <span className="sr-only">(done)</span> : state === "now" ? <span className="sr-only">(in progress)</span> : null}
                  </li>
                );
              })}
            </ol>
          </div>
          <div className="flex flex-col border-2 border-line bg-ink-2 p-6">
            <Tag>If it takes longer than {ROLL_BACK_AFTER_SECONDS} s</Tag>
            <p className="mt-4 flex-1 text-[14.5px] text-stone">
              The server may still be booting, or something between you and it is blocking UDP. Rolling
              back removes the tunnel and gives you your normal internet straight away. The server keeps
              running, so you can try again.
            </p>
            <div className="mt-5">{rollBack}</div>
          </div>
        </div>
      ) : (
        <div className="flex flex-col items-center gap-4">
          {rollBackReady ? rollBack : null}
          <button
            type="button"
            aria-expanded={explained}
            onClick={() => setExplained((value) => !value)}
            className="border-b-2 border-stone text-[15px] text-stone hover:text-bone"
          >
            Why does this happen? →
          </button>
          {explained ? (
            <p className="max-w-[640px] text-[14px] text-stone">
              The moment the tunnel starts, Windows sends all your traffic into it — that is the kill
              switch working. Until your server answers the first handshake there is nowhere for that
              traffic to go. Blink only calls it secure once data has actually come back through.
            </p>
          ) : null}
        </div>
      )}
    </Panel>
  );
}
