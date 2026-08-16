"use client";

import { useEffect, useState, type ReactNode } from "react";
import { formatElapsed } from "@/lib/connection-display";
import type { CommandErrorKind, ProviderPlanPricing, VpnNodeRecord } from "@/lib/tauri";

// Re-exported for views that import their types from this module; `ProviderRecord` and
// `TunnelRecord` are not referenced here, so they are re-exported without being imported.
export type { ProviderPlanPricing, ProviderRecord, TunnelRecord, VpnNodeRecord } from "@/lib/tauri";

export type ToastKind = "info" | "success" | "warning" | "error";
export type AppView = "dashboard" | "nodes" | "health" | "activity" | "settings";
export type DashboardBusyAction =
  | "refresh"
  // Replaces "save-wireguard-mode", left behind when the admin/user connection-mode toggle was
  // removed in migration 0014 and referenced by nothing since.
  | "save-tunnel-settings"
  | "save-token"
  | "save-docker-token"
  | "validate-token"
  | "validate-docker"
  | "suspend-provider"
  | "resume-provider"
  | "delete-provider"
  | "create-node"
  // "create-tunnel" and "connect-tunnel" are deliberately absent. A connect runs for tens of
  // seconds and leaves the user without internet, and every control in the app is disabled while
  // this slot is set — so using it for a connect froze the whole window during the one operation
  // the user most needs to understand. Connects are tracked by `ConnectAttempt` instead.
  | "disconnect-tunnel"
  | "destroy-node"
  | "force-cleanup"
  | "copy-diagnostics"
  | "export-diagnostics"
  | "refresh-health"
  | null;
export type ErrorCategory = "provider" | "database" | "wireguard" | "state" | "unknown";
export type ProviderAccessState = "missing" | "checking" | "ready" | "invalid" | "unknown";

export type PlanDisplayModel = {
  planLabel: string;
  sizeSlug: string;
  vcpus: number | null;
  memoryMb: number | null;
  diskGb: number | null;
  transferNote: string;
  transferTb: number | null;
  hourlyUsd: number | null;
  monthlyUsd: number | null;
  estimated: boolean;
  sourceTimestamp: string | null;
};

export type Toast = {
  id: string;
  kind: ToastKind;
  title: string;
  message: string;
};

export type RecoveryState = {
  kind: ErrorCategory;
  title: string;
  nextAction: string;
  message: string;
};

export const DEFAULT_PROVIDER_ID = "provider-1";
export const DEFAULT_PROVIDER_NAME = "DigitalOcean";
export const VIEW_ROUTE: Record<AppView, string> = {
  dashboard: "/dashboard",
  nodes: "/dashboard/nodes",
  health: "/dashboard/health",
  activity: "/dashboard/activity",
  settings: "/dashboard/settings",
};

export const NODE_CREATE_ROUTE = "/dashboard/nodes/create";

// `statusBadge` lived here: a stringly-typed switch over lowercased status text returning Tailwind
// classes. Every view that shows a status now derives it through a typed map instead —
// `NODE_STATUS_LABEL` in node-display, `STATUS_STYLES` in health-view, `ACTIVITY_TONE` in
// activity-display — each of which is exhaustive over a union rather than matching on free text.

/**
 * Map a backend failure kind onto the UI's display category and heading.
 *
 * This is a total switch over `CommandErrorKind`, so adding a kind on the backend surfaces here
 * as a type error rather than silently falling through to "unknown".
 *
 * It replaced a chain of `String.includes` checks against backend message text — phrases like
 * "Provider orchestration failed" and "OpenService FAILED 1060" — which meant any reworded
 * message silently downgraded a specific error to a generic one.
 */
export function categorizeError(kind: CommandErrorKind): { kind: ErrorCategory; title: string } {
  switch (kind) {
    case "provider":
      return { kind: "provider", title: "Provider issue" };
    case "persistence":
    case "missing-resource":
      return { kind: "database", title: "Local state issue" };
    case "wire-guard":
      return { kind: "wireguard", title: "WireGuard issue" };
    case "invalid-state":
      return { kind: "state", title: "Lifecycle issue" };
    case "secret-storage":
      return { kind: "provider", title: "Credential storage issue" };
    case "dependency":
      return { kind: "wireguard", title: "Missing prerequisite" };
    case "invalid-input":
      return { kind: "state", title: "Invalid request" };
    case "unknown":
      return { kind: "unknown", title: "Operation failed" };
  }
}

