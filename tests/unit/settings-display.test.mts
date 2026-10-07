import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildRowActions,
  countAttachedNodes,
  deriveProviderKeyStatus,
  describeKeepalive,
  describeKeyState,
  validateDnsServer,
  validateKeepalive,
  validateMobileDevices,
  MAX_DEVICE_NAME_CHARS,
  MAX_MOBILE_DEVICES,
} from "../../src/lib/settings-display.ts";
import type { ProviderRecord, VpnNodeRecord } from "../../src/lib/tauri.ts";

function provider(overrides: Partial<ProviderRecord> = {}): ProviderRecord {
  return {
    id: "provider-1",
    name: "DigitalOcean",
    slug: "digitalocean",
    status: "active",
    token_ref: "do-token__provider-1__abc",
    last_validation_status: null,
    last_validated_at: null,
    created_at: "2026-08-16T00:00:00.000Z",
    updated_at: "2026-08-16T00:00:00.000Z",
    ...overrides,
  } as ProviderRecord;
}

function node(providerId: string, id: string): VpnNodeRecord {
  return { id, provider_id: providerId } as VpnNodeRecord;
}

describe("deriveProviderKeyStatus", () => {
  // The bug this replaces: the old table read only `token_ref` and `status`, so a key the provider
  // had rejected still rendered "Installed / Active".
  it("reports a rejected key as invalid", () => {
    const record = provider({ last_validation_status: "failed" });
    assert.equal(deriveProviderKeyStatus({ provider: record, hasKey: true }), "invalid");
  });

  it("reports a validated key as verified", () => {
    const record = provider({ last_validation_status: "ok" });
    assert.equal(deriveProviderKeyStatus({ provider: record, hasKey: true }), "verified");
  });

  it("reports a stored but unchecked key as unverified", () => {
    assert.equal(deriveProviderKeyStatus({ provider: provider(), hasKey: true }), "unverified");
  });

  it("reports a missing key regardless of any past verdict", () => {
    const record = provider({ token_ref: null, last_validation_status: "ok" });
    assert.equal(deriveProviderKeyStatus({ provider: record, hasKey: false }), "missing");
  });

  // Suspension is the more actionable fact: operations will refuse whatever the key's state.
  it("lets suspension outrank verification", () => {
    const record = provider({ status: "suspended", last_validation_status: "ok" });
    assert.equal(deriveProviderKeyStatus({ provider: record, hasKey: true }), "suspended");
  });

  /**
   * Each provider carries its own verdict on its own record.
   *
   * With one shared value, checking Docker moved DigitalOcean's "Verified" badge onto Docker and
   * back again on the next check — the badge appeared to toggle between rows rather than describe
   * either of them.
   */
  it("keeps each provider's result independent", () => {
    const digitalOcean = provider({ id: "provider-1", last_validation_status: "ok" });
    const docker = provider({ id: "provider-docker", slug: "docker", last_validation_status: "failed" });

    assert.equal(deriveProviderKeyStatus({ provider: digitalOcean, hasKey: true }), "verified");
    assert.equal(deriveProviderKeyStatus({ provider: docker, hasKey: true }), "invalid");

    // Checking Docker changes only Docker's record.
    const dockerChecked = provider({ id: "provider-docker", slug: "docker", last_validation_status: "ok" });
    assert.equal(deriveProviderKeyStatus({ provider: dockerChecked, hasKey: true }), "verified");
    assert.equal(deriveProviderKeyStatus({ provider: digitalOcean, hasKey: true }), "verified");
  });
});

describe("describeKeyState", () => {
  const CHECKED_AT = "2026-08-16T10:00:00.000Z";
  const now = Date.parse(CHECKED_AT);

  it("says when a key was last checked", () => {
    const record = provider({ last_validation_status: "ok", last_validated_at: CHECKED_AT });
    assert.equal(describeKeyState(record, now + 30_000), "Checked just now");
    assert.equal(describeKeyState(record, now + 5 * 60_000), "Checked 5m ago");
    assert.equal(describeKeyState(record, now + 3 * 3_600_000), "Checked 3h ago");
    assert.equal(describeKeyState(record, now + 2 * 86_400_000), "Checked 2d ago");
  });

  // A verdict with no age would keep asserting a check that may be weeks old and since revoked.
  it("distinguishes a stored key that has never been checked", () => {
    const record = provider({ last_validation_status: null, last_validated_at: null });
    assert.match(describeKeyState(record, now), /not checked yet/i);
  });

  it("reports no key at all", () => {
    assert.equal(describeKeyState(provider({ token_ref: null }), now), "No key stored");
  });
});

