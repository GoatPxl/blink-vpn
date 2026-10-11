/**
 * Pure presentation logic for the log in Shield.
 *
 * Free of JSX and React so it can be unit tested directly. `now` is always a parameter rather than
 * a `Date.now()` call, which is what keeps these tests deterministic and keeps impure work out of
 * React's render path.
 *
 * The page this feeds answers one question — "what did the app just do, and what do I do about the
 * failure?" — so the grouping and the failure selection are the whole point, not decoration.
 */

import type { ActivityEvent } from "./tauri";

export type ActivityOutcome = "success" | "info" | "error";

/** Which slice of history the user is looking at. */
export type ActivityTab = "all" | "failures" | "provisioning" | "connection";

export type ActivityStep = {
  id: number;
  stage: string;
  outcome: ActivityOutcome;
  detail: string;
  createdAt: string;
};

/**
 * One run of one flow — a provision, a connect, a teardown — with the steps it took.
 *
 * Grouped because a failed provision writes eight rows. Listing them flat buries the one line that
 * says what went wrong among seven that say things went fine until then.
 */
export type ActivityRun = {
  id: string;
  category: string;
  nodeId: string | null;
  providerSlug: string | null;
  outcome: ActivityOutcome;
  /** Newest-first ordering key: the most recent step in the run. */
  latestAt: string;
  startedAt: string;
  steps: ActivityStep[];
};

const CATEGORY_LABEL: Record<string, string> = {
  provision: "Provision",
  connect: "Connect",
  disconnect: "Disconnect",
  destroy: "Destroy",
  cleanup: "Force cleanup",
  app: "Startup check",
};

/** Categories each tab admits. `all` admits everything, so it is absent here. */
const TAB_CATEGORIES: Record<Exclude<ActivityTab, "all" | "failures">, readonly string[]> = {
  provisioning: ["provision", "destroy", "cleanup"],
  connection: ["connect", "disconnect"],
};

export const ACTIVITY_TABS: ReadonlyArray<{ id: ActivityTab; label: string }> = [
  { id: "all", label: "All" },
  { id: "failures", label: "Failures" },
  { id: "provisioning", label: "Servers" },
  { id: "connection", label: "Connection" },
];

function normalizeOutcome(raw: string): ActivityOutcome {
  const value = raw.toLowerCase();
  if (value === "error") return "error";
  if (value === "success") return "success";
  return "info";
}

/**
 * A run is only as good as its worst step.
 *
 * A provision that created a server and then failed to bootstrap has six `success` rows and one
 * `error`; summarising it as anything but failed would be a lie the user acts on.
 */
function worstOutcome(steps: ActivityStep[]): ActivityOutcome {
  if (steps.some((step) => step.outcome === "error")) return "error";
  if (steps.some((step) => step.outcome === "success")) return "success";
  return "info";
}

/**
 * Folds raw events into runs, newest first.
 *
 * Events without a `correlation_id` — a dropped tunnel, a startup finding — each stand alone rather
 * than being lumped together, because they are unrelated occurrences that merely lack a run.
 */
export function groupActivity(events: ActivityEvent[] | null | undefined): ActivityRun[] {
  if (!events || events.length === 0) return [];

  const runs = new Map<string, ActivityRun>();

  for (const event of events) {
    const key = event.correlation_id ?? `event-${event.id}`;
    const step: ActivityStep = {
      id: event.id,
      stage: event.stage,
      outcome: normalizeOutcome(event.outcome),
      detail: event.detail,
      createdAt: event.created_at,
    };

    const existing = runs.get(key);
    if (existing) {
      existing.steps.push(step);
      continue;
    }

    runs.set(key, {
      id: key,
      category: event.category,
      nodeId: event.node_id,
      providerSlug: event.provider_slug,
      outcome: step.outcome,
      latestAt: event.created_at,
      startedAt: event.created_at,
      steps: [step],
    });
  }

  const grouped = [...runs.values()].map((run) => {
    // The backend returns newest first; a run reads forwards, from what it tried to how it ended.
    const steps = [...run.steps].sort((left, right) => left.createdAt.localeCompare(right.createdAt));
    return {
      ...run,
      steps,
      outcome: worstOutcome(steps),
      startedAt: steps[0]?.createdAt ?? run.startedAt,
      latestAt: steps[steps.length - 1]?.createdAt ?? run.latestAt,
    };
  });

  return grouped.sort((left, right) => right.latestAt.localeCompare(left.latestAt));
}

