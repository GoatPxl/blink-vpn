/**
 * Every region the Map can offer, merged across providers, and grouped into places.
 *
 * A place is a city: several providers, and several datacentres of one provider, can sit in the
 * same one, and the map draws one pin for it. Which provider a pin means is decided here, by rules
 * that treat every provider alike — nothing in this module knows any provider by name (AGENTS.md §3).
 */

import type { ProviderPlanPricing, ProviderRegion } from "../tauri";
import { projectToMap, type MapPoint } from "./project.ts";

export type MapProvider = {
  id: string;
  name: string;
  /** A stored key that has not failed its last check: Blink can build here right now. */
  verified: boolean;
};

export type MapRegion = {
  /** `<provider_id>:<region slug>`, the same key favourites use. */
  key: string;
  providerId: string;
  providerName: string;
  slug: string;
  city: string | null;
  countryCode: string | null;
  continent: string | null;
  latitude: number | null;
  longitude: number | null;
  verified: boolean;
  /** Cheapest priced plan, or null while prices load or when the provider publishes none. */
  cheapest: ProviderPlanPricing | null;
};

export type PinState = "yours" | "favourite" | "ready" | "needs-key";

export type MapPlace = {
  /** `city|country_code`. */
  key: string;
  city: string;
  countryCode: string;
  point: MapPoint;
  /** Every region in this place, best first (see `rankRegions`). */
  regions: MapRegion[];
  state: PinState;
};

/** The plan with the lowest monthly price. Unpriced plans lose to any priced one. */
export function cheapestPlan(plans: readonly ProviderPlanPricing[] | undefined): ProviderPlanPricing | null {
  let best: ProviderPlanPricing | null = null;
  for (const plan of plans ?? []) {
    if (plan.monthly_usd === null) continue;
    if (best === null || (best.monthly_usd ?? Infinity) > plan.monthly_usd) best = plan;
  }
  return best;
}

export function buildMapRegions(input: {
  providers: readonly MapProvider[];
  regionsByProvider: Readonly<Record<string, readonly ProviderRegion[]>>;
  pricingByProvider: Readonly<Record<string, Readonly<Record<string, readonly ProviderPlanPricing[]>>>>;
}): MapRegion[] {
  const regions: MapRegion[] = [];
  for (const provider of input.providers) {
    for (const region of input.regionsByProvider[provider.id] ?? []) {
      regions.push({
        key: `${provider.id}:${region.slug}`,
        providerId: provider.id,
        providerName: provider.name,
        slug: region.slug,
        city: region.city,
        countryCode: region.country_code,
        continent: region.continent,
        latitude: region.latitude,
        longitude: region.longitude,
        verified: provider.verified,
        cheapest: cheapestPlan(input.pricingByProvider[provider.id]?.[region.slug]),
      });
    }
  }
  return regions;
}

/**
 * Best first: a provider you can build on, then the cheapest, then the shortest slug (the
 * provider's main datacentre in a city usually has the plainest name), then alphabetically, so the
 * order is stable from one render to the next.
 */
export function rankRegions(regions: readonly MapRegion[]): MapRegion[] {
  return [...regions].sort((a, b) => {
    if (a.verified !== b.verified) return a.verified ? -1 : 1;
    const priceA = a.cheapest?.monthly_usd ?? Infinity;
    const priceB = b.cheapest?.monthly_usd ?? Infinity;
    if (priceA !== priceB) return priceA - priceB;
    if (a.slug.length !== b.slug.length) return a.slug.length - b.slug.length;
    return a.key.localeCompare(b.key);
  });
}

/**
 * One place per city with coordinates. Regions without them are left out here — the list view
 * still shows them — because a pin in the wrong place is worse than no pin.
 */
export function groupPlaces(input: {
  regions: readonly MapRegion[];
  /** Region keys with a server running now. */
  running: ReadonlySet<string>;
  favourites: ReadonlySet<string>;
}): MapPlace[] {
  const byPlace = new Map<string, MapRegion[]>();
  for (const region of input.regions) {
    if (!region.city || !region.countryCode) continue;
    const key = `${region.city}|${region.countryCode}`;
    byPlace.set(key, [...(byPlace.get(key) ?? []), region]);
  }

  const places: MapPlace[] = [];
  for (const [key, regions] of byPlace) {
    const [first] = regions;
    const point = projectToMap(first.latitude, first.longitude);
    if (!point || !first.city || !first.countryCode) continue;

    const state: PinState = regions.some((region) => input.running.has(region.key))
      ? "yours"
      : regions.some((region) => input.favourites.has(region.key))
        ? "favourite"
        : regions.some((region) => region.verified)
          ? "ready"
          : "needs-key";

    places.push({ key, city: first.city, countryCode: first.countryCode, point, regions: rankRegions(regions), state });
  }
  return places.sort((a, b) => a.key.localeCompare(b.key));
}

/** The region a place means when the user clicks it: the one running, else the best ranked. */
export function primaryRegion(place: MapPlace, running: ReadonlySet<string>): MapRegion {
  return place.regions.find((region) => running.has(region.key)) ?? place.regions[0];
}
