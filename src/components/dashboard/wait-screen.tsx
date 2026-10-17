"use client";

import { Component, useCallback, useEffect, useState, type ReactNode } from "react";

import { GoatInvaders } from "@/components/game/goat-invaders";
import {
  awaitingWindowsPermission,
  CONNECT_STEPS,
  CONNECT_STEP_LABEL,
  deriveConnectStep,
  describeConnectProgress,
  formatElapsedMs,
  outageElapsedSeconds,
  WINDOWS_PERMISSION_COPY,
} from "@/lib/connect-progress-display";
import { closeSession, openSession, setGameOver } from "@/lib/game/session";
import { describeProvisionProgress, estimateConnectProgress, PROVISION_STAGES } from "@/lib/progress-estimate";
import { useDashboard, type ConnectAttempt, type OperationAttempt } from "./dashboard-provider";
import { BlockProgress, Button, ConnectionTimer, Panel, Tag, useNow } from "./shared";

type ProvisionAttempt = Extract<OperationAttempt, { kind: "provision" }>;

const TYPICAL_BUILD_MS = PROVISION_STAGES.reduce((sum, stage) => sum + stage.typicalMs, 0);
/** When the first-time HOLD ON offers to roll back. Past a healthy handshake, well short of giving up. */
const ROLL_BACK_AFTER_SECONDS = 45;

/**
 * Keeps a game fault inside the game panel.
 *
 * Every strip renders outside this boundary on purpose: whatever the game does, the user can still
 * see that their server is building, or that their internet is paused and why.
 */
class GameBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    if (this.state.failed) {
      return <p className="px-7 py-10 text-[14px] text-stone">The game stopped. Nothing else is affected.</p>;
    }
    return this.props.children;
  }
}

/** BUILDING <CITY>, the clock against ~2:00, and the 40-block bar. */
function BuildStrip({ attempt }: { attempt: ProvisionAttempt }) {
  const { requestCancelBuild } = useDashboard();
  const now = useNow();
  const report = describeProvisionProgress({
    status: attempt.status,
    startedAt: attempt.startedAt,
    bootstrappingSince: attempt.bootstrappingSince,
    now,
    providerName: attempt.providerName,
  });

  return (
    <Panel tint="busy" bodyClassName="px-10 py-6 max-[700px]:px-5">
      <div data-testid="operation-progress" data-kind="provision">
        <span className="sr-only">{attempt.nodeName}</span>
        <div className="flex flex-wrap items-baseline gap-x-8 gap-y-2">
          <h1 className="font-display text-[clamp(32px,4vw,52px)] uppercase leading-none text-ember">
            {attempt.cancelRequested ? "Cancelling" : "Building"} {attempt.place}
          </h1>
          <p className="text-[16px] text-stone">
            {attempt.cancelRequested
              ? `Blink deletes it as soon as ${attempt.providerName} finishes creating it.`
              : `${report.line.replace(/…$/, "")} · you still have internet · connects by itself when ready`}
          </p>
        </div>
        <div className="mt-4 flex flex-wrap items-center gap-x-6 gap-y-3">
          <span className="flex items-baseline gap-3">
            <span className="font-mono text-[44px] font-bold leading-none text-bone tabular-nums">{formatElapsedMs(attempt.startedAt, now)}</span>
            <span className="font-display text-[13px] uppercase text-stone">of ~{formatElapsedMs(0, TYPICAL_BUILD_MS)}</span>
          </span>
          <span className="flex-1" />
          <Button tone="quiet" size="sm" onClick={requestCancelBuild} disabled={attempt.cancelRequested}>
            {attempt.cancelRequested ? "Cancelling" : "Cancel build"}
          </Button>
        </div>
        <div className="mt-4">
          <BlockProgress percent={report.percent} blocks={40} tone="ember" height={20} line={report.line} />
        </div>
      </div>
    </Panel>
  );
}

/**
 * HOLD ON, condensed to sit above the game: the user has no internet right now, so this says so,
 * counts, shows the bar, and keeps Roll back in reach. It ends only when inbound traffic proves the
 * tunnel works (AGENTS.md §4).
 *
 * Before that, while the UAC prompt is unanswered, the internet is fine and nothing is counting —
 * so the strip says it is waiting on Windows, shows no clock, and offers Cancel straight away.
 */
