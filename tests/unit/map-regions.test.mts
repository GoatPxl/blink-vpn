import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildMapRegions,
  cheapestPlan,
  groupPlaces,
  primaryRegion,
  type MapRegion,
} from "../../src/lib/map/map-regions.ts";
import { continentForTimeZone, suggestRegion } from "../../src/lib/suggest-region.ts";
import type { ProviderPlanPricing, ProviderRegion } from "../../src/lib/tauri.ts";

function plan(region: string, monthly: number | null, key = `plan-${monthly}`): ProviderPlanPricing {
  return {
    plan_key: key, plan_label: key, size_slug: key, region, memory_mb: 512, vcpus: 1, disk_gb: 10,
    transfer_tb: 1, transfer_note: "", monthly_usd: monthly, hourly_usd: monthly === null ? null : monthly / 730,
    estimated: false, source_timestamp: "2026-10-11T00:00:00Z",
  };
}

function region(slug: string, city: string, countryCode: string, continent: string, latitude: number, longitude: number): ProviderRegion {
  return { slug, city, area: null, country: null, country_code: countryCode, continent, latitude, longitude };
}

const regionsByProvider = {
  a: [region("sfo2", "San Francisco", "US", "North America", 37.77, -122.42), region("fra1", "Frankfurt", "DE", "Europe", 50.11, 8.68)],
  b: [region("eu-central", "Frankfurt", "DE", "Europe", 50.11, 8.68), region("ap-south", "Singapore", "SG", "Asia", 1.35, 103.82)],
  c: [region("us-east-1", "Ashburn", "US", "North America", 39.04, -77.49), region("nowhere", "Atlantis", "GR", "Europe", Number.NaN, Number.NaN)],
};
const pricingByProvider = {
  a: { sfo2: [plan("sfo2", 6), plan("sfo2", 4)], fra1: [plan("fra1", 4)] },
  b: { "eu-central": [plan("eu-central", 5)], "ap-south": [plan("ap-south", 5)] },
  c: { "us-east-1": [plan("us-east-1", 3.5)] },
};
const regions = buildMapRegions({
  providers: [
    { id: "a", name: "Alpha", verified: true },
    { id: "b", name: "Beta", verified: true },
    { id: "c", name: "Gamma", verified: false },
  ],
  regionsByProvider,
  pricingByProvider,
});

describe("cheapestPlan", () => {
  it("picks the lowest monthly price and never an unpriced plan over a priced one", () => {
    assert.equal(cheapestPlan([plan("x", 6), plan("x", null), plan("x", 4)])?.monthly_usd, 4);
    assert.equal(cheapestPlan([plan("x", null)]), null);
    assert.equal(cheapestPlan(undefined), null);
  });
});

describe("groupPlaces", () => {
  const places = groupPlaces({ regions, running: new Set(), favourites: new Set() });

  it("draws one pin per city, across providers", () => {
    const frankfurt = places.filter((place) => place.city === "Frankfurt");
    assert.equal(frankfurt.length, 1);
    assert.equal(frankfurt[0].regions.length, 2);
  });

  it("does not pin a place it cannot locate", () => {
    assert.ok(!places.some((place) => place.city === "Atlantis"));
  });

  it("ranks a buildable provider's cheapest region first", () => {
    const frankfurt = places.find((place) => place.city === "Frankfurt");
    assert.equal(frankfurt?.regions[0].key, "a:fra1");
  });

  it("marks a place needing a key when no provider there is verified", () => {
    assert.equal(places.find((place) => place.city === "Ashburn")?.state, "needs-key");
    assert.equal(places.find((place) => place.city === "Singapore")?.state, "ready");
  });

  it("lets a running server and a favourite outrank readiness", () => {
    const marked = groupPlaces({ regions, running: new Set(["b:eu-central"]), favourites: new Set(["b:ap-south"]) });
    const frankfurt = marked.find((place) => place.city === "Frankfurt");
    assert.equal(frankfurt?.state, "yours");
    assert.equal(marked.find((place) => place.city === "Singapore")?.state, "favourite");
    assert.ok(frankfurt);
    assert.equal(primaryRegion(frankfurt, new Set(["b:eu-central"])).key, "b:eu-central", "the running one, not the cheapest");
  });
});

describe("suggestRegion", () => {
  it("maps time zones to the region tables' continents", () => {
    assert.equal(continentForTimeZone("America/Los_Angeles"), "North America");
    assert.equal(continentForTimeZone("America/Sao_Paulo"), "South America");
    assert.equal(continentForTimeZone("Europe/Berlin"), "Europe");
    assert.equal(continentForTimeZone("Australia/Sydney"), "Oceania");
    assert.equal(continentForTimeZone("UTC"), null);
  });

  it("picks the cheapest buildable region on the user's continent", () => {
    assert.equal(suggestRegion(regions, "Europe/Paris")?.key, "a:fra1");
    // Gamma's $3.50 Ashburn is cheaper, but there is no key for it.
    assert.equal(suggestRegion(regions, "America/New_York")?.key, "a:sfo2");
  });

  it("falls back to North America, then anywhere", () => {
    assert.equal(suggestRegion(regions, "Africa/Johannesburg")?.key, "a:sfo2");
    const asiaOnly: MapRegion[] = regions.filter((candidate) => candidate.continent === "Asia");
    assert.equal(suggestRegion(asiaOnly, "Africa/Johannesburg")?.key, "b:ap-south");
  });

  it("suggests nothing without a verified key", () => {
    assert.equal(suggestRegion(regions.map((candidate) => ({ ...candidate, verified: false })), "Europe/Paris"), null);
  });
});
