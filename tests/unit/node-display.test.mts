import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildRegionOptions,
  describeLocation,
  deriveNodeStatus,
  duplicatedRegions,
  flagEmoji,
  isConnectable,
  selectNodeRows,
  type NodeRow,
} from "../../src/lib/node-display.ts";
import type { ProviderRegion, VpnNodeRecord } from "../../src/lib/tauri.ts";

function node(overrides: Partial<VpnNodeRecord> = {}): VpnNodeRecord {
  return {
    id: "node-1",
    provider_id: "provider-1",
    name: "Node One",
    region: "sfo3",
    plan_key: null,
    size_slug: null,
    price_monthly_usd: 4,
    price_hourly_usd: 0.00595,
    price_source_timestamp: null,
    lifecycle_status: "ready",
    external_ref: "42",
    public_ipv4: "203.0.113.10",
    server_public_key: null,
    error_reason: null,
    created_at: "2026-08-15T10:00:00.000Z",
    updated_at: "2026-08-15T10:00:00.000Z",
    endpoint_port: 51820,
    ...overrides,
  };
}

const SFO: ProviderRegion = {
  slug: "sfo3",
  city: "San Francisco",
  area: "California",
  country: "United States",
  country_code: "US",
  continent: "North America",
};

describe("deriveNodeStatus", () => {
  // The original bug: STATUS rendered provider_status, so two powered-on droplets both read
  // "ACTIVE" and neither said whether the VPN was up.
  it("does not report two provider-active nodes as connected", () => {
    const a = deriveNodeStatus({ node: node({ provider_status: "active" }), isCurrent: true, isLiveConnected: false });
    const b = deriveNodeStatus({ node: node({ provider_status: "active" }), isCurrent: false, isLiveConnected: false });

    assert.equal(a, "current");
    assert.equal(b, "ready");
    assert.notEqual(a, "connected");
    assert.notEqual(b, "connected");
  });

  it("reports the live tunnel as connected regardless of the stored row", () => {
    const status = deriveNodeStatus({
      node: node({ lifecycle_status: "ready", provider_status: "active" }),
      isCurrent: false,
      isLiveConnected: true,
    });
    assert.equal(status, "connected");
  });

  // You cannot connect to a powered-off server; burying that in a details panel would leave
  // Connect failing for no visible reason.
  it("surfaces a powered-off droplet as unavailable even when the app thinks it is ready", () => {
    for (const providerStatus of ["off", "archive", "deleted"]) {
      const status = deriveNodeStatus({
        node: node({ lifecycle_status: "ready", provider_status: providerStatus }),
        isCurrent: true,
        isLiveConnected: false,
      });
      assert.equal(status, "unavailable", `provider_status=${providerStatus}`);
    }
  });

  it("treats in-progress lifecycles as provisioning", () => {
    for (const lifecycle of ["new", "provisioning", "bootstrapping"] as const) {
      assert.equal(
        deriveNodeStatus({ node: node({ lifecycle_status: lifecycle }), isCurrent: false, isLiveConnected: false }),
        "provisioning",
      );
    }
  });

  it("lets error outrank current, so a problem is never masked", () => {
    const status = deriveNodeStatus({
      node: node({ lifecycle_status: "error" }),
      isCurrent: true,
      isLiveConnected: false,
    });
    assert.equal(status, "error");
  });

  it("marks only connectable states as connectable", () => {
    assert.equal(isConnectable("ready"), true);
    assert.equal(isConnectable("current"), true);
    assert.equal(isConnectable("unavailable"), false);
    assert.equal(isConnectable("provisioning"), false);
    assert.equal(isConnectable("error"), false);
  });
});

describe("describeLocation", () => {
  it("leads with the city and area, and carries the country beneath", () => {
    const location = describeLocation(SFO, "sfo3");
    assert.equal(location.primary, "San Francisco, California");
    assert.equal(location.secondary, "United States");
    assert.equal(location.flag, "🇺🇸");
    assert.equal(location.unresolved, false);
  });

  it("omits the area when the provider does not disambiguate one", () => {
    const region: ProviderRegion = { ...SFO, slug: "fra1", city: "Frankfurt", area: null, country: "Germany", country_code: "DE" };
    assert.equal(describeLocation(region, "fra1").primary, "Frankfurt");
  });

  // Inventing a city for an unrecognised slug would tell the user they are exiting somewhere
  // they are not.
  it("falls back to the raw slug rather than guessing a place", () => {
    const location = describeLocation({ ...SFO, slug: "mars1", city: null, area: null, country: null, country_code: null }, "mars1");
    assert.equal(location.primary, "mars1");
    assert.equal(location.secondary, null);
    assert.equal(location.flag, null);
    assert.equal(location.unresolved, true);
  });

  it("falls back when the backend attached no geography at all", () => {
    assert.equal(describeLocation(undefined, "tor1").primary, "tor1");
    assert.equal(describeLocation(null, "tor1").unresolved, true);
  });
});

describe("flagEmoji", () => {
  it("maps ISO codes to regional-indicator pairs", () => {
    assert.equal(flagEmoji("US"), "🇺🇸");
    assert.equal(flagEmoji("de"), "🇩🇪");
  });

  it("rejects anything that is not a two-letter code", () => {
    assert.equal(flagEmoji(null), null);
    assert.equal(flagEmoji("USA"), null);
    assert.equal(flagEmoji("1"), null);
    assert.equal(flagEmoji("12"), null);
  });
});