function HoldOnStrip({ attempt, place }: { attempt: ConnectAttempt; place: string }) {
  const { appPreferences, busyAction, cancelConnect } = useDashboard();
  const [explained, setExplained] = useState(false);
  const now = useNow();

  const phase = attempt.snapshot?.phase ?? null;
  const waitingForWindows = awaitingWindowsPermission(attempt);
  const elapsedSeconds = outageElapsedSeconds(attempt, now);
  const copy = describeConnectProgress({ phase, elapsedSeconds });
  const step = deriveConnectStep({ creatingTunnel: attempt.creatingTunnel, snapshot: attempt.snapshot });
  const percent = estimateConnectProgress({ step, elapsedMs: elapsedSeconds * 1000, done: phase === "connected" || phase === "degraded" });
  const firstTime = appPreferences !== null && !appPreferences.first_connect_done;
  const stalled = phase === "stalled";
  const rollBackReady = copy.showDisconnectEscape || elapsedSeconds >= ROLL_BACK_AFTER_SECONDS;
  const cancelling = busyAction === "disconnect-tunnel";
  const position = CONNECT_STEPS.indexOf(step);

  return (
    <Panel tint={stalled ? "failed" : "busy"} bodyClassName="px-10 py-6 max-[700px]:px-5">
      <div data-testid="connect-progress" data-phase={phase ?? "starting"}>
        <div className="flex flex-wrap items-center gap-x-8 gap-y-3">
          <div className="min-w-0">
            <Tag state={stalled ? "failed" : "busy"}>Connecting to {place}</Tag>
            <h1 className={`mt-1 font-display text-[clamp(32px,4vw,52px)] uppercase leading-none ${stalled ? "text-alert" : "text-ember"}`}>
              {waitingForWindows ? WINDOWS_PERMISSION_COPY.headline : stalled ? "Still waiting" : "Hold on"}
            </h1>
          </div>
          {attempt.serviceStartedAt !== null ? (
            <span
              data-testid="connect-progress-elapsed"
              className={`font-mono text-[44px] font-bold leading-none tabular-nums ${stalled ? "text-alert" : "text-ember"}`}
            >
              {formatElapsedMs(attempt.serviceStartedAt, now)}
            </span>
          ) : null}
          <span className="flex-1" />
          {waitingForWindows ? (
            // Nothing to roll back yet, so no countdown: leaving costs nothing.
            <Button tone="destructive" size="sm" onClick={() => void cancelConnect()} disabled={cancelling}>
              Cancel
            </Button>
          ) : firstTime || rollBackReady ? (
            <Button tone="destructive" size="sm" onClick={() => void cancelConnect()} disabled={!rollBackReady || cancelling}>
              {cancelling ? "Rolling back" : rollBackReady ? "Roll back" : `Roll back · ${ROLL_BACK_AFTER_SECONDS - elapsedSeconds}`}
            </Button>
          ) : null}
        </div>
        <p className="mt-3 text-[16px] font-semibold text-bone">
          {waitingForWindows
            ? WINDOWS_PERMISSION_COPY.detail
            : stalled
              ? `Your server has not answered after ${elapsedSeconds} s. Your internet stays paused until it does.`
              : "Internet pauses for ~15 s while the tunnel shakes hands. Don't disconnect."}
        </p>
        <div className="mt-4">
          <BlockProgress percent={percent} blocks={30} tone={stalled ? "alert" : "ember"} height={18} line={`${CONNECT_STEP_LABEL[step]}…`} />
        </div>
        {firstTime ? (
          <ol aria-label="Connect steps" className="mt-4 flex flex-wrap gap-2">
            {CONNECT_STEPS.map((candidate, index) => (
              <li
                key={candidate}
                className={`border-2 px-3 py-1 text-[12.5px] ${index < position ? "border-ember bg-ember text-ink" : index === position ? "border-ember text-ember" : "border-line text-stone-2"}`}
              >
                {CONNECT_STEP_LABEL[candidate]}
                {index < position ? <span className="sr-only"> (done)</span> : index === position ? <span className="sr-only"> (in progress)</span> : null}
              </li>
            ))}
          </ol>
        ) : null}
        {waitingForWindows ? null : (
          <button
            type="button"
            aria-expanded={explained}
            onClick={() => setExplained((value) => !value)}
            className="mt-3 border-b-2 border-stone text-[14px] text-stone hover:text-bone"
          >
            Why does this happen? →
          </button>
        )}
        {explained && !waitingForWindows ? (
          <p className="mt-2 max-w-[760px] text-[14px] text-stone">
            The moment the tunnel starts, Windows sends all your traffic into it — that is the kill switch
            working. Until your server answers the first handshake there is nowhere for that traffic to go.
            Blink only calls it secure once data has actually come back through.
          </p>
        ) : null}
      </div>
    </Panel>
  );
}

