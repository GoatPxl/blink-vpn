"use client";

import { getCurrentWindow } from "@tauri-apps/api/window";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { useEffect, useRef } from "react";

import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { costSoFarUsd, formatUsd } from "@/lib/home-display";

type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

/** A server left on and disconnected this long gets one reminder that it is still billing. */
const LONG_RUNNING_MS = 12 * 60 * 60 * 1000;

/**
 * Native toasts for what happens while the window is hidden in the tray.
 *
 * One per event, and only when the window is not showing — in the window the same events already
 * have a place on screen. Never for routine success. The toasts carry no buttons: the plugin offers
 * actions on mobile only, so the next step is named in the text and is one click away in the tray.
 */
async function notifyIfHidden(title: string, body: string) {
  if (typeof window === "undefined" || !(window as TauriWindow).__TAURI_INTERNALS__) return;
  try {
    if (await getCurrentWindow().isVisible()) return;
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (granted) sendNotification({ title, body });
  } catch (error) {
    console.warn("notification failed", error);
  }
}

export function NotificationBridge() {
  const { health, nodes, operationAttempt, providers, tunnelPhase } = useDashboard();

  // Last seen values, so each notification fires on a change, not on every render.
  const seenDrop = useRef<number | null>(null);
  const lastPhase = useRef(tunnelPhase);
  const building = useRef<{ nodeId: string; place: string } | null>(null);
  const keyStatus = useRef(new Map<string, string | null>());
  const remindedLongRunning = useRef(new Set<string>());

  // The tunnel stopped on its own: traffic is on the normal connection.
  useEffect(() => {
    const drop = health?.unexpected_drop?.observed_at_epoch_ms ?? null;
    if (drop !== null && drop !== seenDrop.current) {
      seenDrop.current = drop;
      void notifyIfHidden("Tunnel dropped", "Your traffic is using your normal connection. Reconnect from the Blink tray icon.");
    }
  }, [health?.unexpected_drop?.observed_at_epoch_ms]);

  // A working tunnel whose server stopped answering.
  useEffect(() => {
    const previous = lastPhase.current;
    lastPhase.current = tunnelPhase;
    if (previous === "connected" && tunnelPhase === "degraded") {
      void notifyIfHidden("No reply from your server", "Traffic may be stalled. Usually the network hiccuped; reconnect from the tray if it persists.");
    }
  }, [tunnelPhase]);

  // A build finished while nobody was looking.
  useEffect(() => {
    if (operationAttempt?.kind === "provision") {
      building.current = { nodeId: operationAttempt.nodeId, place: operationAttempt.place };
      return;
    }
    const finished = building.current;
    building.current = null;
    if (!finished) return;
    const node = nodes.find((candidate) => candidate.id === finished.nodeId);
    if (node && (node.lifecycle_status === "ready" || node.lifecycle_status === "connected")) {
      void notifyIfHidden(`${finished.place} is ready`, "Connecting now. Windows will ask for permission.");
    }
  }, [nodes, operationAttempt]);

  // A key that checked out before and now fails.
  useEffect(() => {
    for (const provider of providers) {
      const previous = keyStatus.current.get(provider.id);
      keyStatus.current.set(provider.id, provider.last_validation_status);
      if (previous === "ok" && provider.last_validation_status === "failed") {
        void notifyIfHidden(
          `Your ${provider.name} key stopped working`,
          "Nothing changes right now, but you cannot build or power off there until it is replaced in Setup.",
        );
      }
    }
  }, [providers]);

  // A server on for twelve hours and not connected is probably forgotten, and still billing.
  useEffect(() => {
    const check = () => {
      const now = Date.now();
      for (const node of nodes) {
        if (node.lifecycle_status !== "ready" || remindedLongRunning.current.has(node.id)) continue;
        if (now - Date.parse(node.created_at) < LONG_RUNNING_MS) continue;
        remindedLongRunning.current.add(node.id);
        const place = node.region_geo?.city ?? node.name;
        void notifyIfHidden(
          `${place} has been on for 12 h · ${formatUsd(costSoFarUsd(node, now))}`,
          "It is disconnected but still billing. Power it off from the tray if you are done.",
        );
      }
    };
    check();
    const timer = window.setInterval(check, 10 * 60 * 1000);
    return () => window.clearInterval(timer);
  }, [nodes]);

  return null;
}
