/**
 * Latitude and longitude to a position on the dot map, as percentages of its box.
 *
 * Equirectangular to match how the land mask was rasterised (`land-mask.ts` holds the bounds), so a
 * pin sits on the dot that contains its city.
 */

import { MASK_COLUMNS, MASK_LAT_BOTTOM, MASK_LAT_TOP, MASK_ROWS } from "./land-mask.ts";

export type MapPoint = { xPercent: number; yPercent: number };

/** Null for a place the map cannot show: no coordinates, or below the cropped southern edge. */
export function projectToMap(latitude: number | null, longitude: number | null): MapPoint | null {
  if (latitude === null || longitude === null) return null;
  if (!Number.isFinite(latitude) || !Number.isFinite(longitude)) return null;
  if (latitude > MASK_LAT_TOP || latitude < MASK_LAT_BOTTOM) return null;

  const xPercent = ((longitude + 180) / 360) * 100;
  const yPercent = ((MASK_LAT_TOP - latitude) / (MASK_LAT_TOP - MASK_LAT_BOTTOM)) * 100;
  return { xPercent: Math.min(100, Math.max(0, xPercent)), yPercent };
}

/** The mask cell a point falls in, for snapping a pin onto its dot. */
export function cellOf(point: MapPoint): { column: number; row: number } {
  return {
    column: Math.min(MASK_COLUMNS - 1, Math.floor((point.xPercent / 100) * MASK_COLUMNS)),
    row: Math.min(MASK_ROWS - 1, Math.floor((point.yPercent / 100) * MASK_ROWS)),
  };
}

/** Great-circle distance in km, for "nearest". Haversine, which is plenty for picking a city. */
export function distanceKm(
  a: { latitude: number; longitude: number },
  b: { latitude: number; longitude: number },
): number {
  const radians = (degrees: number) => (degrees * Math.PI) / 180;
  const dLat = radians(b.latitude - a.latitude);
  const dLon = radians(b.longitude - a.longitude);
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(radians(a.latitude)) * Math.cos(radians(b.latitude)) * Math.sin(dLon / 2) ** 2;
  return 2 * 6371 * Math.asin(Math.sqrt(h));
}