/** After the wait, while a game is still going: what happened, and the way back to Home. */
function AfterStrip({ onDone }: { onDone: () => void }) {
  const { activeNode, health, tunnelConnected } = useDashboard();
  const place = activeNode ? activeNode.region_geo?.city ?? activeNode.name : null;

  if (tunnelConnected) {
    return (
      <Panel tint="secure" bodyClassName="flex flex-wrap items-center gap-x-8 gap-y-3 px-10 py-6 max-[700px]:px-5">
        <div className="min-w-0">
          <Tag state="secure">You&apos;re protected</Tag>
          <h1 className="mt-1 font-display text-[clamp(32px,4vw,52px)] uppercase leading-none text-lime">
            Secure{place ? ` · ${place}` : ""}
          </h1>
        </div>
        <span className="font-mono text-[44px] font-bold leading-none text-bone tabular-nums">
          {health?.connected_at ? <ConnectionTimer startedAt={health.connected_at} /> : null}
        </span>
        <span className="flex-1" />
        <Button onClick={onDone}>Done</Button>
      </Panel>
    );
  }

  // A build that failed or was cancelled: nothing to celebrate, and nothing hidden either.
  return (
    <Panel bodyClassName="flex flex-wrap items-center gap-x-8 gap-y-3 px-10 py-6 max-[700px]:px-5">
      <div className="min-w-0 flex-1">
        <Tag>Nothing in progress</Tag>
        <p className="mt-1 text-[16px] text-stone">The wait is over. Finish your game, or head back.</p>
      </div>
      <Button tone="ghost" onClick={onDone}>Back to Home</Button>
    </Panel>
  );
}

/**
 * Everything between "Build" and "Secure", on one screen that stays mounted.
 *
 * The status strip changes — BUILDING, then HOLD ON, then SECURE — and the game beneath it carries
 * on through all three, so a two-minute build and a slow first handshake are one game, not two.
 * Once secure the game keeps going until the user says Done, presses Esc, or runs out of lives.
 */
export function WaitScreen({ place }: { place: string }) {
  const { appPreferences, busyAction, connectAttempt, operationAttempt, uacPrompt, updateAppPreferences } = useDashboard();
  const [gameVisible, setGameVisible] = useState(true);
  const highScore = appPreferences?.game_high_score ?? 0;
  const waiting = connectAttempt !== null || operationAttempt?.kind === "provision";
  // Every moment Windows may be asking for permission, plus the explainer that leads into it. The
  // user's attention is on the prompt, not the game, so the game must not be losing lives meanwhile.
  const gamePaused =
    uacPrompt !== null ||
    (connectAttempt !== null && awaitingWindowsPermission(connectAttempt)) ||
    busyAction === "disconnect-tunnel";

  useEffect(() => {
    if (gameVisible) openSession();
  }, [gameVisible]);

  const onGameOver = useCallback((score: number) => {
    if (score > highScore) void updateAppPreferences({ game_high_score: score });
  }, [highScore, updateAppPreferences]);

  const onStatusChange = useCallback((status: "playing" | "over") => setGameOver(status === "over"), []);

  const leave = useCallback(() => {
    setGameVisible(false);
    closeSession();
  }, []);

  return (
    <div className="flex flex-1 flex-col gap-5">
      {operationAttempt?.kind === "provision" ? (
        <BuildStrip attempt={operationAttempt} />
      ) : connectAttempt ? (
        <HoldOnStrip attempt={connectAttempt} place={place} />
      ) : (
        <AfterStrip onDone={leave} />
      )}

      <Panel className="flex flex-1 flex-col" bodyClassName="flex flex-1 flex-col">
        {gameVisible ? (
          <div className="flex-1">
            <GameBoundary>
              <GoatInvaders highScore={highScore} onGameOver={onGameOver} onStatusChange={onStatusChange} onHide={leave} paused={gamePaused} />
            </GameBoundary>
          </div>
        ) : (
          <div className="flex flex-1 items-center justify-center px-7 py-12">
            <p className="text-[15px] text-stone">Game hidden. {waiting ? "Everything above carries on either way." : ""}</p>
          </div>
        )}
        <div className="flex flex-wrap items-center gap-4 border-t-2 border-line px-7 py-5">
          <p className="min-w-0 flex-1 text-[14px] text-stone">
            Optional. Runs locally, no network. {waiting ? "Nothing waits on it — the build and the connect carry on whatever you do here." : "Your tunnel is unaffected by the game."}
          </p>
          {gameVisible ? (
            <Button tone="quiet" onClick={leave}>Hide game</Button>
          ) : (
            <Button tone="quiet" onClick={() => setGameVisible(true)}>Show game</Button>
          )}
        </div>
      </Panel>
    </div>
  );
}