describe("buildRowActions", () => {
  it("blocks removing a key while nodes depend on it, and says why", () => {
    const actions = buildRowActions({
      provider: provider(),
      hasKey: true,
      attachedNodeCount: 1,
      busy: false,
    });

    const remove = actions.find((action) => action.id === "delete");
    assert.equal(remove?.disabled, true);
    assert.match(remove!.disabledReason!, /destroy the 1 attached node/i);
  });

  it("allows removing a key once nothing depends on it", () => {
    const actions = buildRowActions({
      provider: provider(),
      hasKey: true,
      attachedNodeCount: 0,
      busy: false,
    });

    const remove = actions.find((action) => action.id === "delete");
    assert.equal(remove?.disabled, false);
    assert.equal(remove?.disabledReason, null);
  });

  it("pluralises the blocked reason", () => {
    const actions = buildRowActions({ provider: provider(), hasKey: true, attachedNodeCount: 3, busy: false });
    assert.match(actions.find((action) => action.id === "delete")!.disabledReason!, /3 attached nodes/);
  });

  it("has nothing to remove when no key is stored", () => {
    const actions = buildRowActions({ provider: provider({ token_ref: null }), hasKey: false, attachedNodeCount: 0, busy: false });
    assert.equal(actions.find((action) => action.id === "delete")?.disabled, true);
    assert.equal(actions.find((action) => action.id === "edit")?.label, "Add key");
  });

  it("flips the suspend label for a suspended provider", () => {
    const actions = buildRowActions({ provider: provider({ status: "suspended" }), hasKey: true, attachedNodeCount: 0, busy: false });
    assert.equal(actions.find((action) => action.id === "suspend")?.label, "Resume provider");
  });

  // Busy state is per-row now; it used to be global, so acting on one provider disabled them all.
  it("disables every action while that row is busy", () => {
    const actions = buildRowActions({ provider: provider(), hasKey: true, attachedNodeCount: 0, busy: true });
    assert.ok(actions.every((action) => action.disabled));
  });
});

describe("countAttachedNodes", () => {
  it("counts only nodes belonging to that provider", () => {
    const nodes = [node("provider-1", "a"), node("provider-1", "b"), node("provider-2", "c")];
    assert.equal(countAttachedNodes(nodes, "provider-1"), 2);
    assert.equal(countAttachedNodes(nodes, "provider-2"), 1);
    assert.equal(countAttachedNodes(nodes, "provider-3"), 0);
  });
});

describe("validateDnsServer", () => {
  it("accepts IPv4 and IPv6", () => {
    assert.equal(validateDnsServer("1.1.1.1").valid, true);
    assert.equal(validateDnsServer("9.9.9.9").valid, true);
    assert.equal(validateDnsServer("2606:4700:4700::1111").valid, true);
  });

  // Empty is a real choice: no DNS line, leaving the system resolver in place.
  it("accepts empty as meaning system DNS", () => {
    assert.equal(validateDnsServer("").valid, true);
    assert.equal(validateDnsServer("   ").valid, true);
  });

  it("rejects a hostname", () => {
    const result = validateDnsServer("dns.google");
    assert.equal(result.valid, false);
    assert.match((result as { message: string }).message, /IP address/);
  });

  it("rejects out-of-range octets", () => {
    assert.equal(validateDnsServer("999.1.1.1").valid, false);
    assert.equal(validateDnsServer("1.1.1.256").valid, false);
  });

  // Leading zeros are read as octal by some resolvers, so 010 is not 10.
  it("rejects leading zeros", () => {
    assert.equal(validateDnsServer("010.1.1.1").valid, false);
  });
});

describe("validateKeepalive", () => {
  it("accepts whole seconds within range, including zero", () => {
    assert.equal(validateKeepalive("0").valid, true);
    assert.equal(validateKeepalive("25").valid, true);
    assert.equal(validateKeepalive("65535").valid, true);
  });

  it("rejects empty, non-numeric, negative and out-of-range values", () => {
    assert.equal(validateKeepalive("").valid, false);
    assert.equal(validateKeepalive("abc").valid, false);
    assert.equal(validateKeepalive("-5").valid, false);
    assert.equal(validateKeepalive("2.5").valid, false);
    assert.equal(validateKeepalive("65536").valid, false);
  });
});

describe("describeKeepalive", () => {
  it("explains what zero actually does rather than just saying disabled", () => {
    const text = describeKeepalive(0);
    assert.match(text, /disabled/i);
    assert.match(text, /idle/i);
  });

  it("names the interval", () => {
    assert.match(describeKeepalive(25), /every 25s/);
  });
});

describe("validateMobileDevices", () => {
  it("accepts an empty list and a list of distinct names", () => {
    assert.deepEqual(validateMobileDevices([]), { valid: true });
    assert.deepEqual(validateMobileDevices(["Pixel 9", "  iPad "]), { valid: true });
  });

  it("refuses more devices than a server has spare slots", () => {
    const names = Array.from({ length: MAX_MOBILE_DEVICES + 1 }, (_, index) => `Phone ${index}`);
    const result = validateMobileDevices(names);
    assert.equal(result.valid, false);
    assert.match(!result.valid ? result.message : "", /room for 4 devices/);
  });

  it("refuses a blank name", () => {
    const result = validateMobileDevices(["Pixel 9", "   "]);
    assert.equal(result.valid, false);
    assert.match(!result.valid ? result.message : "", /Give every device a name/);
  });

  it("refuses a name over the limit, counting characters rather than bytes", () => {
    assert.deepEqual(validateMobileDevices(["é".repeat(MAX_DEVICE_NAME_CHARS)]), { valid: true });
    assert.equal(validateMobileDevices(["n".repeat(MAX_DEVICE_NAME_CHARS + 1)]).valid, false);
  });

  it("refuses the same name twice, ignoring case and surrounding spaces", () => {
    const result = validateMobileDevices(["Pixel 9", " pixel 9"]);
    assert.equal(result.valid, false);
    assert.match(!result.valid ? result.message : "", /in the list twice/);
  });
});
