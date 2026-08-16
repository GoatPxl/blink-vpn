"use client";

import { listProviderPricing, listProviderRegions, type ProviderPlanPricing, type ProviderRegion } from "@/lib/tauri";

export type ProviderPricingCatalog = {
  providerId: string;
  /** Full region records, so callers can name a location without a second lookup. */
  regions: ProviderRegion[];
  /** Keyed by region slug. */
  pricingByRegion: Record<string, ProviderPlanPricing[]>;
};

const pricingCatalogCache = new Map<string, Promise<ProviderPricingCatalog>>();

export async function loadProviderPricingCatalog(providerId: string): Promise<ProviderPricingCatalog> {
  const cachedCatalog = pricingCatalogCache.get(providerId);
  if (cachedCatalog) {
    return cachedCatalog;
  }

  const catalogPromise = (async () => {
    const regions = await listProviderRegions(providerId);
    const pricingEntries = await Promise.all(
      regions.map(async (region) => {
        const response = await listProviderPricing(providerId, region.slug);
        return [region.slug, response.plans] as const;
      }),
    );

    return {
      providerId,
      regions,
      pricingByRegion: Object.fromEntries(pricingEntries),
    };
  })();

  pricingCatalogCache.set(providerId, catalogPromise);

  try {
    return await catalogPromise;
  } catch (error) {
    pricingCatalogCache.delete(providerId);
    throw error;
  }
}