/**
 * Pure logic behind the Home screen: what a server has cost so far, how long ago a location was
 * last on, and which location "Connect" means.
 *
 * Cost is the product's argument — a server that exists only while it is needed — so Home shows it
 * at all times. It is an estimate from the price snapshot taken at create time; the provider's
 * invoice is the truth, and Shield says so.
 */

import type { ProviderRegion, SavedLocationRecord, VpnNodeRecord } from "./tauri";

/**
 * `(now - created_at) * hourly`, or null when the node has no hourly price.
 *
 * Null rather than zero: an OpenStack operator sets its own prices and publishes none, and "$0.00"
 * would claim the server is free.
 */
export function costSoFarUsd(
  node: Pick<VpnNodeRecord, "created_at" | "price_hourly_usd">,
  now: number,
): number | null {
  if (node.price_hourly_usd === null) return null;
  const created = Date.parse(node.created_at);
  if (Number.isNaN(created)) return null;
  const hours = Math.max(0, now - created) / 3_600_000;
  return hours * node.price_hourly_usd;
}

/** `$0.02`, or `$0.006` below a cent so a fresh server does not read as free. */
export function formatUsd(value: number | null): string {
  if (value === null) return "—";
  if (value > 0 && value < 0.01) return `$${value.toFixed(3)}`;
  return `$${value.toFixed(2)}`;
}

/** `$0.006/hr`, keeping the significant digits hourly prices actually have. */
export function formatHourly(value: number | null): string | null {
  if (value === null) return null;
  const digits = value < 0.1 ? 3 : 2;
  return `$${value.toFixed(digits)}/hr`;
}

/** `$4/mo`, without trailing zeros on whole dollars. */
export function formatMonthly(value: number | null): string | null {
  if (value === null) return null;
  return Number.isInteger(value) ? `$${value}/mo` : `$${value.toFixed(2)}/mo`;
}

/** `just now`, `14 m ago`, `3 h ago`, `6 d ago`. Null for a missing or unparseable time. */
export function formatAgo(iso: string | null, now: number): string | null {
  if (!iso) return null;
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return null;
  const minutes = Math.floor(Math.max(0, now - then) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} h ago`;
  return `${Math.floor(hours / 24)} d ago`;
}

/** Local wall-clock `21:40` for a timestamp, as in "SECURE SINCE 21:40". */
export function formatClock(iso: string | null): string | null {
  if (!iso) return null;
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return null;
  const date = new Date(then);
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

/** "San Francisco, US", falling back to the region slug when the adapter cannot place it. */
export function cityLine(region: ProviderRegion | null | undefined, slug: string): string {
  if (!region?.city) return slug;
  return region.country_code ? `${region.city}, ${region.country_code}` : region.city;
}

/** Short code for a tile: the region slug, upper-cased (`SFO2`, `EU-CENTRAL`). */
export function regionCode(slug: string): string {
  return slug.toUpperCase();
}

/**
 * Saved locations in the order Home offers them: favourites first, then most recently on.
 *
 * `favourites` holds `<provider_id>:<region>` keys.
 */
export function orderLocations(
  locations: SavedLocationRecord[],
  favourites: readonly string[] = [],
): SavedLocationRecord[] {
  const favourite = new Set(favourites);
  const lastOn = (location: SavedLocationRecord) => Date.parse(location.last_turned_on_at ?? "") || 0;
  return [...locations].sort((a, b) => {
    const favA = favourite.has(locationKey(a)) ? 1 : 0;
    const favB = favourite.has(locationKey(b)) ? 1 : 0;
    if (favA !== favB) return favB - favA;
    return lastOn(b) - lastOn(a);
  });
}

/** The location "Connect" turns back on: the one most recently on, regardless of favourites. */
export function lastLocation(locations: SavedLocationRecord[]): SavedLocationRecord | null {
  return orderLocations(locations)[0] ?? null;
}

/** The key favourites are stored under. */
export function locationKey(location: { provider_id: string; region: string }): string {
  return `${location.provider_id}:${location.region}`;
}
