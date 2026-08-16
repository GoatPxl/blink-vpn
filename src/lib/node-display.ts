/**
 * Pure presentation logic for the node list.
 *
 * Kept free of JSX and React so it can be unit tested directly, and so the status shown in a row
 * and the status offered by the filter are guaranteed to come from one function.
 */

import type { ProviderRegion, VpnNodeRecord } from "./tauri";

/**
 * What a node is doing, from the user's point of view.
 *
 * Deliberately one value derived from three independent facts, because showing any one of them
 * alone misleads: the table used to render `provider_status`, so two provisioned droplets both
 * read "ACTIVE" and neither indicated whether the VPN was up.
 */
export type NodeStatus =
  | "connected"
  | "connecting"
  | "current"
  | "ready"
  | "provisioning"
  | "unavailable"
  | "error";

export const NODE_STATUS_LABEL: Record<NodeStatus, string> = {
  connected: "Connected",
  connecting: "Connecting",
  current: "Current",
  ready: "Ready",
  provisioning: "Setting up",
  unavailable: "Unavailable",
  error: "Error",
};

/** Ordering for the filter dropdown and for status-first sorting: most relevant first. */
export const NODE_STATUS_ORDER: NodeStatus[] = [
  "connected",
  "connecting",
  "current",
  "ready",
  "provisioning",
  "unavailable",
  "error",
];

/**
 * Collapse a node's three status axes into the one thing worth showing.
 *
 * Precedence matters:
 * - A live tunnel wins outright — it is the only fact the user can feel.
 * - A connect in flight comes next, and outranks `error`: it is the most recent thing the user did,
 *   and during it their internet is down, so it is what they need explained. A stale error from a
 *   previous attempt must not mask the attempt now running.
 * - `error` outranks the rest, because it needs action.
 * - A provider-side problem (`off`/`archive`/`deleted`) surfaces as `unavailable` even for a node
 *   the app considers ready: you cannot connect to a powered-off server, and hiding that in a
 *   details panel would leave Connect failing for no visible reason.
 * - `current` only shows for a node that is otherwise ready, so it never masks a problem.
 */
export function deriveNodeStatus(input: {
  node: VpnNodeRecord;
  isCurrent: boolean;
  isLiveConnected: boolean;
  /** A connect this app started is still in flight for this node. */
  isConnecting: boolean;
}): NodeStatus {
  const { node, isCurrent, isLiveConnected, isConnecting } = input;

  if (isLiveConnected) return "connected";
  if (isConnecting) return "connecting";

  const lifecycle = node.lifecycle_status.toLowerCase();
  if (lifecycle === "error") return "error";
  if (lifecycle === "new" || lifecycle === "provisioning" || lifecycle === "bootstrapping") {
    return "provisioning";
  }
  if (lifecycle === "deleting") return "unavailable";

  const providerStatus = node.provider_status?.toLowerCase();
  if (providerStatus === "off" || providerStatus === "archive" || providerStatus === "deleted") {
    return "unavailable";
  }

  return isCurrent ? "current" : "ready";
}

/**
 * Whether a node in this state can be connected to at all.
 *
 * `connecting` is excluded: a second connect for a tunnel already coming up raises a second UAC
 * prompt and races the same service, which the backend now rejects outright.
 */
export function isConnectable(status: NodeStatus): boolean {
  return status === "current" || status === "ready";
}

export type LocationDisplay = {
  /** The line to lead with — a city, or the raw slug when geography is unknown. */
  primary: string;
  /** Country, or null when unknown. Never a guess. */
  secondary: string | null;
  /** Emoji flag derived from the ISO-3166 code, or null. */
  flag: string | null;
  /** True when nothing but the slug is known, so callers can style it as unresolved. */
  unresolved: boolean;
};

/**
 * How to render a node's location.
 *
 * Falls back to the slug rather than inventing a place. The previous mapping covered three slugs
 * out of roughly fifteen and silently showed bare codes for the rest.
 */
export function describeLocation(region: ProviderRegion | null | undefined, slug: string): LocationDisplay {
  const city = region?.city ?? null;

  if (!city) {
    return { primary: slug || "Unknown", secondary: null, flag: null, unresolved: true };
  }

  const area = region?.area ?? null;
  const country = region?.country ?? null;

  return {
    primary: area ? `${city}, ${area}` : city,
    secondary: country,
    flag: flagEmoji(region?.country_code ?? null),
    unresolved: false,
  };
}

