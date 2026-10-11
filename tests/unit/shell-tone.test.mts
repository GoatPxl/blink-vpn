import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { deriveShellTone, type ShellToneInput } from "../../src/lib/shell-tone.ts";

const idle: ShellToneInput = { tunnelPhase: "disconnected", connecting: false, operating: false, failed: false };

describe("deriveShellTone", () => {
  it("is off with nothing running", () => {
    assert.equal(deriveShellTone(idle), "off");
  });

  it("is secure only on traffic evidence", () => {
    assert.equal(deriveShellTone({ ...idle, tunnelPhase: "connected" }), "secure");
    assert.equal(deriveShellTone({ ...idle, tunnelPhase: "degraded" }), "secure");
  });

  it("is busy while a connect is in flight, even if a stale phase says connected", () => {
    assert.equal(deriveShellTone({ ...idle, connecting: true, tunnelPhase: "connected" }), "busy");
  });

  it("is busy while a provision or destroy runs", () => {
    assert.equal(deriveShellTone({ ...idle, operating: true }), "busy");
  });

  it("does not paint a working tunnel red for an unrelated failure", () => {
    assert.equal(deriveShellTone({ ...idle, tunnelPhase: "connected", failed: true }), "secure");
  });

  it("is failed for a stalled tunnel or an unresolved failure", () => {
    assert.equal(deriveShellTone({ ...idle, tunnelPhase: "stalled" }), "failed");
    assert.equal(deriveShellTone({ ...idle, failed: true }), "failed");
  });
});
