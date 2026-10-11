"use client";

import Link from "next/link";
import { memo, useEffect, useMemo, useState } from "react";

import { PixelIcon } from "@/components/ui/pixel-icon";
import { formatAgo, formatMonthly, lastLocation, locationKey, regionCode } from "@/lib/home-display";
import { LAND_MASK, MASK_COLUMNS, MASK_ROWS } from "@/lib/map/land-mask";
import {
  buildMapRegions,
  groupPlaces,
  primaryRegion,
  rankRegions,
  type MapPlace,
  type MapProvider,
  type MapRegion,
  type PinState,
} from "@/lib/map/map-regions";
import { cellOf } from "@/lib/map/project";
import { loadProviderPricingCatalog } from "@/lib/provider-pricing-cache";
import { hasUsableKey, visibleProviders } from "@/lib/settings-display";
import { localTimeZone, suggestRegion } from "@/lib/suggest-region";
import {
  listKnownProviderRegions,
  listProviderRegions,
  type ProviderPlanPricing,
  type ProviderRecord,
  type ProviderRegion,
  type SavedLocationRecord,
  type VpnNodeRecord,
} from "@/lib/tauri";
import { Button, Panel, Row, Tag, useNow, VIEW_ROUTE } from "./shared";

export type MapFilter = "all" | "cheapest" | "visited" | "favourites";

const PIN_STYLE: Record<PinState, string> = {
  yours: "bg-lime outline outline-2 outline-offset-[3px] outline-lime",
  favourite: "bg-ember",
  ready: "bg-bone",
  "needs-key": "bg-stone-2",
};

const PIN_LABEL: Record<PinState, string> = {
  yours: "your server",
  favourite: "favourite",
  ready: "ready to build",
  "needs-key": "needs a key",
};