export function filterActivity(runs: ActivityRun[], tab: ActivityTab): ActivityRun[] {
  if (tab === "all") return runs;
  if (tab === "failures") return runs.filter((run) => run.outcome === "error");
  return runs.filter((run) => TAB_CATEGORIES[tab].includes(run.category));
}

/**
 * The failure the user can still do something about.
 *
 * Deliberately the most recent *failure*, not the most recent event: a user who provisions, fails,
 * and then connects to an older node still needs the failure explained.
 *
 * But a failure stays in history forever, while the banner is a call to action — so it has to stop
 * asking once the situation is dealt with. A failure is settled when either:
 *
 * - the node it names no longer exists, so there is nothing left to act on; or
 * - a later run on that same node succeeded.
 *
 * Without this the banner kept offering Force Cleanup for a node that had already been cleaned up.
 * Each click was a no-op, nothing on screen changed, and so it invited being clicked again — which
 * is exactly what happened, seven times in thirty-seven seconds.
 */
export function selectUnresolvedFailure(
  runs: ActivityRun[],
  existingNodeIds: ReadonlySet<string>,
): ActivityRun | null {
  for (const run of runs) {
    if (run.outcome !== "error") continue;

    // A run against a node that is gone describes something already resolved by deletion.
    if (run.nodeId !== null && !existingNodeIds.has(run.nodeId)) continue;

    const settledLater = runs.some(
      (other) =>
        other.outcome === "success"
        && other.nodeId === run.nodeId
        && other.latestAt > run.latestAt,
    );

    if (!settledLater) return run;
  }

  return null;
}

/** The step that actually failed, which is the one carrying the cause. */
export function failingStep(run: ActivityRun): ActivityStep | null {
  return [...run.steps].reverse().find((step) => step.outcome === "error") ?? null;
}

function describeCategory(category: string): string {
  return CATEGORY_LABEL[category] ?? category;
}

/**
 * How long ago, in the coarsest unit that is still honest.
 *
 * Returns null for an unparseable timestamp rather than "just now", which would date a record the
 * app cannot actually place in time.
 */
export function formatRelativeTime(timestamp: string, now: number): string | null {
  const parsed = Date.parse(timestamp);
  if (Number.isNaN(parsed)) return null;

  const seconds = Math.max(0, Math.floor((now - parsed) / 1000));
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

/** Run header line: what it was, how it ended, when. */
export function describeRunHeadline(run: ActivityRun, now: number): string {
  const outcome =
    run.outcome === "error" ? "failed" : run.outcome === "success" ? "completed" : "in progress";
  const when = formatRelativeTime(run.latestAt, now);
  const target = run.nodeId ? ` · ${run.nodeId}` : "";
  return `${describeCategory(run.category)}${target} · ${outcome}${when ? ` · ${when}` : ""}`;
}

/** Clock time of a step, for the aligned left column. Empty when unparseable. */
export function formatStepTime(timestamp: string): string {
  const parsed = Date.parse(timestamp);
  if (Number.isNaN(parsed)) return "";
  return new Date(parsed).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/**
 * What the user should do about a failure, keyed off the run's own category.
 *
 * A provision that failed usually leaves a node stuck in `error` that Force Cleanup clears; a
 * connect that failed just needs retrying. Suggesting cleanup for a failed connect would send the
 * user to destroy a working server.
 */
export function describeRecovery(run: ActivityRun): { action: "cleanup" | "retry-connect" | "none"; label: string } {
  if (run.category === "provision") {
    return { action: "cleanup", label: "Run Force Cleanup on this node, then try creating it again." };
  }
  if (run.category === "connect") {
    return { action: "retry-connect", label: "Try connecting again. Approve the elevation prompt if one appears." };
  }
  if (run.category === "cleanup" || run.category === "destroy") {
    return {
      action: "none",
      label: "Check your provider's console — the server may still exist and still be billing.",
    };
  }
  return { action: "none", label: "No automatic recovery for this one." };
}
