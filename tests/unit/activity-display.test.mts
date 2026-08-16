import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  describeRecovery,
  describeRunHeadline,
  failingStep,
  filterActivity,
  formatRelativeTime,
  groupActivity,
  selectUnresolvedFailure,
} from "../../src/lib/activity-display.ts";
import type { ActivityEvent } from "../../src/lib/tauri.ts";

let nextId = 1;

function event(overrides: Partial<ActivityEvent> = {}): ActivityEvent {
  return {
    id: nextId++,
    node_id: "node-1",
    provider_slug: "digitalocean",
    category: "provision",
    stage: "create.start",
    outcome: "info",
    correlation_id: "run-1",
    detail: "init flow started",
    created_at: "2026-08-16T10:00:00.000Z",
    ...overrides,
  };
}

/** The backend returns newest first; these fixtures mirror that. */
function provisionRunThatFailed(): ActivityEvent[] {
  return [
    event({
      stage: "create.error",
      outcome: "error",
      detail: "Node creation failed <- name must be a valid hostname",
      created_at: "2026-08-16T10:00:09.000Z",
    }),
    event({ stage: "provider.create", outcome: "success", detail: "droplet created", created_at: "2026-08-16T10:00:05.000Z" }),
    event({ stage: "create.start", outcome: "info", detail: "init flow started", created_at: "2026-08-16T10:00:00.000Z" }),
  ];
}

describe("groupActivity", () => {
  it("returns nothing before any events arrive", () => {
    assert.deepEqual(groupActivity(null), []);
    assert.deepEqual(groupActivity([]), []);
  });

  it("folds the stages sharing a correlation id into one run", () => {
    const runs = groupActivity(provisionRunThatFailed());
    assert.equal(runs.length, 1);
    assert.equal(runs[0].steps.length, 3);
  });

  /**
   * A failed provision writes six successful rows and one error. Summarising it by its last-written
   * or first-written step would call a failure a success.
   */
  it("takes a run's outcome from its worst step", () => {
    const runs = groupActivity(provisionRunThatFailed());
    assert.equal(runs[0].outcome, "error");
  });

  it("orders steps oldest first so a run reads forwards", () => {
    const [run] = groupActivity(provisionRunThatFailed());
    assert.equal(run.steps[0].stage, "create.start");
    assert.equal(run.steps[2].stage, "create.error");
    assert.equal(run.startedAt, "2026-08-16T10:00:00.000Z");
    assert.equal(run.latestAt, "2026-08-16T10:00:09.000Z");
  });

  it("orders runs newest first", () => {
    const runs = groupActivity([
      event({ correlation_id: "run-new", category: "connect", stage: "connect.ready", outcome: "success", created_at: "2026-08-16T12:00:00.000Z" }),
      event({ correlation_id: "run-old", category: "connect", stage: "connect.ready", outcome: "success", created_at: "2026-08-16T09:00:00.000Z" }),
    ]);

    assert.deepEqual(runs.map((run) => run.id), ["run-new", "run-old"]);
  });

  /**
   * A dropped tunnel and a startup finding have no run to belong to. Bucketing them together by
   * their shared null would merge unrelated occurrences into one entry.
   */
  it("keeps events without a correlation id separate from each other", () => {
    const runs = groupActivity([
      event({ correlation_id: null, category: "connect", stage: "connect.dropped", outcome: "error", detail: "first drop" }),
      event({ correlation_id: null, category: "connect", stage: "connect.dropped", outcome: "error", detail: "second drop" }),
    ]);

    assert.equal(runs.length, 2);
    assert.ok(runs.every((run) => run.steps.length === 1));
  });
});

describe("filterActivity", () => {
  const runs = groupActivity([
    event({ correlation_id: "p", category: "provision", outcome: "error", stage: "create.error", created_at: "2026-08-16T10:00:00.000Z" }),
    event({ correlation_id: "c", category: "connect", outcome: "success", stage: "connect.ready", created_at: "2026-08-16T11:00:00.000Z" }),
    event({ correlation_id: "d", category: "disconnect", outcome: "success", stage: "disconnect.completed", created_at: "2026-08-16T12:00:00.000Z" }),
    event({ correlation_id: "k", category: "cleanup", outcome: "success", stage: "cleanup.completed", created_at: "2026-08-16T13:00:00.000Z" }),
  ]);

  it("passes everything through on the All tab", () => {
    assert.equal(filterActivity(runs, "all").length, 4);
  });

  it("shows only failures on the Failures tab", () => {
    const failures = filterActivity(runs, "failures");
    assert.equal(failures.length, 1);
    assert.equal(failures[0].category, "provision");
  });

  it("groups server lifecycle categories together", () => {
    const servers = filterActivity(runs, "provisioning").map((run) => run.category);
    assert.deepEqual(servers.sort(), ["cleanup", "provision"]);
  });

  it("groups connect and disconnect together", () => {
    const connection = filterActivity(runs, "connection").map((run) => run.category);
    assert.deepEqual(connection.sort(), ["connect", "disconnect"]);
  });
});