export function formatBytes(value: number | null | undefined): string {
  if (value === null || value === undefined) return "n/a";
  if (value < 1024) return `${value} B`;
  const kib = value / 1024;
  if (kib < 1024) return `${kib.toFixed(1)} KiB`;
  return `${(kib / 1024).toFixed(1)} MiB`;
}

/**
 * Handshake age, or why it is missing.
 *
 * A missing value means different things depending on the tunnel. Reading it requires `wg show`,
 * which talks to the tunnel service's restricted pipe and so needs administrator — so on a
 * *connected* tunnel the number is unavailable rather than zero, and a bare "n/a" beside a working
 * tunnel reads as a fault. On a tunnel that is not connected there is simply nothing to report.
 *
 * Throughput is unaffected either way: adapter byte counters need no elevation.
 */
export function formatHandshakeAge(
  seconds: number | null | undefined,
  tunnelStatus?: string | null,
): string {
  if (seconds === null || seconds === undefined) {
    return tunnelStatus?.toLowerCase() === "connected" ? "needs admin" : "n/a";
  }
  if (seconds === 0) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  return `${Math.floor(minutes / 60)}h ago`;
}

/**
 * Ticks once a second for as long as it is mounted.
 *
 * Mount it only where a clock is actually shown: the initial value is read on mount, so a
 * component rendered fresh when a session starts needs no resynchronisation, and nothing ticks
 * while there is nothing to count.
 *
 * @param enabled set false to hold the clock still. For a long-lived component — the app shell —
 * that shows a counter only some of the time, unmounting is not an option, so this is how it avoids
 * re-rendering every page once a second for a counter nobody is looking at.
 */
export function useNow(enabled = true): number {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!enabled) return;

    // Resynchronise on enable: while paused the value went stale, and an elapsed counter starting
    // from a stale reading shows a wrong number until the first tick. Scheduled rather than set in
    // the effect body, which would cascade a render.
    const resync = window.setTimeout(() => setNow(Date.now()), 0);
    const timer = window.setInterval(() => setNow(Date.now()), 1000);

    return () => {
      window.clearTimeout(resync);
      window.clearInterval(timer);
    };
  }, [enabled]);

  return now;
}

/** Live elapsed session time. Mounted only when a start time is known. */
export function ConnectionTimer({ startedAt }: { startedAt: string }) {
  const now = useNow();
  return <>{formatElapsed(startedAt, now) ?? "--:--:--"}</>;
}

export function formatCurrency(value: number | null | undefined, digits = 2): string {
  if (value === null || value === undefined) return "n/a";
  return `$${value.toFixed(digits)}`;
}

export function formatMemory(memoryMb: number | null | undefined): string {
  if (memoryMb === null || memoryMb === undefined) return "n/a";
  if (memoryMb >= 1024) return `${(memoryMb / 1024).toFixed(memoryMb % 1024 === 0 ? 0 : 1)} GB`;
  return `${memoryMb} MB`;
}

export function formatCreatedTime(value: string | null | undefined): string {
  if (!value) return "n/a";
  const timestamp = Date.parse(value);
  if (Number.isNaN(timestamp)) return value;
  return new Date(timestamp).toLocaleString();
}

export function planDisplayFromPricing(plan: ProviderPlanPricing): PlanDisplayModel {
  return {
    planLabel: plan.plan_label,
    sizeSlug: plan.size_slug,
    vcpus: plan.vcpus,
    memoryMb: plan.memory_mb,
    diskGb: plan.disk_gb,
    transferNote: plan.transfer_note,
    transferTb: plan.transfer_tb,
    hourlyUsd: plan.hourly_usd,
    monthlyUsd: plan.monthly_usd,
    estimated: plan.estimated,
    sourceTimestamp: plan.source_timestamp,
  };
}

export function providerAccessBadge(state: ProviderAccessState): { label: string; classes: string } {
  if (state === "ready") {
    return { label: "Ready", classes: "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" };
  }
  if (state === "checking") {
    return { label: "Checking", classes: "border-[color:rgba(171,199,255,0.25)] bg-[color:rgba(171,199,255,0.1)] text-[var(--primary)]" };
  }
  if (state === "invalid") {
    return { label: "Needs attention", classes: "border-[color:rgba(255,107,107,0.25)] bg-[color:rgba(255,107,107,0.1)] text-[#ffd5d5]" };
  }
  if (state === "missing") {
    return { label: "Missing", classes: "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]" };
  }
  return { label: "Unknown", classes: "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)]" };
}

