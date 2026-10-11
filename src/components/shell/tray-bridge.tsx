"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useRef } from "react";

import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { useNow, VIEW_ROUTE } from "@/components/dashboard/shared";
import { costSoFarUsd, formatClock, formatUsd, lastLocation, locationKey, orderLocations } from "@/lib/home-display";
import { deriveShellTone } from "@/lib/shell-tone";

type TrayWindow = Window & { __TAURI_INTERNALS__?: unknown };

type TrayItem = { id: string; label: string };
type TrayState = {
  tone: "secure" | "busy" | "off" | "failed";
  status: string;
  primary: TrayItem | null;
  places_label: string;
  places: TrayItem[];
  power_off: TrayItem | null;
};

/** How often the "$x so far" in the menu is allowed to move. The menu is rebuilt on change. */
const COST_TICK_MS = 60_000;

function inDesktopShell(): boolean {
  return typeof window !== "undefined" && !!(window as TrayWindow).__TAURI_INTERNALS__;
}

async function showWindow() {
  const appWindow = getCurrentWindow();
  await appWindow.show();
  await appWindow.unminimize();
  await appWindow.setFocus();
}

/**
 * Keeps the tray in step with the app, and runs what its menu asks for.
 *
 * The tray's state comes from the same `deriveShellTone` that colours the rail, so the two cannot
 * disagree. Its menu clicks arrive as `tray-action` events and run through the same functions the
 * window's buttons call; the tray never calls a provider itself.
 */
export function TrayBridge() {
  const router = useRouter();
  const dashboard = useDashboard();
  const {
    activeNode,
    appPreferences,
    connectAttempt,
    health,
    nodes,
    operationAttempt,
    recovery,
    savedLocations,
    tunnelConnected,
    tunnelPhase,
  } = dashboard;

  // The cost line moves once a minute; anything faster would rebuild the menu for nothing.
  const costTick = Math.floor(useNow() / COST_TICK_MS);
  const node = activeNode ?? nodes[0] ?? null;
  const last = lastLocation(savedLocations);

  const state = useMemo<TrayState>(() => {
    const tone = deriveShellTone({
      tunnelPhase,
      connecting: connectAttempt !== null,
      operating: operationAttempt !== null,
      failed: recovery !== null,
    });
    const nodePlace = node ? node.region_geo?.city ?? node.name : null;
    const lastPlace = last ? last.region_geo?.city ?? last.name : null;
    const since = formatClock(health?.connected_at ?? null);

    const status = operationAttempt?.kind === "provision"
      ? `Building ${operationAttempt.place}…`
      : connectAttempt
        ? "Connecting · internet pauses briefly"
        : tunnelConnected && nodePlace
          ? `Secure · ${nodePlace}${since ? ` · since ${since}` : ""}`
          : node && nodePlace
            ? `Server on · ${nodePlace} · not connected`
            : "Off · nothing billing";

    const busy = connectAttempt !== null || operationAttempt !== null;
    const primary: TrayItem | null = busy
      ? null
      : tunnelConnected
        ? { id: "disconnect", label: "Disconnect" }
        : node && nodePlace
          ? { id: "connect", label: `Connect · ${nodePlace}` }
          : lastPlace
            ? { id: "connect-last", label: `Connect · ${lastPlace}` }
            : null;

    const places = orderLocations(savedLocations, appPreferences?.favourites ?? [])
      .filter((location) => location.node_id === null)
      .slice(0, 3)
      .map((location) => ({ id: `place:${locationKey(location)}`, label: location.region_geo?.city ?? location.name }));

    const cost = node ? formatUsd(costSoFarUsd(node, costTick * COST_TICK_MS)) : null;
    return {
      tone,
      status,
      primary,
      places_label: tunnelConnected ? "Change location" : "Connect elsewhere",
      places,
      power_off: node && !busy ? { id: "power-off", label: `Power off server (${cost} so far)` } : null,
    };
  }, [appPreferences?.favourites, connectAttempt, costTick, health?.connected_at, last, node, operationAttempt, recovery, savedLocations, tunnelConnected, tunnelPhase]);

  // Push only real changes: each push rebuilds the native menu.
  const lastSent = useRef<string | null>(null);
  useEffect(() => {
    if (!inDesktopShell()) return;
    const encoded = JSON.stringify(state);
    if (encoded === lastSent.current) return;
    lastSent.current = encoded;
    void invoke("update_tray", { state }).catch(() => {
      lastSent.current = null;
    });
  }, [state]);

  // The handlers read the latest dashboard through a ref, so listeners are registered once.
  const latest = useRef({ ...dashboard, router, node, last });
  useEffect(() => {
    latest.current = { ...dashboard, router, node, last };
  });

  useEffect(() => {
    if (!inDesktopShell()) return;
    let disposed = false;
    const stops: Array<() => void> = [];

    void listen<string>("tray-action", (event) => {
      const current = latest.current;
      const id = event.payload;
      if (id === "disconnect") {
        void (current.connectedTunnel
          ? current.disconnectTunnel(current.connectedTunnel)
          : current.node
            ? current.forceDisconnectNodeService(current.node)
            : Promise.resolve());
      } else if (id === "connect") {
        void (current.connectedTunnel ? current.connectTunnel(current.connectedTunnel) : current.connectPrimary());
      } else if (id === "connect-last") {
        // Double-click: connect to the server that is on, else turn the last place back on.
        if (current.node) void (current.connectedTunnel ? current.connectTunnel(current.connectedTunnel) : current.connectPrimary());
        else if (current.last) void current.turnOnLocation(current.last);
      } else if (id === "open-map") {
        void showWindow();
        current.router.push(VIEW_ROUTE.map);
      } else if (id === "power-off" && current.node) {
        // The undo lives in the window, so the window comes up with it.
        void showWindow();
        void current.destroyNode(current.node);
      } else if (id.startsWith("place:")) {
        const key = id.slice("place:".length);
        const location = current.savedLocations.find((candidate) => locationKey(candidate) === key);
        if (location && !current.node) void current.turnOnLocation(location);
        else {
          // Something is running: a new place is a Move, which has a sheet to decide in.
          void showWindow();
          current.router.push(VIEW_ROUTE.map);
        }
      }
    }).then((stop) => (disposed ? stop() : stops.push(stop)));

    void listen("tray-quit", async () => {
      // A power-off still in its undo window must reach the provider before the app goes.
      await latest.current.flushRemoval();
      await invoke("quit_app");
    }).then((stop) => (disposed ? stop() : stops.push(stop)));

    return () => {
      disposed = true;
      stops.forEach((stop) => stop());
    };
  }, []);

  return null;
}
