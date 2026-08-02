"use client";

import { listProviderPricing, listProviderRegions, type ProviderPlanPricing } from "@/lib/tauri";

export type ProviderPricingCatalog = {
  providerId: string;
  regions: string[];
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
        const response = await listProviderPricing(providerId, region);
        return [region, response.plans] as const;
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