describe("selectNodeRows", () => {
  function row(overrides: Partial<VpnNodeRecord>, status: NodeRow["status"]): NodeRow {
    const record = node(overrides);
    return { node: record, status, location: describeLocation(SFO, record.region) };
  }

  const rows: NodeRow[] = [
    row({ id: "a", name: "Alpha", region: "sfo3", created_at: "2026-08-01T00:00:00.000Z" }, "ready"),
    row({ id: "b", name: "Bravo", region: "fra1", created_at: "2026-08-10T00:00:00.000Z" }, "connected"),
    row({ id: "c", name: "Cirrus", region: "sfo3", created_at: "2026-08-05T00:00:00.000Z" }, "unavailable"),
  ];

  // The filter and the column now read the same derived value; they used to read different fields.
  it("filters on the same status the row displays", () => {
    const result = selectNodeRows({ rows, statusFilter: "connected", regionFilter: "all", sort: "newest" });
    assert.deepEqual(result.map((entry) => entry.node.id), ["b"]);
  });

  it("filters by region", () => {
    const result = selectNodeRows({ rows, statusFilter: "all", regionFilter: "sfo3", sort: "newest" });
    assert.deepEqual(result.map((entry) => entry.node.id), ["c", "a"]);
  });

  it("sorts newest and oldest first by creation time", () => {
    assert.deepEqual(
      selectNodeRows({ rows, statusFilter: "all", regionFilter: "all", sort: "newest" }).map((entry) => entry.node.id),
      ["b", "c", "a"],
    );
    assert.deepEqual(
      selectNodeRows({ rows, statusFilter: "all", regionFilter: "all", sort: "oldest" }).map((entry) => entry.node.id),
      ["a", "c", "b"],
    );
  });

  it("sorts by location, breaking ties on node name", () => {
    const result = selectNodeRows({ rows, statusFilter: "all", regionFilter: "all", sort: "location" });
    assert.deepEqual(result.map((entry) => entry.node.name), ["Alpha", "Bravo", "Cirrus"]);
  });

  it("does not mutate the input array", () => {
    const original = rows.map((entry) => entry.node.id);
    selectNodeRows({ rows, statusFilter: "all", regionFilter: "all", sort: "oldest" });
    assert.deepEqual(rows.map((entry) => entry.node.id), original);
  });
});

describe("buildRegionOptions", () => {
  const sfo2: ProviderRegion = { ...SFO, slug: "sfo2" };
  const fra1: ProviderRegion = {
    slug: "fra1", city: "Frankfurt", area: null, country: "Germany", country_code: "DE", continent: "Europe",
  };

  // Providers run several datacentres in one metro, which collapse to an identical label and
  // leave the user choosing between two entries that read exactly the same.
  it("appends the slug only where two regions would read identically", () => {
    const options = buildRegionOptions([
      { region: SFO, slug: "sfo3" },
      { region: sfo2, slug: "sfo2" },
      { region: fra1, slug: "fra1" },
    ]);

    const labels = options.map((option) => option.label);
    assert.equal(new Set(labels).size, labels.length, "every option label must be unique");

    const frankfurt = options.find((option) => option.slug === "fra1");
    assert.equal(frankfurt?.label, "Frankfurt — Germany", "an unambiguous region keeps a clean label");

    for (const slug of ["sfo2", "sfo3"]) {
      const option = options.find((entry) => entry.slug === slug);
      assert.ok(option?.label.includes(slug), `${slug} should be disambiguated by its slug`);
    }
  });

  // Grouped by country, then city — so the picker reads as a place list, not slug order.
  it("keeps labels clean when no two regions collide, sorted by country", () => {
    const options = buildRegionOptions([
      { region: SFO, slug: "sfo3" },
      { region: fra1, slug: "fra1" },
    ]);
    assert.deepEqual(options.map((entry) => entry.label), [
      "Frankfurt — Germany",
      "San Francisco, California — United States",
    ]);
  });

  it("sorts regions it cannot name after the ones it can", () => {
    const options = buildRegionOptions([
      { region: undefined, slug: "mars1" },
      { region: fra1, slug: "fra1" },
    ]);
    assert.deepEqual(options.map((entry) => entry.slug), ["fra1", "mars1"]);
  });

  it("dedupes repeated slugs and still labels unknown regions", () => {
    const options = buildRegionOptions([
      { region: SFO, slug: "sfo3" },
      { region: SFO, slug: "sfo3" },
      { region: undefined, slug: "mars1" },
    ]);

    assert.equal(options.length, 2);
    assert.equal(options.find((entry) => entry.slug === "mars1")?.label, "mars1");
  });
});

describe("duplicatedRegions", () => {
  // The table leads with location, so same-city nodes need the name to stay distinguishable.
  it("reports only regions holding more than one node", () => {
    const result = duplicatedRegions([
      node({ id: "a", region: "sfo3" }),
      node({ id: "b", region: "sfo3" }),
      node({ id: "c", region: "fra1" }),
    ]);

    assert.equal(result.has("sfo3"), true);
    assert.equal(result.has("fra1"), false);
  });

  it("is empty when every node is somewhere different", () => {
    const result = duplicatedRegions([node({ id: "a", region: "sfo3" }), node({ id: "b", region: "fra1" })]);
    assert.equal(result.size, 0);
  });
});
