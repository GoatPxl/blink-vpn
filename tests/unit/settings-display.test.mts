import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildRowActions,
  countAttachedNodes,
  deriveProviderKeyStatus,
  describeKeepalive,
  validateDnsServer,
  validateKeepalive,
} from "../../src/lib/settings-display.ts";
import type { ProviderRecord, VpnNodeRecord } from "../../src/lib/tauri.ts";

function provider(overrides: Partial<ProviderRecord> = {}): ProviderRecord {
  return {
    id: "provider-1",
    name: "DigitalOcean",
    slug: "digitalocean",
    status: "active",
    token_ref: "do-token__provider-1__abc",
    created_at: "2026-08-16T00:00:00.000Z",
    updated_at: "2026-08-16T00:00:00.000Z",
    ...overrides,
  } as ProviderRecord;
}

function node(providerId: string, id: string): VpnNodeRecord {
  return { id, provider_id: providerId } as VpnNodeRecord;
}

describe("deriveProviderKeyStatus", () => {
  const base = { provider: provider(), hasKey: true, isSelected: true };

  // The bug this replaces: the old table read only `token_ref` and `status`, so a key the provider
  // had rejected still rendered "Installed / Active".
  it("reports a rejected key as invalid", () => {
    assert.equal(deriveProviderKeyStatus({ ...base, providerAccess: "invalid" }), "invalid");
  });

  it("reports a validated key as verified", () => {
    assert.equal(deriveProviderKeyStatus({ ...base, providerAccess: "ready" }), "verified");
  });

  it("reports a stored but unchecked key as unverified", () => {
    assert.equal(deriveProviderKeyStatus({ ...base, providerAccess: "unknown" }), "unverified");
  });

  it("reports a missing key regardless of access state", () => {
    assert.equal(
      deriveProviderKeyStatus({ ...base, hasKey: false, providerAccess: "ready" }),
      "missing",
    );
  });

  // Suspension is the more actionable fact: operations will refuse whatever the key's state.
  it("lets suspension outrank verification", () => {
    assert.equal(
      deriveProviderKeyStatus({
        ...base,
        provider: provider({ status: "suspended" }),
        providerAccess: "ready",
      }),
      "suspended",
    );
  });

  // Validation state describes the selected provider only, so it must not leak onto other rows.
  it("does not apply the selected provider's validation result to other rows", () => {
    assert.equal(
      deriveProviderKeyStatus({ ...base, isSelected: false, providerAccess: "invalid" }),
      "unverified",
    );
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
