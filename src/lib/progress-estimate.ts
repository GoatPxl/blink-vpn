/**
 * Estimated progress for long operations: a percentage and one line saying what is happening.
 *
 * Nothing in the backend reports a percentage. Provisioning exposes only a lifecycle status, a
 * destroy exposes nothing until it returns, and a connect exposes a phase. So the bar is a guess:
 * each operation is a list of stages with a typical duration, and the bar moves through them on
 * the clock, jumping ahead whenever the backend reports a later stage.
 *
 * The guess is shaped so it cannot mislead in the two ways a fake bar usually does:
 * - it never reaches 100% until the operation has actually finished, and
 * - it never moves backwards.
 * Both follow from the curve, not from any state: a finished stage counts in full, and the stage in
 * progress fills along `1 - e^(-t/typical)`, which rises quickly and then flattens out without ever
 * reaching its full share. A slow server makes the bar crawl rather than stop or lie.
 *
 * Pure, with no React, so it can be unit tested directly like the other `*-display.ts` modules.
 */

import type { ConnectStep } from "./connect-progress-display.ts";
import { CONNECT_STEPS } from "./connect-progress-display.ts";
import type { NodeLifecycleStatus } from "./tauri";

export type ProgressStage = {
  /** How long this stage typically takes. Also its share of the bar. */
  typicalMs: number;
};

/** Highest the bar goes before the operation reports that it is done. */
export const UNFINISHED_CAP_PERCENT = 95;

/**
 * Percentage complete, 0–100.
 *
 * @param stageIndex the stage in progress. Callers must only ever advance it.
 * @param stageElapsedMs time spent in that stage so far.
 */
export function estimateProgress(input: {
  stages: readonly ProgressStage[];
  stageIndex: number;
  stageElapsedMs: number;
  done: boolean;
}): number {
  const { stages, done } = input;
  if (done) return 100;
  if (stages.length === 0) return 0;

  const totalMs = stages.reduce((sum, stage) => sum + stage.typicalMs, 0);
  if (totalMs <= 0) return 0;

  const stageIndex = Math.min(Math.max(0, Math.floor(input.stageIndex)), stages.length - 1);
  const stageElapsedMs = Math.max(0, input.stageElapsedMs);

  let completedMs = 0;
  for (let index = 0; index < stageIndex; index += 1) {
    completedMs += stages[index].typicalMs;
  }

  const current = stages[stageIndex];
  const fill = current.typicalMs > 0 ? 1 - Math.exp(-stageElapsedMs / current.typicalMs) : 1;
  const percent = ((completedMs + current.typicalMs * fill) / totalMs) * 100;

  return Math.min(UNFINISHED_CAP_PERCENT, percent);
}

/** What the progress card renders. */
export type ProgressReport = {
  percent: number;
  /** One line, present tense. */
  line: string;
};

// ---------------------------------------------------------------------------------------------
// Provisioning
// ---------------------------------------------------------------------------------------------

/**
 * Provisioning, from the click to a server that is ready to connect to.
 *
 * The first two stages share the backend's `provisioning` status — keys are generated and the
 * payload rendered before the provider is called, with no signal in between — so the bar moves from
 * one to the other on the clock alone. `bootstrapping` is a real signal: the server exists and the
 * app is waiting for it to boot.
 */
export const PROVISION_STAGES = [
  { typicalMs: 5_000 },
  { typicalMs: 25_000 },
  { typicalMs: 90_000 },
] as const satisfies readonly ProgressStage[];

const PROVISION_PREPARE_MS = PROVISION_STAGES[0].typicalMs;

export function describeProvisionProgress(input: {
  /** Latest polled status; null before the first poll lands. */
  status: NodeLifecycleStatus | null;
  /** `Date.now()` when the user started provisioning. */
  startedAt: number;
  /** `Date.now()` when `bootstrapping` was first observed, or null. */
  bootstrappingSince: number | null;
  now: number;
  providerName: string;
}): ProgressReport {
  const { status, startedAt, bootstrappingSince, now, providerName } = input;

  if (status === "ready" || status === "connected") {
    return { percent: 100, line: "Server ready." };
  }

  if (status === "bootstrapping" || bootstrappingSince !== null) {
    return {
      percent: estimateProgress({
        stages: PROVISION_STAGES,
        stageIndex: 2,
        stageElapsedMs: now - (bootstrappingSince ?? now),
        done: false,
      }),
      line: "Waiting for the server to boot and start WireGuard…",
    };
  }

  const elapsedMs = Math.max(0, now - startedAt);
  if (elapsedMs < PROVISION_PREPARE_MS) {
    return {
      percent: estimateProgress({
        stages: PROVISION_STAGES,
        stageIndex: 0,
        stageElapsedMs: elapsedMs,
        done: false,
      }),
      line: "Generating keys and server config…",
    };
  }

  return {
    percent: estimateProgress({
      stages: PROVISION_STAGES,
      stageIndex: 1,
      stageElapsedMs: elapsedMs - PROVISION_PREPARE_MS,
      done: false,
    }),
    line: `Creating the server on ${providerName}…`,
  };
}

// ---------------------------------------------------------------------------------------------
// Destroy and force cleanup
// ---------------------------------------------------------------------------------------------

/** A destroy is one provider call and a local cleanup, with nothing to observe in between. */
export const DESTROY_STAGES = [{ typicalMs: 20_000 }] as const satisfies readonly ProgressStage[];

export function describeRemovalProgress(input: {
  kind: "destroy" | "force-cleanup";
  nodeName: string;
  providerName: string;
  startedAt: number;
  now: number;
}): ProgressReport {
  const { kind, nodeName, providerName, startedAt, now } = input;

  return {
    percent: estimateProgress({
      stages: DESTROY_STAGES,
      stageIndex: 0,
      stageElapsedMs: now - startedAt,
      done: false,
    }),
    line:
      kind === "destroy"
        ? `Deleting ${nodeName} from ${providerName}…`
        : `Cleaning up ${nodeName}…`,
  };
}

// ---------------------------------------------------------------------------------------------
// Connect
// ---------------------------------------------------------------------------------------------

/**
 * Typical duration of each connect step. The handshake wait is the measured fifteen seconds from
 * the reported outage; authorizing covers a user reading the UAC prompt.
 */
export const CONNECT_STEP_TYPICAL_MS: Record<ConnectStep, number> = {
  "creating-tunnel": 2_000,
  authorizing: 5_000,
  "starting-service": 2_000,
  "waiting-handshake": 15_000,
  verifying: 2_000,
};

const CONNECT_STAGES: readonly ProgressStage[] = CONNECT_STEPS.map((step) => ({
  typicalMs: CONNECT_STEP_TYPICAL_MS[step],
}));

/**
 * Connect progress. A connect's steps do not expose when each began, so time is measured from the
 * start of the attempt minus the typical length of the steps already passed — close enough for a
 * bar, and it still never runs backwards because the step only advances.
 */
export function estimateConnectProgress(input: {
  step: ConnectStep;
  elapsedMs: number;
  done: boolean;
}): number {
  const stageIndex = CONNECT_STEPS.indexOf(input.step);
  const passedMs = CONNECT_STEPS.slice(0, stageIndex).reduce(
    (sum, step) => sum + CONNECT_STEP_TYPICAL_MS[step],
    0,
  );

  return estimateProgress({
    stages: CONNECT_STAGES,
    stageIndex,
    stageElapsedMs: input.elapsedMs - passedMs,
    done: input.done,
  });
}
