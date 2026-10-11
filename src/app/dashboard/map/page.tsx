"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useMemo, useState } from "react";

import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { MapView, type MapFilter } from "@/components/dashboard/map-view";
import { MoveSheet } from "@/components/dashboard/move-sheet";
import { ServerList } from "@/components/dashboard/server-list";
import { Button, PageHeader, VIEW_ROUTE } from "@/components/dashboard/shared";
import { locationKey } from "@/lib/home-display";
import type { MapRegion } from "@/lib/map/map-regions";

const FILTERS: Array<{ id: MapFilter; label: string }> = [
  { id: "all", label: "All" },
  { id: "cheapest", label: "Cheapest" },
  { id: "visited", label: "Visited" },
  { id: "favourites", label: "Favourites" },
];

function MapPage() {
  const router = useRouter();
  const params = useSearchParams();
  const {
    activeNode,
    appPreferences,
    busyAction,
    connectAttempt,
    dockerDevEnabled,
    moveTo,
    nodes,
    operationAttempt,
    providers,
    savedLocations,
    updateAppPreferences,
  } = useDashboard();
  const [view, setView] = useState<"map" | "list">("map");
  const [filter, setFilter] = useState<MapFilter>(params.get("filter") === "cheapest" ? "cheapest" : "all");
  const [target, setTarget] = useState<MapRegion | null>(null);

  const favourites = useMemo(() => appPreferences?.favourites ?? [], [appPreferences?.favourites]);
  const busy = busyAction !== null || connectAttempt !== null || operationAttempt !== null;
  const from = nodes.find((node) => node.lifecycle_status === "connected") ?? activeNode ?? nodes[0] ?? null;

  const onFavourite = useCallback((key: string, favourite: boolean) => {
    const next = favourite ? [...favourites, key] : favourites.filter((candidate) => candidate !== key);
    void updateAppPreferences({ favourites: next });
  }, [favourites, updateAppPreferences]);

  const lastPlanKey = target
    ? savedLocations.find((location) => locationKey(location) === target.key)?.plan_key ?? null
    : null;

  const counts: Partial<Record<MapFilter, number>> = {
    visited: savedLocations.length,
    favourites: favourites.length,
  };

  return (
    <>
      <PageHeader
        title="Map"
        subtitle="Every region from every cloud you have a key for. Click a dot to build there."
        action={
          <div className="flex flex-wrap gap-3">
            {FILTERS.map((option) => (
              <Button
                key={option.id}
                size="sm"
                tone={view === "map" && filter === option.id ? "ghost" : "quiet"}
                pressed={view === "map" && filter === option.id}
                className={view === "map" && filter === option.id ? "!border-lime !text-lime" : ""}
                onClick={() => {
                  setView("map");
                  setFilter(option.id);
                }}
              >
                {option.label}{counts[option.id] !== undefined ? ` · ${counts[option.id]}` : ""}
              </Button>
            ))}
            <Button
              size="sm"
              tone={view === "list" ? "ghost" : "quiet"}
              pressed={view === "list"}
              className={view === "list" ? "!border-lime !text-lime" : ""}
              onClick={() => setView("list")}
            >
              List view
            </Button>
          </div>
        }
      />

      {view === "list" ? (
        <ServerList onBuildNew={() => setView("map")} />
      ) : (
        <MapView
          providers={providers}
          nodes={nodes}
          savedLocations={savedLocations}
          favourites={favourites}
          dockerDevEnabled={dockerDevEnabled}
          busy={busy}
          filter={filter}
          moveHint={params.get("move") === "1" && from !== null}
          onFavourite={onFavourite}
          onChoose={setTarget}
        />
      )}

      {target ? (
        <MoveSheet
          key={target.key}
          target={target}
          from={from}
          lastPlanKey={lastPlanKey}
          busy={busy}
          onClose={() => setTarget(null)}
          onConfirm={(input) => {
            setTarget(null);
            // The build has its own screen, with the game; Home is where it lives.
            router.push(VIEW_ROUTE.home);
            void moveTo(input);
          }}
        />
      ) : null}
    </>
  );
}

/** `useSearchParams` needs a Suspense boundary in a static export. */
export default function MapRoute() {
  return (
    <Suspense fallback={null}>
      <MapPage />
    </Suspense>
  );
}
