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
  unverified: "Not verified",
  invalid: "Key rejected",
  suspended: "Suspended",
};

/** Provider access states the dashboard tracks, narrowed to what this page needs. */
export type ProviderAccess = "ready" | "missing" | "invalid" | "checking" | "unknown";

/**
 * What a provider row should report.
 *
 * `providerAccess` is the result of the last validation and only describes the *selected* provider,
 * so it is applied only to that row — the previous table could not show a rejected key at all and
 * rendered "Installed / Active" for one.
 *
 * Suspension outranks verification: a suspended provider will refuse operations regardless of
 * whether its key is good, and that is the more actionable fact.
 */
export function deriveProviderKeyStatus(input: {
  provider: ProviderRecord;
  hasKey: boolean;
  isSelected: boolean;
  providerAccess: ProviderAccess;
}): ProviderKeyStatus {
  const { provider, hasKey, isSelected, providerAccess } = input;

  if (!hasKey) return "missing";
  if (provider.status === "suspended") return "suspended";

  if (isSelected) {
    if (providerAccess === "invalid") return "invalid";
    if (providerAccess === "ready") return "verified";
  }

  return "unverified";
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
 * droplets the app could then never destroy, leaving the user paying for them.
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
