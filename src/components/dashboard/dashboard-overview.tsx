"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import { PixelIcon } from "@/components/ui/pixel-icon";
import { cityLine, costSoFarUsd, formatClock, formatHourly, formatUsd } from "@/lib/home-display";
import type { ConnectivityCheck } from "@/lib/health-display";
import type { ConnectAttempt } from "./dashboard-provider";
import { HeroStat, HomeHero, type HeroTone } from "./home-hero";
import {
  Button,
  ConnectionTimer,
  GiantAction,
  Panel,
  useNow,
  VIEW_ROUTE,
  type DashboardBusyAction,
  type HealthPanelData,
  type TunnelRecord,
  type VpnNodeRecord,
} from "./shared";

/**
 * Home while a server exists: secure, coming up, or running but not connected.
 *
 * One primary action, always the next step. Throughput, cipher and handshake live under Shield;
 * this screen keeps only what the user acts on and what it is costing them.
 */
export function DashboardOverview({
  activeNode,
  connectedTunnel,
  health,
  tunnelConnected,
  busyAction,
  connectAttempt,
  checks,
  onConnectPrimary,
  onConnectTunnel,
  onDisconnectTunnel,
  onForceDisconnectNodeService,
  onPowerOff,
  devices,
}: {
  activeNode: VpnNodeRecord;
  connectedTunnel: TunnelRecord | null;
  health: HealthPanelData | null;
  tunnelConnected: boolean;
  busyAction: DashboardBusyAction;
  connectAttempt: ConnectAttempt | null;
  checks: ConnectivityCheck[];
  onConnectPrimary: () => Promise<void>;
  onConnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onDisconnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  onForceDisconnectNodeService: (node: VpnNodeRecord) => Promise<void>;
  onPowerOff: (node: VpnNodeRecord) => Promise<void>;
  /** The phones card, beside "Done for the day?". */
  devices?: ReactNode;
}) {
  const now = useNow();
  const connecting = connectAttempt !== null;
  const tone: HeroTone = connecting ? "busy" : tunnelConnected ? "secure" : "off";

  // Known only when this app established the session. A tunnel already running at startup has no
  // recorded start, and inventing one would render a timer counting from an arbitrary moment.
  const sessionStartedAt = tunnelConnected ? health?.connected_at ?? null : null;
  const since = formatClock(sessionStartedAt);

  const tag = connecting
    ? "Connecting · internet pauses until the handshake"
    : tunnelConnected
      ? since ? `Secure since ${since}` : "Secure"
      : "Server on · not connected · billing";
  const headline = connecting ? "Hold on" : tunnelConnected ? "Secure" : "Ready";

  const cost = costSoFarUsd(activeNode, now);
  const hourly = formatHourly(activeNode.price_hourly_usd);
  const passing = checks.filter((check) => check.status === "pass").length;
  const busy = busyAction !== null || connecting;

  function onPrimary() {
    if (tunnelConnected) {
      // The label says Disconnect, so the click must never connect. With no tunnel record loaded
      // yet, the node-wide stop is the only handle available.
      void (connectedTunnel ? onDisconnectTunnel(connectedTunnel) : onForceDisconnectNodeService(activeNode));
      return;
    }
    if (connectedTunnel) {
      void onConnectTunnel(connectedTunnel);
      return;
    }
    void onConnectPrimary();
  }

  return (
    <>
      <HomeHero
        tone={tone}
        tag={tag}
        headline={headline}
        subline={
          <span className="flex flex-wrap items-baseline gap-x-5 gap-y-2">
            <span>{cityLine(activeNode.region_geo, activeNode.region)}</span>
            <Link
              href={`${VIEW_ROUTE.map}?move=1`}
              className="inline-flex items-center gap-1 border-b-2 border-lime text-[18px] font-medium text-lime"
            >
              change <PixelIcon name="arrow-down" className="h-3.5 w-3.5" />
            </Link>
          </span>
        }
        action={
          tunnelConnected ? (
            <GiantAction
              tone="destructive"
              label="Disconnect"
              icon={<PixelIcon name="power" className="h-12 w-12" />}
              onClick={onPrimary}
              disabled={busy}
            />
          ) : (
            <GiantAction
              tone="primary"
              label="Connect"
              sublabel={connecting ? "Waiting for the handshake" : "One Windows prompt"}
              icon={<PixelIcon name="power" className="h-12 w-12" />}
              onClick={onPrimary}
              disabled={busy}
            />
          )
        }
        footer={
          <div className="grid grid-cols-3 max-[900px]:grid-cols-1 max-[900px]:gap-6 max-[900px]:[&>*]:!border-l-0 max-[900px]:[&>*]:!pl-0">
            <HeroStat label="This session">
              <span className="font-mono text-[36px] font-bold leading-none text-bone tabular-nums">
                {sessionStartedAt ? <ConnectionTimer startedAt={sessionStartedAt} /> : tunnelConnected ? "—" : "--:--:--"}
              </span>
              {tunnelConnected && sessionStartedAt === null ? (
                <span className="mt-2 block text-[12.5px] text-stone">Started before Blink opened</span>
              ) : null}
            </HeroStat>
            <HeroStat label="Server cost so far">
              <span className="flex flex-wrap items-baseline gap-x-3">
                <span className="font-mono text-[36px] font-bold leading-none text-bone tabular-nums">{formatUsd(cost)}</span>
                {hourly ? <span className="font-mono text-[14px] text-stone">· {hourly}</span> : null}
              </span>
            </HeroStat>
            <HeroStat label="Checks">
              <Link
                href={VIEW_ROUTE.shield}
                className={`inline-flex items-center gap-3 text-[28px] font-semibold ${passing === checks.length ? "text-lime" : "text-ember"}`}
              >
                <span aria-hidden="true" className="inline-block h-5 w-5 bg-current" />
                <span>{passing} / {checks.length} passing</span>
                <PixelIcon name="arrow-right" className="h-6 w-6" />
              </Link>
            </HeroStat>
          </div>
        }
      />

      <div className="grid grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)] gap-5 max-[1000px]:grid-cols-1">
        {devices ?? <div />}
        <Panel bodyClassName="flex items-center gap-6 px-8 py-5">
          <div className="min-w-0 flex-1">
            <div className="text-[18px] font-semibold text-bone">Done for the day?</div>
            <p className="text-[14px] text-stone">Power off deletes the server and stops the bill.</p>
          </div>
          <Button tone="ghost" onClick={() => void onPowerOff(activeNode)} disabled={busy}>
            Power off
          </Button>
        </Panel>
      </div>
    </>
  );
}
