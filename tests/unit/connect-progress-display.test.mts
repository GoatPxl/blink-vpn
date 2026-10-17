import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  awaitingWindowsPermission,
  deriveConnectStep,
  describeConnectProgress,
  formatElapsedMs,
  outageElapsedSeconds,
  serviceHasStarted,
} from "../../src/lib/connect-progress-display.ts";
import type { TunnelPhaseSnapshot } from "../../src/lib/tauri.ts";

function snapshot(overrides: Partial<TunnelPhaseSnapshot> = {}): TunnelPhaseSnapshot {
  return {
    tunnel_id: "tunnel-1",
    tunnel_name: "Primary-Node-wg",
    phase: "connecting",
    service_state: "running",
    elapsed_seconds: 3,
    rx_bytes: 0,
    tx_bytes: 512,
    ...overrides,
  };
}

describe("describeConnectProgress", () => {
  // The whole point of the card. The outage was already happening; what was missing was the app
  // saying so. Asserting the copy is non-empty would pass for any string, including the old
  // silence — so this asserts the specific promise made to the user.
  it("warns that the internet will be unavailable while connecting", () => {
    const copy = describeConnectProgress({ phase: "connecting", elapsedSeconds: 4 });

    assert.equal(copy.tone, "info");
    assert.match(copy.detail, /internet will be unavailable/i);
    assert.match(copy.detail, /expected/i);
  });

  it("gives the same warning before the first snapshot arrives", () => {
    // The gap between clicking Connect and the first poll includes the UAC prompt, and the tunnel
    // may already be up and routing by then. Saying nothing here reproduces the original bug.
    const copy = describeConnectProgress({ phase: null, elapsedSeconds: 0 });

    assert.match(copy.detail, /internet will be unavailable/i);
  });

  it("does not offer an escape hatch while a connect looks healthy", () => {
    for (const phase of [null, "connecting", "disconnected"] as const) {
      const copy = describeConnectProgress({ phase, elapsedSeconds: 5 });
      assert.equal(copy.showDisconnectEscape, false, `phase=${phase}`);
    }
  });

  it("offers an escape hatch once the connect has stalled, naming the real elapsed time", () => {
    const copy = describeConnectProgress({ phase: "stalled", elapsedSeconds: 27 });

    assert.equal(copy.showDisconnectEscape, true);
    assert.equal(copy.tone, "warning");
    assert.match(copy.detail, /27 seconds/);
  });

  it("reports success only for phases that prove traffic crossed the tunnel", () => {
    for (const phase of ["connected", "degraded"] as const) {
      const copy = describeConnectProgress({ phase, elapsedSeconds: 12 });
      assert.equal(copy.tone, "positive", `phase=${phase}`);
      assert.equal(copy.headline, "Connected");
    }
  });

  // A running service is not a working tunnel. If `connecting` ever reported positively, the card
  // would be repeating the exact claim this change exists to remove.
  it("never reports a connecting tunnel as positive", () => {
    const copy = describeConnectProgress({ phase: "connecting", elapsedSeconds: 8 });
    assert.notEqual(copy.tone, "positive");
    assert.notEqual(copy.headline, "Connected");
  });
});

describe("deriveConnectStep", () => {
  it("reports tunnel creation before any service exists", () => {
    const step = deriveConnectStep({ creatingTunnel: true, snapshot: null });
    assert.equal(step, "creating-tunnel");
  });

  it("reports authorizing while waiting for the first snapshot", () => {
    // This is the UAC window — the app has asked for elevation and is waiting on the user.
    const step = deriveConnectStep({ creatingTunnel: false, snapshot: null });
    assert.equal(step, "authorizing");
  });

  it("reports starting-service while Windows brings the service up", () => {
    const step = deriveConnectStep({
      creatingTunnel: false,
      snapshot: snapshot({ phase: "connecting", service_state: "starting" }),
    });
    assert.equal(step, "starting-service");
  });

  it("reports waiting-handshake for a running service that has carried nothing", () => {
    const step = deriveConnectStep({
      creatingTunnel: false,
      snapshot: snapshot({ phase: "connecting", service_state: "running", rx_bytes: 0 }),
    });
    assert.equal(step, "waiting-handshake");
  });

  it("keeps a stalled connect on the handshake step rather than advancing it", () => {
    const step = deriveConnectStep({
      creatingTunnel: false,
      snapshot: snapshot({ phase: "stalled", service_state: "running" }),
    });
    assert.equal(step, "waiting-handshake");
  });

  it("advances to verifying once traffic has crossed the tunnel", () => {
    const step = deriveConnectStep({
      creatingTunnel: false,
      snapshot: snapshot({ phase: "connected", rx_bytes: 4096 }),
    });
    assert.equal(step, "verifying");
  });
});

describe("formatElapsedMs", () => {
  it("formats as M:SS with a zero-padded seconds field", () => {
    const start = 1_000_000;
    assert.equal(formatElapsedMs(start, start), "0:00");
    assert.equal(formatElapsedMs(start, start + 7_000), "0:07");
    assert.equal(formatElapsedMs(start, start + 65_000), "1:05");
    assert.equal(formatElapsedMs(start, start + 600_000), "10:00");
  });

  it("clamps a clock that moved backwards rather than rendering a negative timer", () => {
    const start = 1_000_000;
    assert.equal(formatElapsedMs(start, start - 5_000), "0:00");
  });
});

describe("waiting for Windows permission", () => {
  it("holds until the service is seen to start", () => {
    assert.equal(awaitingWindowsPermission({ serviceStartedAt: null }), true);
    assert.equal(awaitingWindowsPermission({ serviceStartedAt: 1_000 }), false);
  });

  // Before the prompt is approved there is no service, which reads as stopped and disconnected.
  it("does not treat a missing service as started", () => {
    assert.equal(serviceHasStarted(snapshot({ phase: "disconnected", service_state: "stopped" })), false);
    assert.equal(serviceHasStarted(snapshot({ phase: "disconnected", service_state: "unknown" })), false);
  });

  it("treats a starting or running service as started", () => {
    assert.equal(serviceHasStarted(snapshot({ phase: "disconnected", service_state: "starting" })), true);
    assert.equal(serviceHasStarted(snapshot({ phase: "connecting", service_state: "running" })), true);
    assert.equal(serviceHasStarted(snapshot({ phase: "stalled", service_state: "running" })), true);
  });

  // The reported bug: an unanswered prompt counted past seven minutes of "Internet pauses for ~15 s".
  it("counts the outage from the service starting, not from the click", () => {
    const clickedAt = 0;
    const approvedAt = 7 * 60 * 1000;
    assert.equal(outageElapsedSeconds({ serviceStartedAt: null }, approvedAt), 0);
    assert.equal(outageElapsedSeconds({ serviceStartedAt: approvedAt }, approvedAt + 12_400), 12);
    assert.notEqual(outageElapsedSeconds({ serviceStartedAt: approvedAt }, approvedAt + 12_400), Math.floor((approvedAt + 12_400 - clickedAt) / 1000));
  });
});
