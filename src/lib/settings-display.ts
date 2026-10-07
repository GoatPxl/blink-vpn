/**
 * Pure presentation and validation logic for the Settings page.
 *
 * Free of JSX and React so it can be unit tested directly.
 */

import type { ProviderRecord, VpnNodeRecord } from "./tauri";

export type ProviderKeyStatus = "missing" | "verified" | "unverified" | "invalid" | "suspended";

export const PROVIDER_KEY_STATUS_LABEL: Record<ProviderKeyStatus, string> = {
  missing: "No key",
  verified: "Verified",
  // "Not checked", not "Not verified": nothing has failed, the key simply has not been tested
  // since the app started.
  unverified: "Not checked",
  invalid: "Key rejected",
  suspended: "Suspended",
};

/** Provider access states the dashboard tracks, narrowed to what this page needs. */
export type ProviderAccess = "ready" | "missing" | "invalid" | "checking" | "unknown";

/**
 * What a provider row should report.
 *
 * Read from the provider's **own** persisted result. It has to be per-provider: a single shared
 * value described whichever provider was selected, so checking one row moved the "Verified" badge
 * off the other instead of setting its own. It is persisted so the verdict survives a restart
 * rather than resetting every key to unchecked on each launch.
 *
 * Suspension outranks verification: a suspended provider refuses operations regardless of whether
 * its key is good, and that is the more actionable fact.
 */
export function deriveProviderKeyStatus(input: {
  provider: ProviderRecord;
  hasKey: boolean;
}): ProviderKeyStatus {
  const { provider, hasKey } = input;

  if (!hasKey) return "missing";
  if (provider.status === "suspended") return "suspended";
  if (provider.last_validation_status === "failed") return "invalid";
  if (provider.last_validation_status === "ok") return "verified";

  return "unverified";
}

/**
 * How long ago a key was checked, for the row's secondary line.
 *
 * A verdict without its age would keep asserting a check that might be weeks old and since
 * revoked; showing when it happened lets the reader judge how much it is worth.
 */
export function describeKeyState(provider: ProviderRecord, now: number): string {
  if (!provider.token_ref) return "No key stored";
  if (!provider.last_validated_at) return "Key stored · not checked yet";

  const checkedAt = Date.parse(provider.last_validated_at);
  if (Number.isNaN(checkedAt)) return "Key stored on this device";

  const seconds = Math.max(0, Math.floor((now - checkedAt) / 1000));
  if (seconds < 60) return "Checked just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `Checked ${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `Checked ${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `Checked ${days}d ago`;
}

export type RowAction = {
  id: "edit" | "suspend" | "delete";
  label: string;
  disabled: boolean;
  /** Why the action is unavailable, for a tooltip. Null when it is available. */
  disabledReason: string | null;
  danger: boolean;
};

/**
 * The `⋯` menu for one provider row.
 *
 * The delete guard mirrors a real backend invariant: removing the credential would orphan live
 * servers the app could then never destroy, leaving the user paying for them.
 */
export function buildRowActions(input: {
  provider: ProviderRecord;
  hasKey: boolean;
  attachedNodeCount: number;
  busy: boolean;
}): RowAction[] {
  const { provider, hasKey, attachedNodeCount, busy } = input;
  const suspended = provider.status === "suspended";

  const deleteBlockedReason = !hasKey
    ? "No key is stored for this provider."
    : attachedNodeCount > 0
      ? `Destroy the ${attachedNodeCount} attached ${attachedNodeCount === 1 ? "node" : "nodes"} first — without this key the app could not delete them.`
      : null;

  return [
    {
      id: "edit",
      label: hasKey ? "Replace key" : "Add key",
      disabled: busy,
      disabledReason: null,
      danger: false,
    },
    {
      id: "suspend",
      label: suspended ? "Resume provider" : "Suspend provider",
      disabled: busy,
      disabledReason: null,
      danger: false,
    },
    {
      id: "delete",
      // Named for what it does: the provider row survives, only the credential goes.
      label: "Remove key",
      disabled: busy || deleteBlockedReason !== null,
      disabledReason: deleteBlockedReason,
      danger: true,
    },
  ];
}

/** How many nodes a provider is responsible for. */
export function countAttachedNodes(nodes: VpnNodeRecord[], providerId: string): number {
  return nodes.filter((node) => node.provider_id === providerId).length;
}

export type SettingsValidation = { valid: true } | { valid: false; message: string };

/**
 * Validates a DNS entry before it is sent.
 *
 * Empty is legitimate and means "no DNS line", leaving the system resolver in place — distinct
 * from an invalid entry. The backend validates again; this exists so the user is told immediately
 * rather than after a round trip.
 */
export function validateDnsServer(value: string): SettingsValidation {
  const trimmed = value.trim();
  if (trimmed === "") return { valid: true };

  const ipv4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(trimmed);
  if (ipv4) {
    const octetsValid = ipv4.slice(1).every((part) => {
      const numeric = Number(part);
      // Reject leading zeros, which some resolvers read as octal.
      return numeric <= 255 && String(numeric) === part;
    });
    return octetsValid ? { valid: true } : { valid: false, message: "Each part must be 0–255." };
  }

  // Deliberately permissive on IPv6: the backend parses it authoritatively, and a strict regex
  // here would reject valid compressed forms.
  if (trimmed.includes(":")) return { valid: true };

  return { valid: false, message: "Enter an IP address, or leave empty to use your system DNS." };
}

export function validateKeepalive(value: string): SettingsValidation {
  const trimmed = value.trim();
  if (trimmed === "") return { valid: false, message: "Enter a number of seconds, or 0 to disable." };

  if (!/^\d+$/.test(trimmed)) return { valid: false, message: "Enter a whole number of seconds." };

  const numeric = Number(trimmed);
  if (numeric > 65535) return { valid: false, message: "Must be 65535 or less." };

  return { valid: true };
}

/** Plain-language note on what a keepalive value does, so the field is not a bare number. */
export function describeKeepalive(seconds: number): string {
  if (seconds === 0) {
    return "Disabled. The tunnel may go quiet behind a router that drops idle connections.";
  }
  return `Sends a packet every ${seconds}s to keep the tunnel reachable through NAT.`;
}

/** A server has five peer slots and slot 0 is this PC. Mirrors `MAX_MOBILE_DEVICES` in Rust. */
export const MAX_MOBILE_DEVICES = 4;

/** Mirrors `MAX_PEER_LABEL_CHARS` in Rust. */
export const MAX_DEVICE_NAME_CHARS = 40;

/**
 * Validates the saved device list before it is sent, with the same rules the backend applies:
 * it fits in the spare slots, every name is non-blank and short enough, and no two names match
 * ignoring case — identical names could not be told apart on the dashboard.
 */
export function validateMobileDevices(names: readonly string[]): SettingsValidation {
  if (names.length > MAX_MOBILE_DEVICES) {
    return { valid: false, message: `A server has room for ${MAX_MOBILE_DEVICES} devices besides this PC.` };
  }

  const seen = new Set<string>();
  for (const raw of names) {
    const name = raw.trim();
    if (name === "") {
      return { valid: false, message: "Give every device a name, or remove the empty row." };
    }
    if ([...name].length > MAX_DEVICE_NAME_CHARS) {
      return { valid: false, message: `A device name can be at most ${MAX_DEVICE_NAME_CHARS} characters.` };
    }
    const key = name.toLowerCase();
    if (seen.has(key)) {
      return { valid: false, message: `"${name}" is in the list twice. Give each device its own name.` };
    }
    seen.add(key);
  }

  return { valid: true };
}
