import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  cityLine,
  costSoFarUsd,
  formatAgo,
  formatHourly,
  formatMonthly,
  formatUsd,
  lastLocation,
  orderLocations,
} from "../../src/lib/home-display.ts";
import type { SavedLocationRecord } from "../../src/lib/tauri.ts";

const NOW = Date.parse("2026-10-11T12:00:00Z");
const hoursAgo = (h: number) => new Date(NOW - h * 3_600_000).toISOString();

function location(region: string, lastOnHoursAgo: number | null): SavedLocationRecord {
  return {
    id: `loc-${region}`,
    provider_id: "provider-1",
    provider_name: "DigitalOcean",
    name: region,
    region,
    plan_key: "s-1vcpu-512mb-10gb",
    price_monthly_usd: 4,
    price_hourly_usd: 0.006,
    price_source_timestamp: null,
    node_id: null,
    node_status: null,
    last_turned_on_at: lastOnHoursAgo === null ? null : hoursAgo(lastOnHoursAgo),
    created_at: hoursAgo(1000),
    updated_at: hoursAgo(1),
  };
}

describe("costSoFarUsd", () => {
  it("multiplies elapsed hours by the hourly price", () => {
    const cost = costSoFarUsd({ created_at: hoursAgo(2), price_hourly_usd: 0.006 }, NOW);
    assert.ok(cost !== null && Math.abs(cost - 0.012) < 1e-9);
  });

  it("is null, not zero, when there is no price", () => {
    assert.equal(costSoFarUsd({ created_at: hoursAgo(2), price_hourly_usd: null }, NOW), null);
  });

  it("never goes negative on clock skew", () => {
    assert.equal(costSoFarUsd({ created_at: hoursAgo(-1), price_hourly_usd: 0.006 }, NOW), 0);
  });
});

describe("money formatting", () => {
  it("keeps sub-cent amounts visible", () => {
    assert.equal(formatUsd(0.004), "$0.004");
    assert.equal(formatUsd(0.0234), "$0.02");
    assert.equal(formatUsd(null), "—");
  });

  it("formats hourly and monthly prices", () => {
    assert.equal(formatHourly(0.006), "$0.006/hr");
    assert.equal(formatMonthly(4), "$4/mo");
    assert.equal(formatMonthly(4.5), "$4.50/mo");
    assert.equal(formatMonthly(null), null);
  });
});

describe("formatAgo", () => {
  it("steps from minutes to hours to days", () => {
    assert.equal(formatAgo(hoursAgo(0), NOW), "just now");
    assert.equal(formatAgo(hoursAgo(0.5), NOW), "30 m ago");
    assert.equal(formatAgo(hoursAgo(3), NOW), "3 h ago");
    assert.equal(formatAgo(hoursAgo(24 * 6), NOW), "6 d ago");
    assert.equal(formatAgo(null, NOW), null);
  });
});

describe("cityLine", () => {
  it("prefers city and country code, falling back to the slug", () => {
    assert.equal(
      cityLine({ slug: "sfo2", city: "San Francisco", area: "California", country: "United States", country_code: "US", continent: "North America" }, "sfo2"),
      "San Francisco, US",
    );
    assert.equal(cityLine(undefined, "regionOne"), "regionOne");
  });
});

describe("location order", () => {
  const sfo = location("sfo2", 2);
  const fra = location("fra1", 24 * 6);
  const nyc = location("nyc3", null);

  it("puts the most recently used first", () => {
    assert.deepEqual(orderLocations([fra, nyc, sfo]).map((l) => l.region), ["sfo2", "fra1", "nyc3"]);
  });

  it("lifts favourites above recency", () => {
    assert.deepEqual(
      orderLocations([sfo, fra, nyc], ["provider-1:nyc3"]).map((l) => l.region),
      ["nyc3", "sfo2", "fra1"],
    );
  });

  it("means the last one used by Connect, never a favourite", () => {
    assert.equal(lastLocation([fra, sfo])?.region, "sfo2");
    assert.equal(lastLocation([]), null);
  });
});
