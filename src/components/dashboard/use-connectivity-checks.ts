"use client";

import { useMemo } from "react";
import { buildConnectivityChecks, type ConnectivityCheck, type ProviderAccess } from "@/lib/health-display";
import { useDashboard } from "./dashboard-provider";

/**
 * The connectivity checks, computed once from dashboard state.
 *
 * Home shows the passing count and Shield the full list; both read this so the count on Home can
 * never disagree with the list it links to.
 */
export function useConnectivityChecks(): ConnectivityCheck[] {
  const { health, providerAccessState, tunnelConnected, wireguardReadiness } = useDashboard();
  return useMemo(
    () => buildConnectivityChecks({
      readiness: wireguardReadiness,
      providerAccess: providerAccessState as ProviderAccess,
      health,
      tunnelConnected,
    }),
    [health, providerAccessState, tunnelConnected, wireguardReadiness],
  );
}
