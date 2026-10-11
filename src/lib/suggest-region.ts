/**
 * A first region to suggest, without asking anyone where the user is.
 *
 * Pure and offline: the IANA time zone the OS reports picks a continent and, for common zones, a
 * rough point to measure from. No IP geolocation and no network call — a VPN app that phoned a
 * third party to learn your location before you have a VPN would be making exactly the leak it
 * exists to prevent.
 */

import type { MapRegion } from "./map/map-regions.ts";
import { distanceKm } from "./map/project.ts";

/** Zones in the Americas south of the US, by their city part. */
const SOUTH_AMERICA_ZONES = new Set([
  "Argentina", "Asuncion", "Bahia", "Belem", "Boa_Vista", "Bogota", "Buenos_Aires", "Campo_Grande",
  "Caracas", "Cayenne", "Cuiaba", "Fortaleza", "Guayaquil", "La_Paz", "Lima", "Maceio", "Manaus",
  "Montevideo", "Paramaribo", "Porto_Velho", "Punta_Arenas", "Recife", "Rio_Branco", "Santarem",
  "Santiago", "Sao_Paulo",
]);

/** A rough centre for common zones, so ties on price go to the nearest city. */
const ZONE_POINTS: Record<string, { latitude: number; longitude: number }> = {
  "America/Los_Angeles": { latitude: 37, longitude: -120 },
  "America/Vancouver": { latitude: 49.3, longitude: -123.1 },
  "America/Denver": { latitude: 39.7, longitude: -105 },
  "America/Phoenix": { latitude: 33.4, longitude: -112 },
  "America/Chicago": { latitude: 41.9, longitude: -87.6 },
  "America/New_York": { latitude: 40.7, longitude: -74 },
  "America/Toronto": { latitude: 43.7, longitude: -79.4 },
  "America/Mexico_City": { latitude: 19.4, longitude: -99.1 },
  "America/Sao_Paulo": { latitude: -23.5, longitude: -46.6 },
  "Europe/London": { latitude: 51.5, longitude: -0.1 },
  "Europe/Dublin": { latitude: 53.3, longitude: -6.3 },
  "Europe/Paris": { latitude: 48.9, longitude: 2.4 },
  "Europe/Berlin": { latitude: 52.5, longitude: 13.4 },
  "Europe/Amsterdam": { latitude: 52.4, longitude: 4.9 },
  "Europe/Madrid": { latitude: 40.4, longitude: -3.7 },
  "Europe/Rome": { latitude: 41.9, longitude: 12.5 },
  "Europe/Stockholm": { latitude: 59.3, longitude: 18.1 },
  "Europe/Warsaw": { latitude: 52.2, longitude: 21 },
  "Asia/Kolkata": { latitude: 20, longitude: 78 },
  "Asia/Singapore": { latitude: 1.35, longitude: 103.8 },
  "Asia/Tokyo": { latitude: 35.7, longitude: 139.7 },
  "Asia/Seoul": { latitude: 37.6, longitude: 127 },
  "Asia/Hong_Kong": { latitude: 22.3, longitude: 114.2 },
  "Asia/Dubai": { latitude: 25.2, longitude: 55.3 },
  "Australia/Sydney": { latitude: -33.9, longitude: 151.2 },
  "Australia/Melbourne": { latitude: -37.8, longitude: 145 },
  "Africa/Johannesburg": { latitude: -26.2, longitude: 28 },
};

/** The continent a zone is on, spelled as the region tables spell it, or null when unclear. */
export function continentForTimeZone(timeZone: string): string | null {
  const [area, city = ""] = timeZone.split("/");
  switch (area) {
    case "America":
      return SOUTH_AMERICA_ZONES.has(city) || timeZone.startsWith("America/Argentina/") ? "South America" : "North America";
    case "US":
    case "Canada":
      return "North America";
    case "Europe":
      return "Europe";
    case "Asia":
      return "Asia";
    case "Australia":
    case "Pacific":
      return "Oceania";
    case "Africa":
      return "Africa";
    default:
      return null;
  }
}

/** The OS time zone, or UTC when the runtime will not say. */
export function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

/**
 * The cheapest region you can build on, on your continent; ties go to the nearest. With nothing on
 * your continent, the cheapest in North America — most providers' first region is there — and
 * failing that, the cheapest anywhere. Null only when no provider has a verified key.
 */
export function suggestRegion(regions: readonly MapRegion[], timeZone: string): MapRegion | null {
  const buildable = regions.filter((region) => region.verified);
  if (buildable.length === 0) return null;

  const continent = continentForTimeZone(timeZone);
  const origin = ZONE_POINTS[timeZone] ?? null;

  const pick = (candidates: readonly MapRegion[]) => {
    const scored = [...candidates].sort((a, b) => {
      const priceA = a.cheapest?.monthly_usd ?? Infinity;
      const priceB = b.cheapest?.monthly_usd ?? Infinity;
      if (priceA !== priceB) return priceA - priceB;
      if (origin && a.latitude !== null && a.longitude !== null && b.latitude !== null && b.longitude !== null) {
        const nearA = distanceKm(origin, { latitude: a.latitude, longitude: a.longitude });
        const nearB = distanceKm(origin, { latitude: b.latitude, longitude: b.longitude });
        if (nearA !== nearB) return nearA - nearB;
      }
      return a.key.localeCompare(b.key);
    });
    return scored[0] ?? null;
  };

  return (continent ? pick(buildable.filter((region) => region.continent === continent)) : null)
    ?? pick(buildable.filter((region) => region.continent === "North America"))
    ?? pick(buildable);
}
