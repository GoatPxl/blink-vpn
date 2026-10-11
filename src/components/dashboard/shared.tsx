"use client";

import { useEffect, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { formatElapsed } from "@/lib/connection-display";
import type { CommandErrorKind, ProviderPlanPricing, VpnNodeRecord } from "@/lib/tauri";

// Re-exported for views that import their types from this module; `ProviderRecord` and
// `TunnelRecord` are not referenced here, so they are re-exported without being imported.
export type { ProviderPlanPricing, ProviderRecord, TunnelRecord, VpnNodeRecord } from "@/lib/tauri";

export type ToastKind = "info" | "success" | "warning" | "error";
export type AppView = "home" | "map" | "shield" | "setup";
export type DashboardBusyAction =
  | "refresh"
  // Replaces "save-wireguard-mode", left behind when the admin/user connection-mode toggle was
  // removed in migration 0014 and referenced by nothing since.
  | "save-tunnel-settings"
  | "save-app-preferences"
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
  | "forget-location"
  | "force-cleanup"
  | "copy-diagnostics"
  | "export-diagnostics"
  | "refresh-health"
  | "claim-device"
  | "export-device-config"
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
  /** One follow-up, as a link: "Fix key" to Setup. */
  action?: { label: string; href: string };
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
  home: "/dashboard",
  map: "/dashboard/map",
  shield: "/dashboard/shield",
  setup: "/dashboard/settings",
};

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


/**
 * The one overlay in this app.
 *
 * There were two hand-rolled copies of this — the credential dialog and the node drawer — and
 * adding a third for device configs is exactly the parallel implementation AGENTS.md §5 forbids.
 * Both were folded into this on the way.
 *
 * `aria-hidden` must never sit on an element containing the dialog — that hides the dialog itself
 * from assistive technology — and Escape closes it.
 */
export function Modal({
  children,
  onClose,
  labelledBy,
  placement = "center",
  width = "md",
}: {
  children: ReactNode;
  onClose: () => void;
  /** Id of the element naming this dialog, usually its heading. */
  labelledBy: string;
  placement?: "center" | "end";
  width?: "md" | "lg";
}) {
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const isDrawer = placement === "end";

  return (
    <div
      className={`fixed inset-x-0 bottom-0 top-[var(--titlebar-h)] z-50 flex bg-[rgba(11,11,10,0.72)] backdrop-grayscale-[0.4] ${isDrawer ? "justify-end" : "items-center justify-center p-6"}`}
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy}
        className={
          isDrawer
            ? "scroll-area flex h-full w-full max-w-[480px] flex-col overflow-y-auto border-l-2 border-bone bg-panel p-6"
            : `scroll-area max-h-full w-full ${width === "lg" ? "max-w-[760px]" : "max-w-[560px]"} overflow-y-auto border-2 border-bone bg-panel p-6 shadow-hard-lg`
        }
        onClick={(event) => event.stopPropagation()}
      >
        {children}
      </div>
    </div>
  );
}

/** Silkscreen 12px uppercase label. `state` adds the `■` prefix and colours it. */
export function Tag({
  children,
  state,
  className = "",
}: {
  children: ReactNode;
  state?: "secure" | "busy" | "off" | "failed";
  className?: string;
}) {
  const color = state ? STATE_TEXT[state] : "text-stone";
  return (
    <span className={`font-display text-[12px] uppercase tracking-[0.06em] ${color} ${className}`}>
      {state ? "■ " : null}
      {children}
    </span>
  );
}

const STATE_TEXT = {
  secure: "text-lime",
  busy: "text-ember",
  off: "text-stone-2",
  failed: "text-alert",
} as const;

export const INPUT_CLASS =
  "h-12 w-full border-2 border-line-3 bg-ink px-3 font-mono text-[14px] text-bone outline-none placeholder:text-stone-2 focus:border-lime disabled:opacity-50";

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    <label className="block space-y-2">
      <Tag>{label}</Tag>
      {children}
      {hint ? <span className="block text-[12.5px] text-stone">{hint}</span> : null}
    </label>
  );
}

export type ButtonTone = "primary" | "ghost" | "quiet" | "destructive";

const BUTTON_TONE: Record<ButtonTone, string> = {
  primary: "border-lime bg-lime text-ink shadow-hard hover:bg-[#e2ff70] active:translate-x-[2px] active:translate-y-[2px] active:shadow-none",
  ghost: "border-bone bg-transparent text-bone hover:bg-panel-2",
  quiet: "border-line-3 bg-transparent text-stone hover:border-stone hover:text-bone",
  destructive: "border-alert bg-transparent text-alert hover:bg-alert-ink",
};

/**
 * Silkscreen label, 48px high, square. One primary per screen; it is always the next step.
 *
 * `label` is the accessible name for an icon-only button.
 */
