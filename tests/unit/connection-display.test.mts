import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { describeEgress, describeIpv6, formatElapsed } from "../../src/lib/connection-display.ts";

const START = "2026-08-15T10:00:00.000Z";
const startMs = Date.parse(START);

describe("formatElapsed", () => {
  it("counts up from the session start", () => {
    assert.equal(formatElapsed(START, startMs), "00:00:00");
    assert.equal(formatElapsed(START, startMs + 45_000), "00:00:45");
    assert.equal(formatElapsed(START, startMs + 3_661_000), "01:01:01");
  });

  it("prefixes days past twenty-four hours", () => {
    assert.equal(formatElapsed(START, startMs + 90_061_000), "1d 01:01:01");
  });

  // A missing start is not a zero-length session. Rendering 00:00:00 would assert a start time
  // the app does not have, for a tunnel that may have been up for hours.
  it("reports an unknown start as null rather than zero", () => {
    assert.equal(formatElapsed(null, startMs), null);
    assert.equal(formatElapsed(undefined, startMs), null);
    assert.equal(formatElapsed("", startMs), null);
    assert.equal(formatElapsed("not a timestamp", startMs), null);
  });

  // Clock adjustments and SQLite/JS rounding can put "now" fractionally behind the stored start.
  it("reports a start in the future as unknown, never as a negative clock", () => {
    assert.equal(formatElapsed(START, startMs - 5_000), null);
  });
});

describe("describeEgress", () => {
  const SERVER = "203.0.113.10";
  const HOME = "198.51.100.7";

  it("reports protected when the observed address matches the server", () => {
    const result = describeEgress({ tunnelConnected: true, observedEgress: SERVER, serverIp: SERVER });
    assert.equal(result.tone, "positive");
    assert.equal(result.value, SERVER);
    assert.match(result.caption, /protected/i);
  });

  it("warns when traffic is leaving by some route other than the tunnel", () => {
    const result = describeEgress({ tunnelConnected: true, observedEgress: HOME, serverIp: SERVER });
    assert.equal(result.tone, "warning");
    assert.match(result.caption, /not exiting through your server/i);
  });

  // The point of the card: the address is shown in both states so the change on connect is
  // visible. Showing the server's address while disconnected would hide exactly that.
  it("shows the user's real address while disconnected", () => {
    const result = describeEgress({ tunnelConnected: false, observedEgress: HOME, serverIp: SERVER });
    assert.equal(result.value, HOME);
    assert.equal(result.tone, "muted");
    assert.match(result.caption, /not protected/i);
  });

  // The bug this guards: substituting the server's own address for a failed probe made this
  // state indistinguishable from a verified one, so the UI claimed a check it had not made.
  it("does not claim protection when the probe failed", () => {
    const result = describeEgress({ tunnelConnected: true, observedEgress: null, serverIp: SERVER });
    assert.equal(result.tone, "muted");
    assert.equal(result.value, "n/a");
    assert.doesNotMatch(result.caption, /protected/i);
    assert.notEqual(result.value, SERVER);
  });
});

describe("describeIpv6", () => {
  // The tunnel is IPv4-only, so reachable IPv6 while connected is traffic leaving outside it.
  it("flags a reachable IPv6 address while connected as a bypass", () => {
    const result = describeIpv6({ tunnelConnected: true, observedIpv6: "2001:db8::1" });
    assert.equal(result?.tone, "warning");
    assert.match(result!.caption, /bypassing/i);
  });

  it("confirms no leak when connected with no IPv6 egress", () => {
    const result = describeIpv6({ tunnelConnected: true, observedIpv6: null });
    assert.equal(result?.tone, "positive");
    assert.match(result!.caption, /no ipv6 leak/i);
  });

  // Disconnected, IPv6 is just the user's other address — nothing is wrong with it.
  it("does not warn about IPv6 while disconnected", () => {
    const result = describeIpv6({ tunnelConnected: false, observedIpv6: "2001:db8::1" });
    assert.equal(result?.tone, "muted");
    assert.doesNotMatch(result!.caption, /bypass|leak/i);
  });

  it("reports nothing when disconnected without IPv6, so the row disappears", () => {
    assert.equal(describeIpv6({ tunnelConnected: false, observedIpv6: null }), null);
  });
});

