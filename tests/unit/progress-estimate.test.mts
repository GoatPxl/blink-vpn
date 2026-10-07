import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  DESTROY_STAGES,
  PROVISION_STAGES,
  UNFINISHED_CAP_PERCENT,
  describeProvisionProgress,
  describeRemovalProgress,
  estimateConnectProgress,
  estimateProgress,
  type ProgressStage,
} from "../../src/lib/progress-estimate.ts";
import { CONNECT_STEPS } from "../../src/lib/connect-progress-display.ts";

const STAGES: ProgressStage[] = [{ typicalMs: 1_000 }, { typicalMs: 3_000 }];

describe("estimateProgress", () => {
  it("is exactly 100 only when done", () => {
    assert.equal(estimateProgress({ stages: STAGES, stageIndex: 1, stageElapsedMs: 0, done: true }), 100);
    assert.equal(
      estimateProgress({ stages: STAGES, stageIndex: 1, stageElapsedMs: 10_000_000, done: false }),
      UNFINISHED_CAP_PERCENT,
    );
  });

  it("starts at zero", () => {
    assert.equal(estimateProgress({ stages: STAGES, stageIndex: 0, stageElapsedMs: 0, done: false }), 0);
  });

  it("counts finished stages at their full share", () => {
    // Stage 0 is a quarter of the total typical time.
    assert.equal(estimateProgress({ stages: STAGES, stageIndex: 1, stageElapsedMs: 0, done: false }), 25);
  });

  it("never moves backwards as time passes or stages advance", () => {
    let previous = -1;
    for (const stageIndex of [0, 1]) {
      for (let elapsed = 0; elapsed <= 20_000; elapsed += 250) {
        const percent = estimateProgress({ stages: STAGES, stageIndex, stageElapsedMs: elapsed, done: false });
        assert.ok(percent >= previous, `dropped from ${previous} to ${percent} at stage ${stageIndex}, ${elapsed}ms`);
        previous = percent;
      }
    }
  });

  it("does not finish a stage on the clock alone", () => {
    // Three times the typical duration still leaves the stage short of its share.
    const longWait = estimateProgress({ stages: STAGES, stageIndex: 0, stageElapsedMs: 3_000, done: false });
    assert.ok(longWait < 25, `stage 0 alone reached ${longWait}%`);
  });
});

describe("describeProvisionProgress", () => {
  const base = { startedAt: 0, bootstrappingSince: null, providerName: "Linode" } as const;

  it("moves from preparing to creating on the clock while the status is provisioning", () => {
    const early = describeProvisionProgress({ ...base, status: "provisioning", now: 1_000 });
    assert.equal(early.line, "Generating keys and server config…");

    const later = describeProvisionProgress({ ...base, status: "provisioning", now: PROVISION_STAGES[0].typicalMs + 1 });
    assert.equal(later.line, "Creating the server on Linode…");
    assert.ok(later.percent > early.percent);
  });

  it("jumps to the boot stage when bootstrapping is observed", () => {
    const creating = describeProvisionProgress({ ...base, status: "provisioning", now: 20_000 });
    const booting = describeProvisionProgress({
      ...base,
      status: "bootstrapping",
      bootstrappingSince: 20_000,
      now: 20_000,
    });
    assert.equal(booting.line, "Waiting for the server to boot and start WireGuard…");
    assert.ok(booting.percent > creating.percent);
  });

  it("reports 100 only once the server is ready", () => {
    const booting = describeProvisionProgress({
      ...base,
      status: "bootstrapping",
      bootstrappingSince: 0,
      now: 10_000_000,
    });
    assert.ok(booting.percent < 100);
    assert.deepEqual(describeProvisionProgress({ ...base, status: "ready", now: 1 }), {
      percent: 100,
      line: "Server ready.",
    });
  });
});

describe("describeRemovalProgress", () => {
  it("names the node and provider for a destroy", () => {
    const report = describeRemovalProgress({
      kind: "destroy",
      nodeName: "Frankfurt",
      providerName: "DigitalOcean",
      startedAt: 0,
      now: DESTROY_STAGES[0].typicalMs,
    });
    assert.equal(report.line, "Deleting Frankfurt from DigitalOcean…");
    assert.ok(report.percent > 50 && report.percent < UNFINISHED_CAP_PERCENT);
  });

  it("uses cleanup wording for a force cleanup", () => {
    const report = describeRemovalProgress({
      kind: "force-cleanup",
      nodeName: "Frankfurt",
      providerName: "DigitalOcean",
      startedAt: 0,
      now: 0,
    });
    assert.equal(report.line, "Cleaning up Frankfurt…");
    assert.equal(report.percent, 0);
  });
});

describe("estimateConnectProgress", () => {
  it("advances with each step and finishes only when done", () => {
    let previous = -1;
    for (const step of CONNECT_STEPS) {
      const percent = estimateConnectProgress({ step, elapsedMs: 0, done: false });
      assert.ok(percent >= previous, `${step} went backwards`);
      previous = percent;
    }
    assert.ok(previous < 100);
    assert.equal(estimateConnectProgress({ step: "verifying", elapsedMs: 0, done: true }), 100);
  });
});
