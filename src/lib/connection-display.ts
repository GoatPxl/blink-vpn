/**
 * Pure presentation logic for live connection state.
 *
 * Kept free of JSX and React so it can be unit tested directly. Every function here has one job:
 * turn a measurement — or its absence — into something the UI can state truthfully.
 */

/**
 * Elapsed session time as `HH:MM:SS`, with a `Nd ` prefix past a day.
 *
 * Returns null for an unknown or unusable start time, so callers render an explicit "unknown"
 * rather than a zeroed clock. A tunnel that was already running when the app started has no
 * knowable start, and `00:00:00` would assert one.
 */
export function formatElapsed(startedAt: string | null | undefined, now: number): string | null {
  if (!startedAt) return null;

  const started = Date.parse(startedAt);
  if (Number.isNaN(started)) return null;

  const totalSeconds = Math.floor((now - started) / 1000);
  if (totalSeconds < 0) return null;

  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3600) % 24;
  const days = Math.floor(totalSeconds / 86400);

  const clock = [hours, minutes, seconds].map((part) => String(part).padStart(2, "0")).join(":");
  return days > 0 ? `${days}d ${clock}` : clock;
}

export type EgressTone = "positive" | "warning" | "muted";

export type EgressDisplay = {
  value: string;
  caption: string;
  tone: EgressTone;
};

/**
 * What the "My IP Address" card should say about IPv4.
 *
 * The address is measured in every state, so the user watches it change from their ISP's address
 * to the server's as the tunnel comes up — that change is the proof the tunnel carries traffic.
 * Comparing it against `serverIp` is what turns the display into an actual check.
 *
 * The backend previously substituted the server's own address whenever the probe was skipped or
 * failed, which made "verified" and "never checked" render identically. It now sends null, so
 * "could not measure" stays distinct from a real reading.
 */
export function describeEgress(input: {
  tunnelConnected: boolean;
  observedEgress: string | null;
  serverIp: string | null;
}): EgressDisplay {
  const { tunnelConnected, observedEgress, serverIp } = input;

  if (observedEgress === null) {
    return {
      value: "n/a",
      caption: "Could not determine your IP address",
      tone: "muted",
    };
  }

  if (!tunnelConnected) {
    return {
      value: observedEgress,
      caption: "Not protected — this is your real address",
      tone: "muted",
    };
  }

  if (serverIp !== null && observedEgress !== serverIp) {
    // Connected, yet traffic is reaching the internet by some route other than the tunnel.
    return {
      value: observedEgress,
      caption: "Traffic is not exiting through your server",
      tone: "warning",
    };
  }

  return {
    value: observedEgress,
    caption: "Protected — traffic exits through your server",
    tone: "positive",
  };
}

export type Ipv6Display = {
  value: string;
  caption: string;
  tone: EgressTone;
};

/**
 * What the card should say about IPv6, if anything.
 *
 * This deployment's tunnel is IPv4-only: the server bootstrap configures `iptables` MASQUERADE and
 * IPv4 forwarding only, and the client interface is given a `/32` with no IPv6 address. So a
 * reachable IPv6 address while connected is not a second protected route — it is traffic leaving
 * outside the tunnel, and the one thing worth saying about IPv6 here.
 *
 * Returns null when there is nothing to report, so the row disappears rather than showing "none".
 */
export function describeIpv6(input: {
  tunnelConnected: boolean;
  observedIpv6: string | null;
}): Ipv6Display | null {
  const { tunnelConnected, observedIpv6 } = input;

  if (!tunnelConnected) {
    // Disconnected, IPv6 is simply the other address the user has. Nothing is wrong with it.
    return observedIpv6 === null
      ? null
      : { value: observedIpv6, caption: "Your real IPv6 address", tone: "muted" };
  }

  if (observedIpv6 === null) {
    return { value: "None detected", caption: "No IPv6 leak detected", tone: "positive" };
  }

  return {
    value: observedIpv6,
    caption: "IPv6 is bypassing the tunnel",
    tone: "warning",
  };
}