export function Button({
  children,
  onClick,
  disabled,
  tone = "primary",
  size = "md",
  className = "",
  label,
  pressed,
  type = "button",
}: {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  tone?: ButtonTone;
  size?: "md" | "sm";
  className?: string;
  label?: string;
  /** Set for a toggle button, such as a filter chip. */
  pressed?: boolean;
  type?: "button" | "submit";
}) {
  const height = size === "md" ? "h-12 px-5 text-[14px]" : "h-9 px-3 text-[12px]";
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      aria-pressed={pressed}
      className={`inline-flex shrink-0 items-center justify-center gap-2 border-2 font-display uppercase tracking-[0.04em] transition-colors disabled:cursor-not-allowed disabled:opacity-40 disabled:shadow-none ${height} ${BUTTON_TONE[tone]} ${className}`}
    >
      {children}
    </button>
  );
}

/**
 * The 200×200 Disconnect / 300×200 Connect square: icon over label, optional sub-label.
 *
 * Its accessible name is `label` alone, so tests and screen readers find "Connect" and "Disconnect"
 * exactly, however much the sub-label says.
 */
export function GiantAction({
  icon,
  label,
  sublabel,
  tone,
  onClick,
  disabled,
  wide = false,
}: {
  icon: ReactNode;
  label: string;
  sublabel?: string;
  tone: "primary" | "destructive";
  onClick: () => void;
  disabled?: boolean;
  wide?: boolean;
}) {
  const toneClass = tone === "primary"
    ? "border-lime bg-lime text-ink shadow-hard hover:bg-[#e2ff70]"
    : "border-alert bg-transparent text-alert shadow-hard-alert hover:bg-alert-ink";
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      className={`flex h-[200px] shrink-0 flex-col items-center justify-center gap-4 border-2 transition-colors active:translate-x-[3px] active:translate-y-[3px] active:shadow-none disabled:cursor-not-allowed disabled:opacity-40 ${wide ? "w-[300px]" : "w-[200px]"} ${toneClass}`}
    >
      <span aria-hidden="true">{icon}</span>
      <span aria-hidden="true" className="font-display text-[20px] uppercase">{label}</span>
      {sublabel ? (
        <span aria-hidden="true" className="px-3 text-center font-display text-[11px] uppercase opacity-75">{sublabel}</span>
      ) : null}
    </button>
  );
}

/**
 * A selectable square: code, name, meta. Dashed for an "add" affordance.
 */
export function Tile({
  code,
  name,
  meta,
  onClick,
  dashed = false,
  selected = false,
  disabled,
}: {
  code: ReactNode;
  name: ReactNode;
  meta?: ReactNode;
  onClick?: () => void;
  dashed?: boolean;
  selected?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-pressed={selected}
      className={`flex min-w-0 flex-col items-start gap-1.5 border-2 bg-panel p-5 text-left transition-colors hover:border-lime disabled:cursor-not-allowed disabled:opacity-40 ${dashed ? "border-dashed border-line-3" : selected ? "border-lime" : "border-line"}`}
    >
      <span className="font-display text-[24px] uppercase leading-none text-lime">{code}</span>
      <span className="mt-1 truncate text-[15px] font-semibold text-bone">{name}</span>
      {meta ? <span className="text-[12.5px] text-stone">{meta}</span> : null}
    </button>
  );
}

/** A list row with a 2px divider. The last row in a list is borderless. */
export function Row({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div className={`flex items-center gap-4 border-b-2 border-line-2 py-3 last:border-b-0 ${className}`}>
      {children}
    </div>
  );
}

