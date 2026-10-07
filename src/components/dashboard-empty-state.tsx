"use client";

import Link from "next/link";

import { SavedLocationList } from "@/components/dashboard/saved-locations-view";
import type { DashboardBusyAction } from "@/components/dashboard/shared";
import type { SavedLocationRecord } from "@/lib/tauri";

/**
 * The dashboard with no server running.
 *
 * Servers are meant to be turned off, so "no server" is the normal resting state, not a first run.
 * When the user has saved locations, this offers them back; only with none is it a setup prompt.
 */
export function DashboardEmptyState({
  savedLocations = [],
  busyAction = null,
  onTurnOn,
}: {
  savedLocations?: SavedLocationRecord[];
  busyAction?: DashboardBusyAction;
  onTurnOn?: (location: SavedLocationRecord) => Promise<void>;
}) {
  if (savedLocations.length > 0 && onTurnOn) {
    return (
      <div className="flex min-h-[58vh] items-center justify-center">
        <div className="w-full max-w-2xl rounded-[24px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(28,32,39,0.96),rgba(16,19,27,0.96))] p-8 shadow-[var(--shadow-elevated)]">
          <h2 className="text-2xl font-semibold tracking-tight text-[var(--on-surface)]">No server is on</h2>
          <p className="mt-2 text-sm leading-6 text-[var(--on-surface-variant)]">
            Turn one of your saved locations back on. It takes a few minutes and billing starts when the
            server is created.
          </p>
          <div className="mt-5">
            <SavedLocationList locations={savedLocations} busyAction={busyAction} onTurnOn={onTurnOn} />
          </div>
          <div className="mt-5 text-right">
            <Link href="/dashboard/nodes/create" className="text-sm text-[var(--primary)] hover:underline">
              Or create a server somewhere new
            </Link>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-[58vh] items-center justify-center">
      <div className="w-full max-w-xl rounded-[24px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(28,32,39,0.96),rgba(16,19,27,0.96))] p-8 text-center shadow-[var(--shadow-elevated)]">
        <h2 className="text-2xl font-semibold tracking-tight text-[var(--on-surface)]">No VPN server configured yet</h2>
        <p className="mt-3 text-sm leading-6 text-[var(--on-surface-variant)]">
          Complete the guided setup to verify WireGuard readiness, save your provider token, and provision your first VPN node.
        </p>
        <div className="mt-7">
          <Link
            href="/onboarding"
            className="inline-flex items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[var(--primary)] px-5 py-2.5 text-sm font-medium text-[#10131b] transition-colors hover:brightness-105"
          >
            Get Started
          </Link>
        </div>
      </div>
    </div>
  );
}