describe("selectUnresolvedFailure", () => {
  const NODES = new Set(["node-1", "node-2"]);

  /**
   * Deliberately the most recent failure, not the most recent event: a user who fails to provision
   * and then successfully connects to a *different* server still needs the failure explained.
   */
  it("finds the newest failure even when newer successes on other nodes exist", () => {
    const runs = groupActivity([
      event({ node_id: "node-2", correlation_id: "later-success", category: "connect", outcome: "success", stage: "connect.ready", created_at: "2026-08-16T12:00:00.000Z" }),
      event({ node_id: "node-1", correlation_id: "the-failure", category: "provision", outcome: "error", stage: "create.error", detail: "bad name", created_at: "2026-08-16T11:00:00.000Z" }),
      event({ node_id: "node-1", correlation_id: "older-failure", category: "provision", outcome: "error", stage: "create.error", detail: "older", created_at: "2026-08-16T09:00:00.000Z" }),
    ]);

    assert.equal(selectUnresolvedFailure(runs, NODES)?.id, "the-failure");
  });

  it("returns null when nothing has failed", () => {
    const runs = groupActivity([
      event({ correlation_id: "ok", category: "connect", outcome: "success", stage: "connect.ready" }),
    ]);
    assert.equal(selectUnresolvedFailure(runs, NODES), null);
  });

  /**
   * The banner is a call to action, and a node that no longer exists offers none.
   *
   * Left unhandled, this kept offering Force Cleanup for an already-deleted node: the click was a
   * no-op, the screen did not change, so it invited being clicked again — seven times in the real
   * report that produced this test.
   */
  it("drops a failure whose node no longer exists", () => {
    const runs = groupActivity([
      event({ node_id: "node-gone", correlation_id: "f", category: "provision", outcome: "error", stage: "create.error" }),
    ]);

    assert.equal(selectUnresolvedFailure(runs, NODES), null);
  });

  it("drops a failure that a later run on the same node put right", () => {
    const runs = groupActivity([
      event({ node_id: "node-1", correlation_id: "recovered", category: "cleanup", outcome: "success", stage: "cleanup.completed", created_at: "2026-08-16T12:00:00.000Z" }),
      event({ node_id: "node-1", correlation_id: "failed", category: "provision", outcome: "error", stage: "create.error", created_at: "2026-08-16T11:00:00.000Z" }),
    ]);

    assert.equal(selectUnresolvedFailure(runs, NODES), null);
  });

  // A success *before* the failure settles nothing; order is the whole point.
  it("keeps a failure that only has successes older than it", () => {
    const runs = groupActivity([
      event({ node_id: "node-1", correlation_id: "failed", category: "connect", outcome: "error", stage: "connect.error", created_at: "2026-08-16T12:00:00.000Z" }),
      event({ node_id: "node-1", correlation_id: "earlier-ok", category: "connect", outcome: "success", stage: "connect.ready", created_at: "2026-08-16T11:00:00.000Z" }),
    ]);

    assert.equal(selectUnresolvedFailure(runs, NODES)?.id, "failed");
  });

  it("keeps an app-level failure, which has no node to check", () => {
    const runs = groupActivity([
      event({ node_id: null, correlation_id: null, category: "app", outcome: "error", stage: "startup.reconcile", detail: "droplet missing" }),
    ]);

    assert.equal(selectUnresolvedFailure(runs, NODES)?.steps[0].stage, "startup.reconcile");
  });
});

describe("failingStep", () => {
  // The banner must show the cause, which lives on the step that failed — not on the run's first
  // or last step, both of which may have succeeded.
  it("picks the step carrying the cause, not the first or last step", () => {
    const [run] = groupActivity(provisionRunThatFailed());
    const step = failingStep(run);
    assert.equal(step?.stage, "create.error");
    assert.match(step?.detail ?? "", /must be a valid hostname/);
  });
});

describe("describeRecovery", () => {
  /**
   * Suggesting Force Cleanup for a failed connect would send the user to tear down a server that
   * works. The recovery has to follow the category.
   */
  it("offers cleanup for a failed provision and a retry for a failed connect", () => {
    const [provision] = groupActivity([event({ correlation_id: "p", category: "provision", outcome: "error", stage: "create.error" })]);
    const [connect] = groupActivity([event({ correlation_id: "c", category: "connect", outcome: "error", stage: "connect.error" })]);

    assert.equal(describeRecovery(provision).action, "cleanup");
    assert.equal(describeRecovery(connect).action, "retry-connect");
  });

  // A failed destroy may have left a billable server behind; there is no button that fixes that.
  it("points a failed teardown at the provider console rather than offering an action", () => {
    const [destroy] = groupActivity([event({ correlation_id: "d", category: "destroy", outcome: "error", stage: "cleanup.completed" })]);
    const recovery = describeRecovery(destroy);
    assert.equal(recovery.action, "none");
    assert.match(recovery.label, /billing/i);
  });
});

describe("formatRelativeTime", () => {
  const now = Date.parse("2026-08-16T12:00:00.000Z");

  it("scales from seconds to days", () => {
    assert.equal(formatRelativeTime("2026-08-16T11:59:30.000Z", now), "just now");
    assert.equal(formatRelativeTime("2026-08-16T11:55:00.000Z", now), "5m ago");
    assert.equal(formatRelativeTime("2026-08-16T09:00:00.000Z", now), "3h ago");
    assert.equal(formatRelativeTime("2026-08-14T12:00:00.000Z", now), "2d ago");
  });

  // Returning "just now" for a timestamp we cannot read would date a record the app cannot place.
  it("returns null rather than guessing at an unreadable timestamp", () => {
    assert.equal(formatRelativeTime("not-a-date", now), null);
  });
});

describe("describeRunHeadline", () => {
  const now = Date.parse("2026-08-16T10:05:00.000Z");

  it("names what ran, how it ended and when", () => {
    const [run] = groupActivity(provisionRunThatFailed());
    const headline = describeRunHeadline(run, now);
    assert.match(headline, /Provision/);
    assert.match(headline, /node-1/);
    assert.match(headline, /failed/);
  });

  it("says completed for a successful run", () => {
    const [run] = groupActivity([
      event({ correlation_id: "c", category: "connect", outcome: "success", stage: "connect.ready" }),
    ]);
    assert.match(describeRunHeadline(run, now), /Connect.*completed/);
  });
});