/** Real tabs: `role="tablist"`, arrow-key navigation, one tab stop. */
export function Tabs<T extends string>({
  tabs,
  selected,
  onSelect,
  label,
  idPrefix,
}: {
  tabs: Array<{ id: T; label: ReactNode }>;
  selected: T;
  onSelect: (id: T) => void;
  label: string;
  /** Tab ids are `${idPrefix}-tab-${id}`, panels `${idPrefix}-panel-${id}`. */
  idPrefix: string;
}) {
  function onKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
    event.preventDefault();
    const index = tabs.findIndex((tab) => tab.id === selected);
    const step = event.key === "ArrowRight" ? 1 : -1;
    const next = tabs[(index + step + tabs.length) % tabs.length];
    onSelect(next.id);
    document.getElementById(`${idPrefix}-tab-${next.id}`)?.focus();
  }

  return (
    <div role="tablist" aria-label={label} onKeyDown={onKeyDown} className="flex flex-wrap gap-0 border-b-2 border-lime">
      {tabs.map((tab) => {
        const active = tab.id === selected;
        return (
          <button
            key={tab.id}
            id={`${idPrefix}-tab-${tab.id}`}
            type="button"
            role="tab"
            aria-selected={active}
            aria-controls={`${idPrefix}-panel-${tab.id}`}
            tabIndex={active ? 0 : -1}
            onClick={() => onSelect(tab.id)}
            className={`-mb-[2px] h-12 border-2 border-b-0 px-5 font-display text-[13px] uppercase transition-colors ${active ? "border-lime bg-panel text-lime" : "border-transparent text-stone hover:text-bone"}`}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}

export type BlockTone = "lime" | "ember" | "alert";

const BLOCK_COLORS: Record<BlockTone, { done: string; current: string; pending: string }> = {
  lime: { done: "var(--lime)", current: "var(--lime-dim)", pending: "var(--track)" },
  ember: { done: "var(--ember)", current: "var(--ember-dim)", pending: "var(--ember-track)" },
  alert: { done: "var(--alert)", current: "var(--ember-dim)", pending: "var(--track)" },
};

/**
 * N equal blocks: done, current, pending. Never a thin bar.
 *
 * The percentage is usually an estimate (see `progress-estimate.ts`); the line beneath is what
 * makes it honest, by naming the step actually in progress.
 */
export function BlockProgress({
  percent,
  blocks = 40,
  tone = "lime",
  height = 18,
  line,
  trailing,
  testId,
}: {
  percent: number;
  blocks?: number;
  tone?: BlockTone;
  /** Block height in px, 14–26. */
  height?: number;
  /** Accessible description, and shown beneath when present. */
  line?: string;
  /** Right-aligned on the text row, typically an elapsed clock. */
  trailing?: ReactNode;
  testId?: string;
}) {
  const clamped = Math.min(100, Math.max(0, percent));
  const done = Math.floor((clamped / 100) * blocks);
  const colors = BLOCK_COLORS[tone];

  return (
    <div data-testid={testId} className="space-y-2">
      <div
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(clamped)}
        aria-valuetext={line}
        className="grid gap-[3px]"
        style={{ gridTemplateColumns: `repeat(${blocks}, minmax(0, 1fr))` }}
      >
        {Array.from({ length: blocks }, (_, index) => (
          <span
            key={index}
            style={{
              height,
              backgroundColor: index < done ? colors.done : index === done && clamped < 100 ? colors.current : colors.pending,
            }}
          />
        ))}
      </div>
      {line || trailing ? (
        <div className="flex items-center justify-between gap-3 text-[12.5px] text-stone">
          <span className="truncate">{line}</span>
          {trailing ? <span className="shrink-0 font-mono tabular-nums">{trailing}</span> : null}
        </div>
      ) : null}
    </div>
  );
}

/** An on/off switch. A labelled button with `role="switch"`, so it is one tab stop and keyboard-operable. */
export function Toggle({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  /** Accessible name; the visible label is the caller's. */
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative inline-flex h-7 w-14 shrink-0 items-center border-2 transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${checked ? "border-lime bg-lime" : "border-line-3 bg-ink"}`}
    >
      <span
        className={`inline-block h-[20px] w-[20px] transition-transform ${checked ? "translate-x-[30px] bg-ink" : "translate-x-[2px] bg-stone"}`}
      />
    </button>
  );
}


/**
 * The panel: `--panel` fill, 2px `--line` border, square. A title is optional; state tints are for
 * the three big states only.
 */
export function Panel({
  title,
  description,
  children,
  action,
  tint = "none",
  className = "",
  bodyClassName = "p-6",
}: {
  title?: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  action?: ReactNode;
  tint?: "none" | "secure" | "busy" | "failed";
  className?: string;
  bodyClassName?: string;
}) {
  const tintClass = {
    none: "border-line bg-panel",
    secure: "border-lime bg-panel",
    busy: "border-ember bg-ember-ink",
    failed: "border-alert bg-alert-ink",
  }[tint];
  return (
    <section className={`border-2 ${tintClass} ${className}`}>
      {title ? (
        <div className="flex items-start justify-between gap-4 border-b-2 border-line px-6 py-4">
          <div className="min-w-0">
            <h2 className="font-mono text-[18px] font-medium text-bone">{title}</h2>
            {description ? <p className="mt-1 text-[13.5px] text-stone">{description}</p> : null}
          </div>
          {action}
        </div>
      ) : null}
      <div className={bodyClassName}>{children}</div>
    </section>
  );
}

/** A page's title row: MAP / SHIELD / SETUP in display-m, a sentence, and a right-hand slot. */
export function PageHeader({ title, subtitle, action }: { title: string; subtitle?: ReactNode; action?: ReactNode }) {
  return (
    <header className="flex flex-wrap items-end justify-between gap-4">
      <div className="min-w-0">
        <h1 className="font-display text-[44px] uppercase leading-none text-bone">{title}</h1>
        {subtitle ? <p className="mt-3 text-[16px] text-stone">{subtitle}</p> : null}
      </div>
      {action}
    </header>
  );
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
