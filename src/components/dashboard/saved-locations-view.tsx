"use client";

import { describeLocation } from "@/lib/node-display";
import {
  LOCATION_STATE_LABEL,
  deriveLocationState,
  formatLocationPrice,
  type LocationState,
} from "@/lib/saved-location-display";
import type { SavedLocationRecord } from "@/lib/tauri";

import { Button, Panel, type DashboardBusyAction } from "./shared";

const STATE_COLOR: Record<LocationState, string> = {
  off: "var(--on-surface-variant)",
  starting: "var(--primary)",
  on: "var(--secondary)",
  stopping: "var(--tertiary)",
  error: "var(--tertiary)",
};

/**
 * Every location the user has run a server in, and whether it is on.
 *
 * The point of the app is that servers do not stay up: this is the list that makes turning one off
 * cheap, because turning it back on is one click at a known price.
 */
export function SavedLocationsView({
  locations,
  busyAction,
  onTurnOn,
  onTurnOff,
  onForget,
}: {
  locations: SavedLocationRecord[];
  busyAction: DashboardBusyAction;
  onTurnOn: (location: SavedLocationRecord) => Promise<void>;
  onTurnOff: (location: SavedLocationRecord) => Promise<void>;
  onForget: (location: SavedLocationRecord) => Promise<void>;
}) {
  if (locations.length === 0) return null;

  return (
    <Panel
      title="Saved locations"
      description="Turn a server off to stop paying for it; turn it back on here in one click. Prices are from the last time each was on."
    >
      <SavedLocationList
        locations={locations}
        busyAction={busyAction}
        onTurnOn={onTurnOn}
        onTurnOff={onTurnOff}
        onForget={onForget}
      />
    </Panel>
  );
}

/** The rows themselves, shown in the server list on the map. */
export function SavedLocationList({
  locations,
  busyAction,
  onTurnOn,
  onTurnOff,
  onForget,
}: {
  locations: SavedLocationRecord[];
  busyAction: DashboardBusyAction;
  onTurnOn: (location: SavedLocationRecord) => Promise<void>;
  onTurnOff?: (location: SavedLocationRecord) => Promise<void>;
  onForget?: (location: SavedLocationRecord) => Promise<void>;
}) {
  const busy = busyAction !== null;

  return (
    <ul className="divide-y divide-[color:var(--outline-variant)]">
      {locations.map((location) => {
        const state = deriveLocationState(location.node_status);
        const place = describeLocation(location.region_geo, location.region);
        const price = formatLocationPrice(location.price_monthly_usd, location.price_hourly_usd);

        return (
          <li
            key={location.id}
            data-saved-location-id={location.id}
            data-state={state}
            className="flex flex-wrap items-center gap-x-4 gap-y-2 py-3"
          >
            <div className="flex min-w-0 flex-1 items-center gap-3">
              {place.flag ? <span className="text-[20px] leading-none" aria-hidden="true">{place.flag}</span> : null}
              <div className="min-w-0">
                <div className="truncate font-medium text-[var(--on-surface)]">
                  {location.name}
                  <span className="ml-2 text-xs font-normal text-[var(--on-surface-variant)]">
                    {place.unresolved ? place.primary : [place.primary, place.secondary].filter(Boolean).join(", ")}
                  </span>
                </div>
                <div className="mt-0.5 truncate text-xs text-[var(--on-surface-variant)]">
                  {location.provider_name ?? location.provider_id} · <span className="font-mono">{location.plan_key}</span>
                  {price ? ` · ${price}` : null}
                </div>
              </div>
            </div>

            <span className="flex items-center gap-2 text-xs" style={{ color: STATE_COLOR[state] }}>
              <span className="h-2 w-2 rounded-full" style={{ backgroundColor: STATE_COLOR[state] }} />
              {LOCATION_STATE_LABEL[state]}
            </span>

            <div className="flex items-center gap-2">
              {state === "off" ? (
                <Button className="min-w-[96px] px-3 py-2 text-xs" disabled={busy} onClick={() => void onTurnOn(location)}>
                  Turn on
                </Button>
              ) : onTurnOff && (state === "on" || state === "error") ? (
                <Button tone="ghost" className="min-w-[96px] px-3 py-2 text-xs" disabled={busy} onClick={() => void onTurnOff(location)}>
                  Turn off
                </Button>
              ) : null}
              {onForget && state === "off" ? (
                <Button tone="ghost" className="px-3 py-2 text-xs" disabled={busy} onClick={() => void onForget(location)}>
                  Forget
                </Button>
              ) : null}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
