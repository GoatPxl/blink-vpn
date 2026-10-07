/**
 * Pure presentation logic for a connect in flight.
 *
 * Kept free of JSX and React so it can be unit tested directly, following `node-display.ts` and
 * `connection-display.ts`. The copy lives here rather than in the component because *what the user
 * is told during the outage is the feature* — it needs to be assertable in a test, not buried in
 * markup.
 *
 * The problem this exists to solve: the WireGuard adapter captures the default route the moment its
 * service starts, so the user's internet drops immediately and stays down until the first handshake
 * completes. That gap was fifteen seconds in the reported case, during which the app said
 * "Connected" and explained nothing.
 */

import type { TunnelPhase, TunnelPhaseSnapshot } from "./tauri";

/** The stages of a connect, in the order they occur. */
export type ConnectStep =
  /** No tunnel record yet, so one is being created and its keys generated. */
  | "creating-tunnel"
  /** Registering the tunnel service — the step that raises the UAC prompt. */
  | "authorizing"
  /** The service exists and Windows is starting it. */
  | "starting-service"
  /** Service up, waiting for the server to answer. This is the window with no internet. */
  | "waiting-handshake"
  /** Traffic confirmed; measuring the egress address to prove it exits through the server. */
  | "verifying";

export const CONNECT_STEPS: ConnectStep[] = [
  "creating-tunnel",
  "authorizing",
  "starting-service",
  "waiting-handshake",
  "verifying",
];

export const CONNECT_STEP_LABEL: Record<ConnectStep, string> = {
  "creating-tunnel": "Preparing tunnel",
  authorizing: "Authorizing",
  "starting-service": "Starting adapter",
  "waiting-handshake": "Contacting server",
  verifying: "Verifying traffic",
};

/**
 * Which step a connect is currently on.
 *
 * Derived from the latest snapshot rather than stored, so it cannot disagree with the phase the
 * indicator uses. Before the first snapshot arrives the app is either creating the tunnel record or
 * waiting on the UAC prompt — neither produces a service to read, which is exactly why the step is
 * inferred from the snapshot's absence.
 */
export function deriveConnectStep(input: {
  creatingTunnel: boolean;
  snapshot: TunnelPhaseSnapshot | null;
}): ConnectStep {
  const { creatingTunnel, snapshot } = input;

  if (creatingTunnel) return "creating-tunnel";
  if (snapshot === null) return "authorizing";

  switch (snapshot.phase) {
    case "connected":
    case "degraded":
      return "verifying";
    case "disconnected":
      // The service is not up yet: either still being registered, or Windows has not started it.
      return snapshot.service_state === "starting" ? "starting-service" : "authorizing";
    case "connecting":
    case "stalled":
      return snapshot.service_state === "starting" ? "starting-service" : "waiting-handshake";
  }
}

export type ConnectTone = "info" | "warning" | "positive";

export type ConnectProgressCopy = {
  headline: string;
  detail: string;
  tone: ConnectTone;
  /** Whether to offer a way out. Only worth showing once waiting has stopped looking normal. */
  showDisconnectEscape: boolean;
};

/**
 * Roughly how long a healthy connect takes, for setting expectations.
 *
 * Copy only — the decision that a connect has stalled is the backend's, via `STALL_AFTER_SECONDS`.
 * Duplicating that threshold here is what would let the two drift.
 */
const TYPICAL_CONNECT_SECONDS = 15;

/**
 * What to tell the user about a connect in progress.
 *
 * The `connecting` copy states plainly that the internet is down and that this is expected. That
 * sentence is the actual fix for the reported problem: the outage was already happening, the app
 * just never mentioned it.
 */
export function describeConnectProgress(input: {
  phase: TunnelPhase | null;
  elapsedSeconds: number;
}): ConnectProgressCopy {
  const { phase, elapsedSeconds } = input;

  if (phase === "connected" || phase === "degraded") {
    return {
      headline: "Connected",
      detail: "Your traffic now exits through your server.",
      tone: "positive",
      showDisconnectEscape: false,
    };
  }

  if (phase === "stalled") {
    return {
      headline: "Still waiting for your server",
      detail:
        `Your server has not answered after ${elapsedSeconds} seconds. Your internet stays ` +
        "unavailable until it does, because all traffic is already routed through the tunnel. " +
        "You can disconnect to restore your normal connection and try again.",
      tone: "warning",
      showDisconnectEscape: true,
    };
  }

  // Covers `connecting`, `disconnected` (service not up yet) and the pre-first-snapshot null.
  return {
    headline: "Securing your connection",
    detail:
      "Your internet will be unavailable for a few seconds while the tunnel completes its " +
      "handshake. This is expected: all traffic is already routed through the tunnel and your " +
      `server has not answered yet. This usually takes about ${TYPICAL_CONNECT_SECONDS} seconds — ` +
      "please do not disconnect.",
    tone: "info",
    showDisconnectEscape: false,
  };
}

/**
 * Elapsed time as `M:SS`, from an epoch-milliseconds start.
 *
 * Separate from `formatElapsed` in `connection-display.ts`, which parses an ISO timestamp from the
 * database. A connect attempt starts in the browser at click time and has no such timestamp.
 */
export function formatElapsedMs(startedAt: number, now: number): string {
  const totalSeconds = Math.max(0, Math.floor((now - startedAt) / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;

  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}
