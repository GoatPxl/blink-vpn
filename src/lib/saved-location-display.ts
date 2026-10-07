/**
 * Pure presentation logic for saved locations.
 *
 * Free of JSX and React so it can be unit tested directly, following `node-display.ts`.
 */

import type { NodeLifecycleStatus, ProviderPlanPricing, SavedLocationRecord } from "./tauri";

/** What a saved location is doing, from the user's point of view. */
export type LocationState = "off" | "starting" | "on" | "stopping" | "error";

export const LOCATION_STATE_LABEL: Record<LocationState, string> = {
  off: "Off",
  starting: "Starting",
  on: "On",
  stopping: "Turning off",
  error: "Needs attention",
};

/**
 * The location's state, from its linked server's lifecycle status.
 *
 * `error` is its own state rather than "on": a server stuck in error may still exist at the
 * provider and bill, and the way out is turning it off (or force cleanup), not connecting.
 */
export function deriveLocationState(nodeStatus: NodeLifecycleStatus | null): LocationState {
  switch (nodeStatus) {
    case null:
    case "deleted":
      return "off";
    case "new":
    case "provisioning":
    case "bootstrapping":
      return "starting";
    case "ready":
    case "connected":
      return "on";
    case "deleting":
      return "stopping";
    case "error":
      return "error";
  }
}

function formatUsd(value: number, digits: number): string {
  return `$${value.toFixed(digits)}`;
}

/** "$6.00/mo · $0.0089/hr", either half alone, or null when no price is known. */
export function formatLocationPrice(
  monthlyUsd: number | null,
  hourlyUsd: number | null,
): string | null {
  const parts: string[] = [];
  if (monthlyUsd !== null) parts.push(`${formatUsd(monthlyUsd, 2)}/mo`);
  if (hourlyUsd !== null) parts.push(`${formatUsd(hourlyUsd, 4)}/hr`);
  return parts.length > 0 ? parts.join(" · ") : null;
}

/**
 * The question asked before turning a location on: it starts billing, so the user is shown what it
 * costs now, not what it cost last time.
 *
 * @param livePlan the plan in today's catalog; `null` when the provider no longer offers it, and
 * `undefined` when the catalog could not be read (the last known price is shown instead).
 * @returns null when the plan is gone, since a turn-on would only fail at the provider.
 */
export function describeTurnOn(
  location: Pick<
    SavedLocationRecord,
    "name" | "provider_name" | "plan_key" | "price_monthly_usd" | "price_hourly_usd"
  >,
  livePlan: ProviderPlanPricing | null | undefined,
): string | null {
  if (livePlan === null) return null;

  const where = `${location.name} on ${location.provider_name ?? "its provider"}`;
  const plan = livePlan?.plan_label ?? location.plan_key;
  const price = livePlan
    ? formatLocationPrice(livePlan.monthly_usd, livePlan.hourly_usd)
    : formatLocationPrice(location.price_monthly_usd, location.price_hourly_usd);
  const priceNote = livePlan
    ? (price ?? "no published price")
    : price
      ? `${price}, last time (current price could not be checked)`
      : "price unknown (current price could not be checked)";

  return `Turn on ${where}?\n\nPlan: ${plan}\nCost: ${priceNote}\n\nBilling starts as soon as the server is created.`;
}