/**
 * ISO-3166 alpha-2 to its emoji flag, by offsetting each letter into the regional-indicator block.
 * Avoids shipping a second country table just for icons.
 */
export function flagEmoji(countryCode: string | null): string | null {
  if (!countryCode || countryCode.length !== 2) return null;

  const code = countryCode.toUpperCase();
  if (!/^[A-Z]{2}$/.test(code)) return null;

  const REGIONAL_INDICATOR_A = 0x1f1e6;
  const LETTER_A = "A".charCodeAt(0);
  return String.fromCodePoint(
    ...[...code].map((letter) => REGIONAL_INDICATOR_A + (letter.charCodeAt(0) - LETTER_A)),
  );
}

export type RegionOption = { slug: string; label: string };

/**
 * Region picker entries, labelled so no two read the same.
 *
 * Providers run several datacentres in one metro — DigitalOcean has `sfo2` and `sfo3` in San
 * Francisco — which collapse to an identical "San Francisco, California" and leave the user
 * choosing between two things that look the same. Where that happens the slug is appended, and
 * only there, so the common case stays clean.
 */
export function buildRegionOptions(regions: Array<{ region: ProviderRegion | null | undefined; slug: string }>): RegionOption[] {
  const seen = new Map<string, { slug: string; base: string; sortKey: string }>();

  for (const { region, slug } of regions) {
    if (seen.has(slug)) continue;
    const { primary, secondary } = describeLocation(region, slug);
    seen.set(slug, {
      slug,
      base: secondary ? `${primary} — ${secondary}` : primary,
      sortKey: locationSortKey(region, slug),
    });
  }

  const entries = [...seen.values()];
  const baseCounts = new Map<string, number>();
  for (const entry of entries) {
    baseCounts.set(entry.base, (baseCounts.get(entry.base) ?? 0) + 1);
  }

  return entries
    .sort((left, right) => left.sortKey.localeCompare(right.sortKey) || left.slug.localeCompare(right.slug))
    .map(({ slug, base }) => ({
      slug,
      label: (baseCounts.get(base) ?? 0) > 1 ? `${base} (${slug})` : base,
    }));
}

/** Sort key for grouping the region filter by country, then city. */
function locationSortKey(region: ProviderRegion | null | undefined, slug: string): string {
  const country = region?.country ?? "";
  const city = region?.city ?? "";
  // Unknown regions sort last rather than first, so recognised places lead the list.
  return country || city ? `0${country}${city}` : `1${slug}`;
}

export type NodeSortOrder = "location" | "newest" | "oldest";

export type NodeRow = {
  node: VpnNodeRecord;
  status: NodeStatus;
  location: LocationDisplay;
};

/**
 * Filter and sort in one place, so the options a filter offers cannot drift from the values the
 * table renders — they did: the dropdown filtered `lifecycle_status` while the column showed
 * `provider_status`, so choosing "ready" returned rows that all read "ACTIVE".
 */
export function selectNodeRows(input: {
  rows: NodeRow[];
  statusFilter: NodeStatus | "all";
  regionFilter: string;
  sort: NodeSortOrder;
}): NodeRow[] {
  const { rows, statusFilter, regionFilter, sort } = input;

  const filtered = rows.filter((row) => {
    const statusMatches = statusFilter === "all" || row.status === statusFilter;
    const regionMatches = regionFilter === "all" || row.node.region === regionFilter;
    return statusMatches && regionMatches;
  });

  return [...filtered].sort((left, right) => {
    if (sort === "location") {
      const compared = left.location.primary.localeCompare(right.location.primary);
      if (compared !== 0) return compared;
      return left.node.name.localeCompare(right.node.name);
    }

    const leftTime = Date.parse(left.node.created_at || left.node.updated_at || "") || 0;
    const rightTime = Date.parse(right.node.created_at || right.node.updated_at || "") || 0;
    return sort === "newest" ? rightTime - leftTime : leftTime - rightTime;
  });
}

/**
 * Region slugs that appear more than once among these nodes.
 *
 * The table leads with location, so two nodes in the same city would otherwise be
 * indistinguishable; callers show the node name only for these.
 */
export function duplicatedRegions(nodes: VpnNodeRecord[]): Set<string> {
  const counts = new Map<string, number>();
  for (const node of nodes) {
    counts.set(node.region, (counts.get(node.region) ?? 0) + 1);
  }

  return new Set([...counts.entries()].filter(([, count]) => count > 1).map(([region]) => region));
}