export function AppIcon({ name, className = "h-4 w-4" }: { name: string; className?: string }) {
  const stroke = "fill-none stroke-current stroke-[1.8]";
  switch (name) {
    case "shield":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><path d="M12 3 19 6v6c0 4.5-2.7 8.2-7 10-4.3-1.8-7-5.5-7-10V6l7-3Z" className={stroke} /></svg>;
    case "grid":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><rect x="3" y="3" width="7" height="7" rx="1.6" className={stroke} /><rect x="14" y="3" width="7" height="7" rx="1.6" className={stroke} /><rect x="3" y="14" width="7" height="7" rx="1.6" className={stroke} /><rect x="14" y="14" width="7" height="7" rx="1.6" className={stroke} /></svg>;
    case "server":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><rect x="3" y="4" width="18" height="6" rx="1.8" className={stroke} /><rect x="3" y="14" width="18" height="6" rx="1.8" className={stroke} /></svg>;
    case "support":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><circle cx="12" cy="12" r="9" className={stroke} /><path d="M9.5 9a2.7 2.7 0 1 1 4 2.4c-.8.4-1.5 1-1.5 2.1v.4" className={stroke} /><circle cx="12" cy="17" r="1" className="fill-current stroke-none" /></svg>;
    case "bell":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><path d="M6 16h12l-1.2-1.5V11a4.8 4.8 0 0 0-9.6 0v3.5L6 16Z" className={stroke} /><path d="M10 18a2 2 0 0 0 4 0" className={stroke} /></svg>;
    case "refresh":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><path d="M20 12a8 8 0 0 1-14 5" className={stroke} /><path d="M6 17V13H2" className={stroke} /><path d="M4 12a8 8 0 0 1 14-5" className={stroke} /><path d="M18 7V11h4" className={stroke} /></svg>;
    case "power":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><path d="M12 3v8" className={stroke} /><path d="M7.2 6.3a8 8 0 1 0 9.6 0" className={stroke} /></svg>;
    case "copy":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><rect x="9" y="9" width="10" height="11" rx="2" className={stroke} /><path d="M7 15H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h7a2 2 0 0 1 2 2v1" className={stroke} /></svg>;
    case "lock":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><rect x="5" y="11" width="14" height="9" rx="2" className={stroke} /><path d="M8 11V8a4 4 0 0 1 8 0v3" className={stroke} /></svg>;
    case "check":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><path d="m6.5 12 3.5 3.5L17.5 8" className={stroke} /><circle cx="12" cy="12" r="8.5" className={stroke} /></svg>;
    case "database":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><ellipse cx="12" cy="6" rx="7" ry="3" className={stroke} /><path d="M5 6v8c0 1.7 3.1 3 7 3s7-1.3 7-3V6" className={stroke} /></svg>;
    case "activity":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><path d="M4 12h4l2-6 4 12 2-6h4" className={stroke} /></svg>;
    case "search":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><circle cx="11" cy="11" r="6.5" className={stroke} /><path d="m16 16 4 4" className={stroke} /></svg>;
    case "user":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><circle cx="12" cy="8" r="4" className={stroke} /><path d="M4 20c1.5-4 5-6 8-6s6.5 2 8 6" className={stroke} /></svg>;
    case "settings":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><circle cx="12" cy="12" r="3" className={stroke} /><path d="M19.4 15a7.8 7.8 0 0 0 .1-2l2-1.1-2-3.5-2.2.6a7.8 7.8 0 0 0-1.7-1l-.3-2.3h-4l-.3 2.3a7.8 7.8 0 0 0-1.7 1L7 8.4 5 11.9 7 13a7.8 7.8 0 0 0 .1 2L5 16.1l2 3.5 2.2-.6a7.8 7.8 0 0 0 1.7 1l.3 2.3h4l.3-2.3a7.8 7.8 0 0 0 1.7-1l2.2.6 2-3.5L19.4 15Z" className={stroke} /></svg>;
    // Points down by default; rotate with a class to point up for an expanded disclosure.
    case "chevron":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><path d="m7 10 5 5 5-5" className={stroke} /></svg>;
    case "cloud":
      return <svg viewBox="0 0 24 24" className={className} aria-hidden="true"><path d="M8.5 18.5h8.8a3.7 3.7 0 0 0 .6-7.4A5.7 5.7 0 0 0 6.9 9.2a4 4 0 0 0 1.6 9.3Z" className={stroke} /></svg>;
    default:
      return null;
  }
}

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    <label className="block space-y-2 text-sm text-[var(--on-surface-variant)]">
      <span className="text-[11px] uppercase tracking-[0.24em]">{label}</span>
      {children}
      {hint ? <span className="block text-xs text-[var(--on-surface-variant)]/80">{hint}</span> : null}
    </label>
  );
}

