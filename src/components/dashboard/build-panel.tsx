"use client";

import { Component, useCallback, useState, type ReactNode } from "react";

import { GoatInvaders } from "@/components/game/goat-invaders";
import { formatElapsedMs } from "@/lib/connect-progress-display";
import { describeProvisionProgress, PROVISION_STAGES } from "@/lib/progress-estimate";
import { useDashboard, type OperationAttempt } from "./dashboard-provider";
import { BlockProgress, Button, Panel, useNow } from "./shared";

type ProvisionAttempt = Extract<OperationAttempt, { kind: "provision" }>;

const TYPICAL_BUILD_MS = PROVISION_STAGES.reduce((sum, stage) => sum + stage.typicalMs, 0);

/**
 * Keeps a game fault inside the game panel.
 *
 * The status strip is rendered outside this boundary on purpose: whatever the game does, the user
 * can still see that their server is building and how far along it is.
 */
class GameBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    if (this.state.failed) {
      return <p className="px-7 py-10 text-[14px] text-stone">The game stopped. The build is unaffected.</p>;
    }
    return this.props.children;
  }
}

/** "BUILDING FRANKFURT", the clock against ~2:00, and the 40-block bar. */
export function BuildStrip({ attempt }: { attempt: ProvisionAttempt }) {
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
      <div className="mt-4 flex items-baseline gap-3">
        <span className="font-mono text-[44px] font-bold leading-none text-bone tabular-nums">
          {formatElapsedMs(attempt.startedAt, now)}
        </span>
        <span className="font-display text-[13px] uppercase text-stone">
          of ~{formatElapsedMs(0, TYPICAL_BUILD_MS)}
        </span>
      </div>
      <div className="mt-5">
        <BlockProgress percent={report.percent} blocks={40} tone="ember" height={20} line={report.line} />
      </div>
    </Panel>
  );
}

/**
 * The build screen: status always visible, the game optional underneath.
 *
 * Provisioning steps and the live log are not here; they are in Shield › Log for anyone who wants
 * them. This screen answers two questions — is it working, and how long — and offers something to
 * do while waiting.
 */
export function BuildPanel({ attempt }: { attempt: ProvisionAttempt }) {
  const { appPreferences, requestCancelBuild, updateAppPreferences } = useDashboard();
  const [gameVisible, setGameVisible] = useState(true);
  const highScore = appPreferences?.game_high_score ?? 0;

  const onGameOver = useCallback((score: number) => {
    if (score > highScore) void updateAppPreferences({ game_high_score: score });
  }, [highScore, updateAppPreferences]);

  return (
    <div data-testid="operation-progress" data-kind="provision" className="flex flex-1 flex-col gap-5">
      <span className="sr-only">{attempt.nodeName}</span>
      <BuildStrip attempt={attempt} />

      <Panel className="flex flex-1 flex-col" bodyClassName="flex flex-1 flex-col">
        {gameVisible ? (
          <div className="flex-1">
            <GameBoundary>
              <GoatInvaders highScore={highScore} onGameOver={onGameOver} onHide={() => setGameVisible(false)} />
            </GameBoundary>
          </div>
        ) : (
          <div className="flex flex-1 items-center justify-center px-7 py-12">
            <p className="text-[15px] text-stone">Game hidden. The build carries on either way.</p>
          </div>
        )}
        <div className="flex flex-wrap items-center gap-4 border-t-2 border-line px-7 py-5">
          <p className="min-w-0 flex-1 text-[14px] text-stone">
            Optional. Runs locally, no network. The build keeps going if you close this.
          </p>
          <Button tone="quiet" onClick={() => setGameVisible((visible) => !visible)}>
            {gameVisible ? "Hide game" : "Show game"}
          </Button>
          <Button tone="quiet" onClick={requestCancelBuild} disabled={attempt.cancelRequested}>
            {attempt.cancelRequested ? "Cancelling" : "Cancel build"}
          </Button>
        </div>
      </Panel>
    </div>
  );
}
