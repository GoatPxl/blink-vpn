"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";

import { HomeHero } from "@/components/dashboard/home-hero";
import { Button, GiantAction, Panel, Tag, Tile, VIEW_ROUTE, useNow, type DashboardBusyAction } from "@/components/dashboard/shared";
import { PixelIcon } from "@/components/ui/pixel-icon";
import { cityLine, formatAgo, formatMonthly, lastLocation, orderLocations, regionCode } from "@/lib/home-display";
import type { SavedLocationRecord } from "@/lib/tauri";

/** How many saved locations get a tile beside "Cheapest near me". */
const TILE_COUNT = 3;

/**
 * Home with no server running.
 *
 * Servers are meant to be turned off, so "off" is the normal resting state, not a first run. The
 * one primary action turns the last location back on; the tiles offer somewhere else.
 */
export function DashboardEmptyState({
  savedLocations,
  favourites,
  busyAction,
  hasVerifiedKey,
  onTurnOn,
}: {
  savedLocations: SavedLocationRecord[];
  favourites: readonly string[];
  busyAction: DashboardBusyAction;
  /** False on a first run: no provider key yet, so nothing can be built. */
  hasVerifiedKey: boolean;
  onTurnOn: (location: SavedLocationRecord) => Promise<void>;
}) {
  const router = useRouter();
  const now = useNow();
  const last = lastLocation(savedLocations);
  const others = orderLocations(savedLocations, favourites).filter((location) => location.id !== last?.id);
  const favourite = new Set(favourites);
  const busy = busyAction !== null;

  if (!hasVerifiedKey) {
    return (
      <HomeHero
        tone="off"
        tag="Off · nothing running · nothing billing"
        headline="Off"
        subline={<span className="text-stone">Add a cloud key and Blink builds your first server.</span>}
        action={
          <GiantAction
            wide
            tone="primary"
            label="Get started"
            sublabel="Key · build · connect"
            icon={<PixelIcon name="key" className="h-12 w-12" />}
            onClick={() => router.push("/onboarding")}
          />
        }
        footer={<p className="text-[14px] text-stone">Takes about three minutes. You need an account with a cloud provider such as DigitalOcean.</p>}
      />
    );
  }

  const lastPrice = last ? formatMonthly(last.price_monthly_usd) : null;

  return (
    <>
      <HomeHero
        tone="off"
        tag="Off · nothing running · nothing billing"
        headline="Off"
        subline={
          last ? (
            <span className="text-stone">
              Last time: {cityLine(last.region_geo, last.region)}
              {formatAgo(last.last_turned_on_at, now) ? ` · ${formatAgo(last.last_turned_on_at, now)}` : ""}
            </span>
          ) : (
            <span className="text-stone">Pick a city on the map to build your first server.</span>
          )
        }
        action={
          last ? (
            <GiantAction
              wide
              tone="primary"
              label="Connect"
              sublabel={[last.region_geo?.city ?? last.region, "~2 min", lastPrice].filter(Boolean).join(" · ")}
              icon={<PixelIcon name="power" className="h-12 w-12" />}
              onClick={() => void onTurnOn(last)}
              disabled={busy}
            />
          ) : (
            <GiantAction
              wide
              tone="primary"
              label="Open the map"
              icon={<PixelIcon name="map" className="h-12 w-12" />}
              onClick={() => router.push(VIEW_ROUTE.map)}
            />
          )
        }
        footer={
          <div>
            <div className="mb-4 flex items-center justify-between gap-4">
              <Tag>Or somewhere else</Tag>
              <Link href={VIEW_ROUTE.map} className="inline-flex items-center gap-1 text-[15px] text-lime">
                Open the map <PixelIcon name="arrow-right" className="h-3.5 w-3.5" />
              </Link>
            </div>
            <div className="grid grid-cols-4 gap-5 max-[1000px]:grid-cols-2">
              {others.slice(0, TILE_COUNT).map((location) => (
                <Tile
                  key={location.id}
                  code={regionCode(location.region)}
                  name={location.region_geo?.city ?? location.name}
                  meta={[
                    favourite.has(`${location.provider_id}:${location.region}`) ? "favourite" : formatAgo(location.last_turned_on_at, now) ? `last on ${formatAgo(location.last_turned_on_at, now)}` : null,
                    formatMonthly(location.price_monthly_usd),
                  ].filter(Boolean).join(" · ")}
                  onClick={() => void onTurnOn(location)}
                  disabled={busy}
                />
              ))}
              <Tile
                dashed
                code="+"
                name="Cheapest near me"
                meta="any cloud with a key"
                onClick={() => router.push(`${VIEW_ROUTE.map}?filter=cheapest`)}
              />
            </div>
          </div>
        }
      />

      <Panel bodyClassName="flex items-center gap-5 px-8 py-5">
        <PixelIcon name="tray" className="h-7 w-7 shrink-0 text-stone" />
        <p className="min-w-0 flex-1 text-[15px] text-stone">
          Tip: closing this window keeps Blink in the tray, next to the clock. Your tunnel stays up.
        </p>
        <Button tone="quiet" onClick={() => router.push(VIEW_ROUTE.setup)}>Window settings</Button>
      </Panel>
    </>
  );
}