/** Regions and prices for every provider the UI offers, loaded once per mount. */
function useMapData(input: { providers: readonly MapProvider[] }) {
  const [regionsByProvider, setRegionsByProvider] = useState<Record<string, ProviderRegion[]>>({});
  const [pricingByProvider, setPricingByProvider] = useState<Record<string, Record<string, ProviderPlanPricing[]>>>({});
  const [failed, setFailed] = useState<string[]>([]);
  // Re-run only when which providers exist, or which can build, changes.
  const signature = input.providers.map((provider) => `${provider.id}:${provider.verified}`).join(",");

  useEffect(() => {
    let active = true;
    const providers = input.providers;
    void Promise.all(
      providers.map(async (provider) => {
        try {
          // A verified key gets the live catalog: what can actually be built. Without one, the
          // adapter's own table still shows where the provider runs.
          const regions = provider.verified ? await listProviderRegions(provider.id) : await listKnownProviderRegions(provider.id);
          return [provider.id, regions] as const;
        } catch {
          if (active) setFailed((current) => [...new Set([...current, provider.name])]);
          return [provider.id, [] as ProviderRegion[]] as const;
        }
      }),
    ).then((entries) => {
      if (active) setRegionsByProvider(Object.fromEntries(entries));
    });

    // Prices arrive after the pins: one request per region adds up, and the map is useful before.
    for (const provider of providers.filter((candidate) => candidate.verified)) {
      void loadProviderPricingCatalog(provider.id)
        .then((catalog) => {
          if (active) setPricingByProvider((current) => ({ ...current, [provider.id]: catalog.pricingByRegion }));
        })
        .catch(() => undefined);
    }
    return () => {
      active = false;
    };
    // `signature` stands in for `input.providers`, whose identity changes on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [signature]);

  const regions = useMemo(
    () => buildMapRegions({ providers: input.providers, regionsByProvider, pricingByProvider }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [signature, regionsByProvider, pricingByProvider],
  );
  return { regions, loaded: Object.keys(regionsByProvider).length > 0, failed };
}

/** The 72×36 dot world. Static, so it renders once. */
const DotWorld = memo(function DotWorld() {
  return (
    <div
      aria-hidden="true"
      className="grid gap-[3px]"
      style={{ gridTemplateColumns: `repeat(${MASK_COLUMNS}, minmax(0, 1fr))` }}
    >
      {LAND_MASK.flatMap((row, y) =>
        [...row].map((cell, x) => (
          <span key={`${x}-${y}`} className={`aspect-square ${cell === "#" ? "bg-panel-2" : "bg-ink-2"}`} />
        )),
      )}
    </div>
  );
});

function Pin({
  place,
  dimmed,
  selected,
  showLabel,
  onSelect,
}: {
  place: MapPlace;
  dimmed: boolean;
  selected: boolean;
  showLabel: boolean;
  onSelect: () => void;
}) {
  // Snapped to the centre of the dot that holds the city.
  const { column, row } = cellOf(place.point);
  const left = ((column + 0.5) / MASK_COLUMNS) * 100;
  const top = ((row + 0.5) / MASK_ROWS) * 100;
  const code = regionCode(place.regions[0].slug);

  return (
    <button
      type="button"
      onClick={onSelect}
      aria-label={`${place.city}, ${place.countryCode} — ${PIN_LABEL[place.state]}`}
      aria-pressed={selected}
      className={`group absolute flex -translate-x-1/2 -translate-y-1/2 items-center gap-1.5 ${dimmed ? "opacity-25" : ""} ${selected ? "z-20" : "z-10 hover:z-20"}`}
      style={{ left: `${left}%`, top: `${top}%` }}
    >
      <span className={`block h-3.5 w-3.5 ${PIN_STYLE[place.state]} ${selected ? "scale-125" : "group-hover:scale-125"} transition-transform`} />
      <span
        className={`absolute left-5 whitespace-nowrap bg-ink/80 px-1 font-mono text-[11.5px] ${place.state === "yours" ? "text-lime" : place.state === "favourite" ? "text-ember" : "text-bone"} ${showLabel || selected ? "" : "hidden group-hover:block group-focus-visible:block"}`}
      >
        {code}{place.state === "yours" ? " · you" : place.state === "favourite" ? " ★" : ""}
      </span>
    </button>
  );
}

export function MapView({
  providers,
  nodes,
  savedLocations,
  favourites,
  dockerDevEnabled,
  busy,
  filter,
  moveHint,
  onFavourite,
  onChoose,
}: {
  providers: ProviderRecord[];
  nodes: VpnNodeRecord[];
  savedLocations: SavedLocationRecord[];
  favourites: readonly string[];
  dockerDevEnabled: boolean;
  busy: boolean;
  filter: MapFilter;
  /** Arrived from Home's "change": say what to do. */
  moveHint: boolean;
  onFavourite: (key: string, favourite: boolean) => void;
  /** Open the build or move sheet for a region. */
  onChoose: (region: MapRegion) => void;
}) {
  const now = useNow();
  const mapProviders = useMemo<MapProvider[]>(
    () => visibleProviders(providers, dockerDevEnabled).map((provider) => ({
      id: provider.id,
      name: provider.name,
      verified: hasUsableKey(provider),
    })),
    [dockerDevEnabled, providers],
  );
  const { regions, loaded, failed } = useMapData({ providers: mapProviders });

  const running = useMemo(
    () => new Set(nodes.filter((node) => node.lifecycle_status !== "deleted").map((node) => `${node.provider_id}:${node.region}`)),
    [nodes],
  );
  const favouriteSet = useMemo(() => new Set(favourites), [favourites]);
  const visited = useMemo(() => new Set(savedLocations.map(locationKey)), [savedLocations]);
  const places = useMemo(() => groupPlaces({ regions, running, favourites: favouriteSet }), [favouriteSet, regions, running]);
  const unplaced = useMemo(
    () => rankRegions(regions.filter((region) => region.verified && (region.latitude === null || region.longitude === null))),
    [regions],
  );
  const suggested = useMemo(() => suggestRegion(regions, localTimeZone()), [regions]);
  const cheapestPrice = useMemo(
    () => Math.min(...regions.filter((region) => region.verified).map((region) => region.cheapest?.monthly_usd ?? Infinity)),
    [regions],
  );

  const placeOf = (key: string) => places.find((place) => place.regions.some((region) => region.key === key)) ?? null;
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  // Default selection: your server, else the suggestion for this time zone.
  const defaultPlace = places.find((place) => place.state === "yours") ?? (suggested ? placeOf(suggested.key) : null) ?? null;
  const selectedPlace = places.find((place) => place.key === selectedKey) ?? defaultPlace;

  function matchesFilter(place: MapPlace): boolean {
    switch (filter) {
      case "all":
        return true;
      case "cheapest":
        return place.regions.some((region) => region.verified && region.cheapest?.monthly_usd === cheapestPrice);
      case "visited":
        return place.regions.some((region) => visited.has(region.key));
      case "favourites":
        return place.regions.some((region) => favouriteSet.has(region.key));
    }
  }

  const last = lastLocation(savedLocations);
  const firstFavourite = favourites.map((key) => regions.find((region) => region.key === key)).find(Boolean) ?? null;
  const quick: Array<{ icon: "refresh" | "dollar" | "star"; label: string; region: MapRegion | null }> = [
    { icon: "refresh", label: "Same as last time", region: last ? regions.find((region) => region.key === locationKey(last)) ?? null : null },
    { icon: "dollar", label: "Cheapest near me", region: suggested },
    { icon: "star", label: "Favourite", region: firstFavourite },
  ];

  return (
    <div className="flex flex-col gap-5">
      {moveHint ? (
        <div className="border-2 border-lime bg-panel px-6 py-3">
          <Tag state="secure">Pick a city to move to</Tag>
        </div>
      ) : null}
      {failed.length > 0 ? (
        <p className="text-[13px] text-ember">Could not read regions from {failed.join(", ")}. Check the key in Setup.</p>
      ) : null}

      <Panel bodyClassName="relative px-6 pb-10 pt-6">
        <div className="relative mx-auto max-w-[960px]">
          <DotWorld />
          {places.map((place) => (
            <Pin
              key={place.key}
              place={place}
              dimmed={!matchesFilter(place)}
              selected={selectedPlace?.key === place.key}
              showLabel={place.state === "yours" || place.state === "favourite" || (place.state === "ready" && matchesFilter(place) && filter !== "all")}
              onSelect={() => setSelectedKey(place.key)}
            />
          ))}
          {!loaded ? (
            <div className="absolute inset-0 flex items-center justify-center">
              <span className="border-2 border-line-3 bg-ink px-4 py-2 font-display text-[12px] uppercase text-stone">Reading regions…</span>
            </div>
          ) : null}
        </div>
        <div className="absolute bottom-3 left-6 flex flex-wrap gap-x-5 gap-y-1">
          {(Object.keys(PIN_LABEL) as PinState[]).map((state) => (
            <span key={state} className="flex items-center gap-2 font-mono text-[11.5px] uppercase text-stone">
              <span className={`h-2.5 w-2.5 ${PIN_STYLE[state].split(" ")[0]}`} />
              {PIN_LABEL[state]}
            </span>
          ))}
        </div>
      </Panel>

      <div className="grid grid-cols-2 gap-5 max-[1000px]:grid-cols-1">
        <Panel title="Quick build" action={<span className="font-display text-[12px] uppercase text-stone">~2 min each</span>} bodyClassName="px-6 py-2">
          {quick.map((item) => (
            <Row key={item.label}>
              <PixelIcon name={item.icon} className={`h-4 w-4 shrink-0 ${item.icon === "star" ? "text-ember" : "text-lime"}`} />
              {item.region ? (
                <button
                  type="button"
                  disabled={busy || running.has(item.region.key)}
                  onClick={() => item.region && onChoose(item.region)}
                  className="flex min-w-0 flex-1 items-center justify-between gap-4 text-left disabled:opacity-40"
                >
                  <span className="truncate text-[16px] text-bone">
                    {item.label} · <span className="font-semibold">{item.region.city ?? item.region.slug}</span>
                  </span>
                  <span className="shrink-0 font-mono text-[14px] text-stone">{formatMonthly(item.region.cheapest?.monthly_usd ?? null) ?? "—"}</span>
                </button>
              ) : (
                <span className="flex-1 text-[15px] text-stone-2">
                  {item.label} · {item.icon === "star" ? "star a city to fill this" : item.icon === "refresh" ? "nothing yet" : "add a key in Setup"}
                </span>
              )}
            </Row>
          ))}
        </Panel>

        <SelectedPanel
          place={selectedPlace}
          running={running}
          favourites={favouriteSet}
          savedLocations={savedLocations}
          anyRunning={running.size > 0}
          busy={busy}
          now={now}
          onFavourite={onFavourite}
          onChoose={onChoose}
        />
      </div>

      {unplaced.length > 0 ? (
        <Panel title="Not on the map" description="Regions Blink cannot place on a map. They build the same way." bodyClassName="px-6 py-2">
          {unplaced.map((region) => (
            <Row key={region.key}>
              <div data-region-key={region.key} className="flex min-w-0 flex-1 items-center gap-4">
                <span className="font-display text-[16px] uppercase text-lime">{regionCode(region.slug)}</span>
                <span className="flex-1 truncate text-[14px] text-stone">{region.providerName}</span>
                <span className="font-mono text-[14px] text-stone">{formatMonthly(region.cheapest?.monthly_usd ?? null) ?? "—"}</span>
                <Button size="sm" tone="ghost" disabled={busy || running.has(region.key)} onClick={() => onChoose(region)}>
                  Build here
                </Button>
              </div>
            </Row>
          ))}
        </Panel>
      ) : null}
    </div>
  );
}

function SelectedPanel({
  place,
  running,
  favourites,
  savedLocations,
  anyRunning,
  busy,
  now,
  onFavourite,
  onChoose,
}: {
  place: MapPlace | null;
  running: ReadonlySet<string>;
  favourites: ReadonlySet<string>;
  savedLocations: SavedLocationRecord[];
  anyRunning: boolean;
  busy: boolean;
  now: number;
  onFavourite: (key: string, favourite: boolean) => void;
  onChoose: (region: MapRegion) => void;
}) {
  if (!place) {
    return (
      <Panel title="Selected">
        <p className="text-[14px] text-stone">Click a dot to see what it costs to build there.</p>
      </Panel>
    );
  }

  const region = primaryRegion(place, running);
  const others = place.regions.filter((candidate) => candidate.providerId !== region.providerId);
  const alsoOn = [...new Map(others.map((candidate) => [candidate.providerId, candidate])).values()];
  const saved = savedLocations.find((location) => locationKey(location) === region.key) ?? null;
  const isFavourite = favourites.has(region.key);
  const isRunning = running.has(region.key);

  return (
    <Panel
      title={<>Selected · {place.city}</>}
      action={<span className="font-display text-[12px] uppercase text-stone">{region.providerName} · {regionCode(region.slug)}</span>}
      bodyClassName="px-6 py-2"
    >
      <Row>
        <span className="flex-1 text-[15px] text-stone">Cheapest size</span>
        <span className="font-mono text-[15px] text-bone">
          {region.cheapest ? `${region.cheapest.size_slug} · ${formatMonthly(region.cheapest.monthly_usd)}` : region.verified ? "reading prices…" : "—"}
        </span>
      </Row>
      <Row>
        <span className="flex-1 text-[15px] text-stone">Also on</span>
        <span className="text-right text-[15px] text-bone">
          {alsoOn.length === 0
            ? "—"
            : alsoOn.map((candidate) => `${candidate.providerName} ${candidate.verified ? formatMonthly(candidate.cheapest?.monthly_usd ?? null) ?? "" : "(no key)"}`.trim()).join(" · ")}
        </span>
      </Row>
      <Row>
        <span className="flex-1 text-[15px] text-stone">Last visit</span>
        <span className="text-[15px] text-bone">{saved ? formatAgo(saved.last_turned_on_at, now) ?? "—" : "never"}</span>
      </Row>
      <div className="flex flex-wrap gap-3 py-4">
        {!region.verified ? (
          <Link
            href={VIEW_ROUTE.setup}
            className="inline-flex h-12 items-center border-2 border-bone px-5 font-display text-[14px] uppercase text-bone hover:bg-panel-2"
          >
            Add a key for {region.providerName}
          </Link>
        ) : isRunning ? (
          <span className="inline-flex h-12 items-center font-display text-[14px] uppercase text-lime">You&apos;re here</span>
        ) : (
          <Button disabled={busy} onClick={() => onChoose(region)}>{anyRunning ? "Move here" : "Build here"}</Button>
        )}
        <Button tone="quiet" pressed={isFavourite} onClick={() => onFavourite(region.key, !isFavourite)}>
          <PixelIcon name="star" className={`h-3.5 w-3.5 ${isFavourite ? "text-ember" : ""}`} /> Favourite
        </Button>
      </div>
    </Panel>
  );
}
