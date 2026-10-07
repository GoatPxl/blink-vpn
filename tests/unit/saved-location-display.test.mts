import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  deriveLocationState,
  describeTurnOn,
  formatLocationPrice,
} from "../../src/lib/saved-location-display.ts";
import type { ProviderPlanPricing } from "../../src/lib/tauri.ts";

const location = {
  name: "Frankfurt",
  provider_name: "DigitalOcean",
  plan_key: "s-1vcpu-1gb",
  price_monthly_usd: 6,
  price_hourly_usd: 0.00893,
};

function plan(overrides: Partial<ProviderPlanPricing> = {}): ProviderPlanPricing {
  return {
    plan_key: "s-1vcpu-1gb",
    plan_label: "Basic 1 GB",
    size_slug: "s-1vcpu-1gb",
    region: "fra1",
    memory_mb: 1024,
    vcpus: 1,
    disk_gb: 25,
    transfer_tb: 1,
    transfer_note: "1 TB",
    monthly_usd: 7,
    hourly_usd: 0.01042,
    estimated: false,
    source_timestamp: "2026-10-06T00:00:00Z",
    ...overrides,
  };
}

describe("deriveLocationState", () => {
  it("reads a location with no server as off", () => {
    assert.equal(deriveLocationState(null), "off");
  });

  it("maps every lifecycle status", () => {
    assert.equal(deriveLocationState("provisioning"), "starting");
    assert.equal(deriveLocationState("bootstrapping"), "starting");
    assert.equal(deriveLocationState("ready"), "on");
    assert.equal(deriveLocationState("connected"), "on");
    assert.equal(deriveLocationState("deleting"), "stopping");
    // Not "on": a server in error may still bill, and connecting is not the way out.
    assert.equal(deriveLocationState("error"), "error");
  });
});

describe("formatLocationPrice", () => {
  it("shows monthly and hourly", () => {
    assert.equal(formatLocationPrice(6, 0.00893), "$6.00/mo · $0.0089/hr");
  });

  it("shows whichever half is known, and null for neither", () => {
    assert.equal(formatLocationPrice(6, null), "$6.00/mo");
    assert.equal(formatLocationPrice(null, null), null);
  });
});

describe("describeTurnOn", () => {
  it("quotes today's price, not last time's", () => {
    const message = describeTurnOn(location, plan());
    assert.ok(message);
    assert.match(message, /Turn on Frankfurt on DigitalOcean\?/);
    assert.match(message, /Plan: Basic 1 GB/);
    assert.match(message, /\$7\.00\/mo/);
    assert.doesNotMatch(message, /\$6\.00/);
  });

  it("falls back to the last known price, saying so, when the catalog cannot be read", () => {
    const message = describeTurnOn(location, undefined);
    assert.ok(message);
    assert.match(message, /\$6\.00\/mo · \$0\.0089\/hr, last time/);
  });

  it("refuses when the provider no longer offers the plan", () => {
    assert.equal(describeTurnOn(location, null), null);
  });
});