export function Button({ children, onClick, disabled, tone = "primary", className = "" }: { children: ReactNode; onClick?: () => void; disabled?: boolean; tone?: "primary" | "secondary" | "danger"; className?: string }) {
  const toneClasses = {
    primary: "border-[color:rgba(171,199,255,0.25)] bg-[var(--primary)] text-[#10131b] hover:brightness-105 disabled:opacity-50",
    secondary: "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface)] hover:bg-[color:rgba(255,255,255,0.06)] disabled:opacity-50",
    danger: "border-[color:rgba(255,107,107,0.22)] bg-[color:rgba(255,107,107,0.12)] text-[#ffd5d5] hover:bg-[color:rgba(255,107,107,0.18)] disabled:opacity-50",
  };

  return <button type="button" onClick={onClick} disabled={disabled} className={`inline-flex items-center justify-center gap-2 rounded-xl border px-4 py-2.5 text-sm font-medium transition-colors disabled:cursor-not-allowed ${toneClasses[tone]} ${className}`}>{children}</button>;
}

export function MetricCard({ title, value, detail, tone = "primary" }: { title: string; value: string; detail: string; tone?: "primary" | "secondary" | "tertiary" }) {
  const accents = {
    primary: "from-[rgba(171,199,255,0.18)]",
    secondary: "from-[rgba(78,222,163,0.18)]",
    tertiary: "from-[rgba(255,185,95,0.18)]",
  };

  return (
    <div className="relative overflow-hidden rounded-[20px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(28,32,39,0.96),rgba(17,20,28,0.98))] p-4 shadow-[var(--shadow-elevated)]">
      <div className={`absolute inset-x-0 top-0 h-[2px] bg-gradient-to-r ${accents[tone]} to-transparent`} />
      <p className="text-[11px] uppercase tracking-[0.28em] text-[var(--on-surface-variant)]">{title}</p>
      <div className="mt-3 text-2xl font-semibold tracking-tight text-[var(--on-surface)]">{value}</div>
      <p className="mt-2 text-sm text-[var(--on-surface-variant)]">{detail}</p>
    </div>
  );
}

export function Card({ title, description, children, action }: { title: string; description?: string; children: ReactNode; action?: ReactNode }) {
  return (
    <section className="rounded-[24px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(28,32,39,0.96),rgba(16,19,27,0.96))] shadow-[var(--shadow-elevated)]">
      <div className="flex items-start justify-between gap-4 border-b border-[color:var(--outline-variant)] px-5 py-4">
        <div>
          <h2 className="text-lg font-semibold tracking-tight text-[var(--on-surface)]">{title}</h2>
          {description ? <p className="mt-1 text-sm text-[var(--on-surface-variant)]">{description}</p> : null}
        </div>
        {action}
      </div>
      <div className="p-5">{children}</div>
    </section>
  );
}

export function PageChrome({ children }: { children: ReactNode }) {
  return <div className="space-y-6">{children}</div>;
}

export function getNodePricingDisplay(node: VpnNodeRecord, pricingByRegion: Record<string, ProviderPlanPricing[]>) {
  const regionPlans = pricingByRegion[node.region] ?? [];
  const matched = regionPlans.find((plan) => plan.plan_key === node.plan_key || plan.size_slug === node.size_slug);
  if (matched) return planDisplayFromPricing(matched);

  if (!node.plan_key && !node.size_slug && node.price_hourly_usd === null && node.price_monthly_usd === null) {
    return null;
  }

  return {
    planLabel: node.plan_key ?? node.size_slug ?? "Unknown plan",
    sizeSlug: node.size_slug ?? node.plan_key ?? "n/a",
    vcpus: null,
    memoryMb: null,
    diskGb: null,
    transferNote: "Transfer estimate unavailable for this node snapshot.",
    transferTb: null,
    hourlyUsd: node.price_hourly_usd,
    monthlyUsd: node.price_monthly_usd,
    estimated: true,
    sourceTimestamp: node.price_source_timestamp,
  };
}

export type HealthPanelData = Awaited<ReturnType<typeof import("@/lib/tauri").getHealthPanelData>>;
export type DiagnosticsBundle = Awaited<ReturnType<typeof import("@/lib/tauri").getDiagnosticsBundle>>;
