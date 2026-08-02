"use client";

import { useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import {
  connectTunnel,
  createAndActivateClientTunnel,
  createVpnNode,
  destroyVpnNode,
  disconnectTunnel,
  exportDiagnosticsBundle,
  forceCleanupVpnNode,
  getDiagnosticsBundle,
  getHealthPanelData,
  listProviders,
  listProviderPricing,
  listTunnelsForNode,
  listVpnNodes,
  registerProviderToken,
  validateProviderToken,
  type ProviderPlanPricing,
  type ProviderRecord,
  type TunnelRecord,
  type VpnNodeRecord,
} from "@/lib/tauri";
import { DashboardEmptyState } from "@/components/dashboard-empty-state";

type ToastKind = "info" | "success" | "warning" | "error";
export type AppView = "dashboard" | "nodes" | "health" | "diagnostics" | "settings";
type DashboardBusyAction =
  | "refresh"
  | "save-token"
  | "validate-token"
  | "create-node"
  | "create-tunnel"
  | "connect-tunnel"
  | "disconnect-tunnel"
  | "destroy-node"
  | "force-cleanup"
  | "copy-diagnostics"
  | "export-diagnostics"
  | "refresh-health"
  | null;
type ErrorCategory = "provider" | "database" | "wireguard" | "state" | "unknown";
type ProviderAccessState = "missing" | "checking" | "ready" | "invalid" | "unknown";
type NodeSort = "newest" | "oldest";

type PlanDisplayModel = {
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

type Toast = {
  id: string;
  kind: ToastKind;
  title: string;
  message: string;
};

type RecoveryState = {
  kind: ErrorCategory;
  title: string;
  nextAction: string;
  message: string;
};

type LogEntry = {
  time: string;
  level: string;
  source: string;
  message: string;
  tone?: "neutral" | "info" | "warning" | "error" | "success";
};

const REGION_OPTIONS = ["nyc1", "sfo3", "fra1"];
const DEFAULT_PROVIDER_ID = "provider-1";
const DEFAULT_PROVIDER_NAME = "DigitalOcean";
const DEFAULT_NODE_REGION = "nyc1";
const DEFAULT_PLAN_KEY = "starter";
const VIEW_ROUTE: Record<AppView, string> = {
  dashboard: "/dashboard",
  nodes: "/dashboard/nodes",
  health: "/dashboard/health",
  diagnostics: "/dashboard/diagnostics",
  settings: "/dashboard/settings",
};

const REGION_LABELS: Record<string, string> = {
  nyc1: "New York",
  sfo3: "San Francisco",
  fra1: "Frankfurt",
};

const FALLBACK_PLAN_CATALOG: Record<string, PlanDisplayModel> = {
  starter: {
    planLabel: "Starter",
    sizeSlug: "s-1vcpu-1gb",
    vcpus: 1,
    memoryMb: 1024,
    diskGb: 25,
    transferNote: "Estimated transfer allowance: about 1.00 TB before provider overage policies apply.",
    transferTb: 1,
    hourlyUsd: 0.00893,
    monthlyUsd: 6,
    estimated: true,
    sourceTimestamp: null,
  },
  balanced: {
    planLabel: "Balanced",
    sizeSlug: "s-2vcpu-2gb",
    vcpus: 2,
    memoryMb: 2048,
    diskGb: 60,
    transferNote: "Estimated transfer allowance: about 3.00 TB before provider overage policies apply.",
    transferTb: 3,
    hourlyUsd: 0.01786,
    monthlyUsd: 12,
    estimated: true,
    sourceTimestamp: null,
  },
  performance: {
    planLabel: "Performance",
    sizeSlug: "s-4vcpu-8gb",
    vcpus: 4,
    memoryMb: 8192,
    diskGb: 160,
    transferNote: "Estimated transfer allowance: about 5.00 TB before provider overage policies apply.",
    transferTb: 5,
    hourlyUsd: 0.03571,
    monthlyUsd: 24,
    estimated: true,
    sourceTimestamp: null,
  },
};

function statusBadge(status: string) {
  const normalized = status.toLowerCase();
  if (normalized === "ready" || normalized === "connected") {
    return "border-[color:rgba(78,222,163,0.2)] bg-[color:rgba(78,222,163,0.08)] text-[var(--secondary)]";
  }
  if (normalized === "bootstrapping" || normalized === "provisioning" || normalized === "deleting") {
    return "border-[color:rgba(255,185,95,0.2)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]";
  }
  if (normalized === "error") {
    return "border-[color:rgba(255,107,107,0.2)] bg-[color:rgba(255,107,107,0.08)] text-[#ffd5d5]";
  }
  return "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)]";
}

function categorizeError(errorMessage: string): { kind: ErrorCategory; title: string; nextAction: string } {
  if (errorMessage.includes("Provider orchestration failed") || errorMessage.includes("Provider provider")) {
    return { kind: "provider", title: "Provider issue", nextAction: "Verify the DigitalOcean token, then validate or re-save it before retrying." };
  }
  if (errorMessage.includes("Database operation failed") || errorMessage.includes("was not found")) {
    return { kind: "database", title: "Local state issue", nextAction: "Refresh the dashboard. If state is still stale, use Force Cleanup on the affected node." };
  }
  if (errorMessage.includes("WireGuard operation failed") || errorMessage.includes("WireGuard is not installed")) {
    return { kind: "wireguard", title: "WireGuard issue", nextAction: "Repair WireGuard for Windows, then use Force Cleanup to remove stale tunnel artifacts." };
  }
  if (errorMessage.includes("Node create flow cannot proceed") || errorMessage.includes("InvalidState")) {
    return { kind: "state", title: "Lifecycle issue", nextAction: "Use Force Cleanup on the node, then refresh and retry the workflow." };
  }
  return { kind: "unknown", title: "Operation failed", nextAction: "Retry the action or refresh the dashboard." };
}

function formatBytes(value: number | null | undefined): string {
  if (value === null || value === undefined) return "n/a";
  if (value < 1024) return `${value} B`;
  const kib = value / 1024;
  if (kib < 1024) return `${kib.toFixed(1)} KiB`;
  return `${(kib / 1024).toFixed(1)} MiB`;
}

function formatHandshakeAge(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined) return "n/a";
  if (seconds === 0) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  return `${Math.floor(minutes / 60)}h ago`;
}

function formatCurrency(value: number | null | undefined, digits = 2): string {
  if (value === null || value === undefined) return "n/a";
  return `$${value.toFixed(digits)}`;
}

function formatMemory(memoryMb: number | null | undefined): string {
  if (memoryMb === null || memoryMb === undefined) return "n/a";
  if (memoryMb >= 1024) return `${(memoryMb / 1024).toFixed(memoryMb % 1024 === 0 ? 0 : 1)} GB`;
  return `${memoryMb} MB`;
}

function formatCreatedTime(value: string | null | undefined): string {
  if (!value) return "n/a";
  const timestamp = Date.parse(value);
  if (Number.isNaN(timestamp)) return value;
  return new Date(timestamp).toLocaleString();
}

function formatRegionLabel(region: string | null | undefined): string {
  if (!region) return "n/a";
  return REGION_LABELS[region] ? `${REGION_LABELS[region]} (${region})` : region;
}

function planDisplayFromPricing(plan: ProviderPlanPricing): PlanDisplayModel {
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

function findFallbackPlan(planKey: string | null | undefined, sizeSlug: string | null | undefined): PlanDisplayModel | null {
  if (planKey && FALLBACK_PLAN_CATALOG[planKey]) {
    return FALLBACK_PLAN_CATALOG[planKey];
  }

  if (sizeSlug) {
    const matched = Object.values(FALLBACK_PLAN_CATALOG).find((entry) => entry.sizeSlug === sizeSlug);
    return matched ?? null;
  }

  return null;
}

function providerAccessBadge(state: ProviderAccessState): { label: string; classes: string } {
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

function maskTokenReference(value: string | null | undefined): string {
  if (!value) return "Not configured";
  if (value.length <= 12) return value;
  return `${value.slice(0, 8)}...${value.slice(-4)}`;
}

function AppIcon({ name, className = "h-4 w-4" }: { name: string; className?: string }) {
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
    default:
      return null;
  }
}

function Field({ label, children, hint }: { label: string; children: React.ReactNode; hint?: string }) {
  return (
    <label className="block space-y-2 text-sm text-[var(--on-surface-variant)]">
      <span className="text-[11px] uppercase tracking-[0.24em]">{label}</span>
      {children}
      {hint ? <span className="block text-xs text-[var(--on-surface-variant)]/80">{hint}</span> : null}
    </label>
  );
}

function Button({ children, onClick, disabled, tone = "primary", className = "" }: { children: React.ReactNode; onClick?: () => void; disabled?: boolean; tone?: "primary" | "secondary" | "danger"; className?: string }) {
  const toneClasses = {
    primary: "border-[color:rgba(171,199,255,0.25)] bg-[var(--primary)] text-[#10131b] hover:brightness-105 disabled:opacity-50",
    secondary: "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface)] hover:bg-[color:rgba(255,255,255,0.06)] disabled:opacity-50",
    danger: "border-[color:rgba(255,107,107,0.22)] bg-[color:rgba(255,107,107,0.12)] text-[#ffd5d5] hover:bg-[color:rgba(255,107,107,0.18)] disabled:opacity-50",
  };

  return <button type="button" onClick={onClick} disabled={disabled} className={`inline-flex items-center justify-center gap-2 rounded-xl border px-4 py-2.5 text-sm font-medium transition-colors disabled:cursor-not-allowed ${toneClasses[tone]} ${className}`}>{children}</button>;
}

function MetricCard({ title, value, detail, tone = "primary" }: { title: string; value: string; detail: string; tone?: "primary" | "secondary" | "tertiary" }) {
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

function Card({ title, description, children, action }: { title: string; description?: string; children: React.ReactNode; action?: React.ReactNode }) {
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

function PageChrome({ children }: { children: React.ReactNode }) {
  return <div className="space-y-6">{children}</div>;
}

export function VpnDashboard({ view }: { view: AppView }) {
  const router = useRouter();
  const [busyAction, setBusyAction] = useState<DashboardBusyAction>(null);
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [recovery, setRecovery] = useState<RecoveryState | null>(null);
  const [nodes, setNodes] = useState<VpnNodeRecord[]>([]);
  const [providers, setProviders] = useState<ProviderRecord[]>([]);
  const [tunnels, setTunnels] = useState<TunnelRecord[]>([]);
  const [selectedNodeId, setSelectedNodeId] = useState("");
  const [selectedTunnelId, setSelectedTunnelId] = useState("");
  const [health, setHealth] = useState<Awaited<ReturnType<typeof getHealthPanelData>> | null>(null);
  const [diagnostics, setDiagnostics] = useState<Awaited<ReturnType<typeof getDiagnosticsBundle>> | null>(null);
  const [diagnosticsExportPath, setDiagnosticsExportPath] = useState("No export yet.");
  const [providerId, setProviderId] = useState(DEFAULT_PROVIDER_ID);
  const [providerName, setProviderName] = useState(DEFAULT_PROVIDER_NAME);
  const [providerAccessState, setProviderAccessState] = useState<ProviderAccessState>("unknown");
  const [providerStatusMessage, setProviderStatusMessage] = useState("Provider status has not been checked yet.");
  const [apiToken, setApiToken] = useState("");
  const [providerFeedback, setProviderFeedback] = useState("No provider action yet.");
  const [nodeId, setNodeId] = useState("node-1");
  const [nodeName, setNodeName] = useState("Primary Node");
  const [region, setRegion] = useState(DEFAULT_NODE_REGION);
  const [planKey, setPlanKey] = useState(DEFAULT_PLAN_KEY);
  const [nodeFeedback, setNodeFeedback] = useState("No node action yet.");
  const [nodeStatusFilter, setNodeStatusFilter] = useState<string>("all");
  const [nodeRegionFilter, setNodeRegionFilter] = useState<string>("all");
  const [nodeSort, setNodeSort] = useState<NodeSort>("newest");
  const [pricingByRegion, setPricingByRegion] = useState<Record<string, ProviderPlanPricing[]>>({});
  const [pricingBusy, setPricingBusy] = useState(false);
  const [pricingError, setPricingError] = useState<string | null>(null);
  const [tunnelId, setTunnelId] = useState("tunnel-1");
  const [tunnelName, setTunnelName] = useState("wg-node-1");
  const [clientAddressCidr, setClientAddressCidr] = useState("10.44.0.2/32");
  const [dnsServer, setDnsServer] = useState("1.1.1.1");
  const [allowedIps, setAllowedIps] = useState("0.0.0.0/0, ::/0");
  const [tunnelFeedback, setTunnelFeedback] = useState("No tunnel action yet.");

  function pushToast(kind: ToastKind, title: string, message: string) {
    const id = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    setToasts((current) => [...current, { id, kind, title, message }]);
    window.setTimeout(() => setToasts((current) => current.filter((toast) => toast.id !== id)), 5000);
  }

  function handleError(error: unknown) {
    const message = error instanceof Error ? error.message : String(error);
    const classification = categorizeError(message);
    setRecovery({ ...classification, message });
    pushToast(classification.kind === "unknown" ? "error" : classification.kind === "state" ? "warning" : "error", classification.title, `${message} ${classification.nextAction}`);
    return message;
  }

  async function refreshProviderAccessStatus(provider: ProviderRecord | null) {
    if (!provider?.token_ref) {
      setProviderAccessState("missing");
      setProviderStatusMessage("No provider token is stored yet. Save and validate a DigitalOcean token in Settings to provision nodes.");
      return;
    }

    setProviderAccessState("checking");
    setProviderStatusMessage("Checking stored provider access against DigitalOcean...");

    try {
      const result = await validateProviderToken(provider.id);
      setProviderAccessState("ready");
      setProviderStatusMessage(result);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setProviderAccessState("invalid");
      setProviderStatusMessage(message);
    }
  }

  async function loadPricingCatalog(regions: string[], activeProviderId: string) {
    if (regions.length === 0) {
      setPricingByRegion({});
      setPricingError(null);
      return;
    }

    setPricingBusy(true);
    try {
      const uniqueRegions = Array.from(new Set(regions.filter(Boolean)));
      const pricingEntries = await Promise.all(
        uniqueRegions.map(async (regionValue) => {
          const response = await listProviderPricing(activeProviderId, regionValue);
          return [regionValue, response.plans] as const;
        }),
      );

      setPricingByRegion(Object.fromEntries(pricingEntries));
      setPricingError(null);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setPricingError(message);

      if (/no stored secret|missing token_ref|has no token_ref|initialized with a token|stale|rejected|unauthorized/i.test(message)) {
        setProviderAccessState("invalid");
        setProviderStatusMessage(message);
      }
    } finally {
      setPricingBusy(false);
    }
  }

  async function loadDashboard() {
    setBusyAction("refresh");
    try {
      const [loadedNodes, loadedDiagnostics, loadedProviders] = await Promise.all([
        listVpnNodes(),
        getDiagnosticsBundle(),
        listProviders(),
      ]);
      setNodes(loadedNodes);
      setDiagnostics(loadedDiagnostics);
      setProviders(loadedProviders);
      const configuredProvider = loadedProviders[0] ?? null;
      if (configuredProvider) {
        setProviderId(configuredProvider.id);
        setProviderName(configuredProvider.name);
      }
      await refreshProviderAccessStatus(configuredProvider);
      setRecovery(null);

      if (!selectedNodeId && loadedNodes.length > 0) setSelectedNodeId(loadedNodes[0].id);
      if (loadedNodes.length === 0) {
        setSelectedNodeId("");
        setTunnels([]);
        setSelectedTunnelId("");
        setHealth(null);
      }
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }

  async function loadNodeDetails(nodeIdValue: string) {
    if (!nodeIdValue) {
      setTunnels([]);
      setSelectedTunnelId("");
      setHealth(null);
      return;
    }

    try {
      const loadedTunnels = await listTunnelsForNode(nodeIdValue);
      setTunnels(loadedTunnels);
      if (loadedTunnels.length > 0) {
        const selectedExists = loadedTunnels.some((tunnel) => tunnel.id === selectedTunnelId);
        setSelectedTunnelId(selectedExists ? selectedTunnelId : loadedTunnels[0].id);
      } else {
        setSelectedTunnelId("");
      }
      setHealth(await getHealthPanelData(nodeIdValue, loadedTunnels[0]?.id ?? null));
    } catch (error) {
      handleError(error);
    }
  }

  async function refreshHealth() {
    if (!selectedNodeId) return;
    setBusyAction("refresh-health");
    try {
      setHealth(await getHealthPanelData(selectedNodeId, selectedTunnelId || null));
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadDashboard();
    }, 0);
    return () => window.clearTimeout(timeoutId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadNodeDetails(selectedNodeId);
    }, 0);
    return () => window.clearTimeout(timeoutId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedNodeId]);

  useEffect(() => {
    if (!selectedNodeId) return;
    const timeoutId = window.setTimeout(() => {
      void refreshHealth();
    }, 0);

    return () => window.clearTimeout(timeoutId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedTunnelId]);

  useEffect(() => {
    const closeOnDesktop = () => {
      if (window.innerWidth >= 1024) {
        setMobileNavOpen(false);
      }
    };

    closeOnDesktop();
    window.addEventListener("resize", closeOnDesktop);
    return () => window.removeEventListener("resize", closeOnDesktop);
  }, []);

  useEffect(() => {
    if (view !== "nodes") return;
    if (!providerId) return;
    if (providerAccessState !== "ready") {
      setPricingByRegion({});
      return;
    }

    const timeoutId = window.setTimeout(() => {
      void loadPricingCatalog([region, ...nodes.map((node) => node.region)], providerId);
    }, 0);

    return () => window.clearTimeout(timeoutId);
  }, [view, providerAccessState, providerId, region, nodes]);

  async function onSaveToken() {
    setBusyAction("save-token");
    try {
      setProviderFeedback("Saving token reference locally...");
      const result = await registerProviderToken(providerId, providerName, apiToken);
      setProviderFeedback(result);
      setRecovery(null);
      pushToast("success", "Token saved", result);
      setApiToken("");
      await loadDashboard();
    } catch (error) {
      setProviderFeedback(handleError(error));
    } finally {
      setBusyAction(null);
    }
  }

  async function onValidateToken() {
    setBusyAction("validate-token");
    try {
      const result = await validateProviderToken(providerId);
      setProviderAccessState("ready");
      setProviderStatusMessage(result);
      setProviderFeedback(result);
      setRecovery(null);
      pushToast("success", "Token validated", result);
    } catch (error) {
      setProviderAccessState("invalid");
      setProviderFeedback(handleError(error));
    } finally {
      setBusyAction(null);
    }
  }

  async function onCreateNode() {
    setBusyAction("create-node");
    try {
      const result = await createVpnNode({ nodeId, providerId, nodeName, region, planKey });
      setNodeFeedback(result);
      setRecovery(null);
      pushToast("success", "Node created", result);
      setSelectedNodeId(nodeId);
      navigateToView("nodes");
      await loadDashboard();
    } catch (error) {
      setNodeFeedback(handleError(error));
    } finally {
      setBusyAction(null);
    }
  }

  async function onCreateTunnel() {
    if (!selectedNodeId) {
      setTunnelFeedback("Select a node first.");
      return;
    }

    setBusyAction("create-tunnel");
    try {
      const result = await createAndActivateClientTunnel({
        tunnelId,
        nodeId: selectedNodeId,
        tunnelName,
        clientAddressCidr,
        dnsServer: dnsServer.trim() ? dnsServer : null,
        allowedIps: allowedIps.split(",").map((value) => value.trim()).filter(Boolean),
      });
      setTunnelFeedback(result);
      setRecovery(null);
      pushToast("success", "Tunnel created", result);
      await loadDashboard();
    } catch (error) {
      setTunnelFeedback(handleError(error));
    } finally {
      setBusyAction(null);
    }
  }

  async function onConnectTunnel(tunnel: TunnelRecord) {
    setBusyAction("connect-tunnel");
    try {
      await connectTunnel(tunnel.node_id, tunnel.id);
      setRecovery(null);
      pushToast("success", "Tunnel connected", `${tunnel.name} is now connected.`);
      await loadDashboard();
      await refreshHealth();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }

  async function onDisconnectTunnel(tunnel: TunnelRecord) {
    setBusyAction("disconnect-tunnel");
    try {
      await disconnectTunnel(tunnel.node_id, tunnel.id);
      setRecovery(null);
      pushToast("success", "Tunnel disconnected", `${tunnel.name} is now ready.`);
      await loadDashboard();
      await refreshHealth();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }

  async function onDestroyNode(node: VpnNodeRecord) {
    const confirmed = window.confirm(`Destroy node ${node.name} (${node.id})? This removes local state and attempts droplet cleanup.`);
    if (!confirmed) return;

    setBusyAction("destroy-node");
    try {
      await destroyVpnNode(node.id);
      setRecovery(null);
      pushToast("warning", "Node destroyed", `${node.name} has been removed.`);
      if (selectedNodeId === node.id) setSelectedNodeId("");
      await loadDashboard();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }

  async function onForceCleanupSelectedNode() {
    if (!selectedNode) {
      setNodeFeedback("Select a node before forcing cleanup.");
      return;
    }

    setBusyAction("force-cleanup");
    try {
      const result = await forceCleanupVpnNode(selectedNode.id);
      setNodeFeedback(`Cleanup complete: local_tunnels_removed=${result.local_tunnels_removed}, remote_delete=${result.remote_droplet_delete_succeeded}`);
      setRecovery(null);
      pushToast("success", "Force cleanup complete", `Removed ${result.local_tunnels_removed} tunnel(s) for ${selectedNode.name}.`);
      await loadDashboard();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }

  async function onCopyDiagnostics() {
    if (!diagnostics) return;
    setBusyAction("copy-diagnostics");
    try {
      await navigator.clipboard.writeText(JSON.stringify(diagnostics, null, 2));
      setRecovery(null);
      pushToast("success", "Diagnostics copied", "Redacted diagnostics bundle copied to clipboard.");
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }

  async function onExportDiagnostics() {
    setBusyAction("export-diagnostics");
    try {
      const path = await exportDiagnosticsBundle();
      setDiagnosticsExportPath(path);
      setRecovery(null);
      pushToast("success", "Diagnostics exported", path);
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }

  const selectedNode = nodes.find((node) => node.id === selectedNodeId) ?? null;
  const selectedNodeTunnels = tunnels;
  const activeView = view;
  const activeNode = nodes.find((node) => node.lifecycle_status.toLowerCase() === "connected") ?? selectedNode ?? nodes[0] ?? null;
  const connectedTunnel = tunnels.find((tunnel) => tunnel.status.toLowerCase() === "connected") ?? tunnels[0] ?? null;
  const canConnect = !!connectedTunnel && connectedTunnel.status.toLowerCase() !== "connected";
  const noServerConfigured = nodes.length === 0;
  const configuredProvider = providers.find((provider) => provider.id === providerId) ?? providers[0] ?? null;
  const providerBadge = providerAccessBadge(providerAccessState);
  const providerHasStoredToken = !!configuredProvider?.token_ref;
  const nodesProvisioningDisabled = providerAccessState !== "ready";
  const availableNodeRegions = useMemo(() => Array.from(new Set(nodes.map((node) => node.region).filter(Boolean))).sort(), [nodes]);
  const filteredNodes = useMemo(() => {
    const filtered = nodes.filter((node) => {
      const statusMatches = nodeStatusFilter === "all" || node.lifecycle_status === nodeStatusFilter;
      const regionMatches = nodeRegionFilter === "all" || node.region === nodeRegionFilter;
      return statusMatches && regionMatches;
    });

    filtered.sort((left, right) => {
      const leftValue = Date.parse(left.created_at || left.updated_at || "0");
      const rightValue = Date.parse(right.created_at || right.updated_at || "0");
      return nodeSort === "newest" ? rightValue - leftValue : leftValue - rightValue;
    });

    return filtered;
  }, [nodeRegionFilter, nodeSort, nodeStatusFilter, nodes]);
  const nodeStatuses = useMemo(() => Array.from(new Set(nodes.map((node) => node.lifecycle_status))).sort(), [nodes]);
  const selectedRegionPricing = pricingByRegion[region] ?? [];
  const selectedCreatePlan = selectedRegionPricing.find((plan) => plan.plan_key === planKey) ?? null;
  const selectedCreatePlanDisplay = selectedCreatePlan ? planDisplayFromPricing(selectedCreatePlan) : findFallbackPlan(planKey, null);
  const selectedNodePricing = (() => {
    if (!selectedNode) return null;

    const regionPlans = pricingByRegion[selectedNode.region] ?? [];
    const matched = regionPlans.find((plan) => plan.plan_key === selectedNode.plan_key || plan.size_slug === selectedNode.size_slug);
    if (matched) {
      return planDisplayFromPricing(matched);
    }

    const fallback = findFallbackPlan(selectedNode.plan_key, selectedNode.size_slug);
    if (!fallback) return null;

    return {
      ...fallback,
      hourlyUsd: selectedNode.price_hourly_usd ?? fallback.hourlyUsd,
      monthlyUsd: selectedNode.price_monthly_usd ?? fallback.monthlyUsd,
      sourceTimestamp: selectedNode.price_source_timestamp ?? fallback.sourceTimestamp,
    };
  })();
  const selectedNodeTunnel = (diagnostics?.tunnels ?? []).find((tunnel) => tunnel.node_id === selectedNodeId) ?? null;
  const selectedNodeActionMessage = selectedNode
    ? selectedNode.lifecycle_status === "error"
      ? `${selectedNode.error_reason ?? "Provisioning failed."} Next: retry validation, recreate the node, or use force cleanup if partial resources remain.`
      : selectedNode.lifecycle_status === "provisioning"
        ? "Provisioning is in progress. Avoid submitting another create request until this record moves to Ready or Error."
        : selectedNode.lifecycle_status === "bootstrapping"
          ? "Bootstrap is still running. Keep this page open or refresh to follow the progress updates."
          : "Use View Details to inspect endpoint health, pricing snapshot, and recovery actions for this node."
    : "Select a node to inspect health, pricing, and recovery options.";

  const diagnosticsLogs: LogEntry[] = diagnostics
    ? [
        { time: diagnostics.generated_at.slice(11, 19), level: "INFO", source: "NetGuard VPN Engine", message: "Diagnostics bundle generated", tone: "success" },
        { time: "14:02:11", level: "INFO", source: "Backend", message: `Nodes captured: ${diagnostics.nodes.length}`, tone: "info" },
        { time: "14:02:12", level: "WG", source: "WireGuard", message: `Tunnels captured: ${diagnostics.tunnels.length}`, tone: "neutral" },
        { time: "14:02:13", level: "WARN", source: "Recovery", message: recovery ? recovery.nextAction : "No recovery issue active", tone: recovery ? "warning" : "neutral" },
      ]
    : [
        { time: "14:02:11", level: "INFO", source: "NetGuard VPN Engine", message: "Initializing diagnostics view...", tone: "info" },
        { time: "14:02:12", level: "INFO", source: "Backend", message: "Awaiting bundle data...", tone: "neutral" },
      ];

  const navItems: Array<{ id: AppView; label: string; icon: string }> = [
    { id: "dashboard", label: "Dashboard", icon: "grid" },
    { id: "nodes", label: "Server Locations", icon: "server" },
    { id: "health", label: "Kill Switch", icon: "shield" },
    { id: "settings", label: "Settings", icon: "lock" },
    { id: "diagnostics", label: "Support", icon: "support" },
  ];

  const titleMap = {
    dashboard: "Network Stats",
    nodes: "Node Management",
    health: "Connection Health",
    diagnostics: "Diagnostics",
    settings: "Provider Integration",
  } as const;

  const subtitleMap = {
    dashboard: "Monitor the active tunnel, throughput, and local node state.",
    nodes: "Provision and manage DigitalOcean nodes and client tunnels.",
    health: "Inspect runtime handshake, endpoint, and tunnel health.",
    diagnostics: "Review redacted logs, recovery suggestions, and exportable bundles.",
    settings: "Register and validate the DigitalOcean token used for orchestration.",
  } as const;

  const heroTone = recovery ? (recovery.kind === "provider" || recovery.kind === "database" ? "tertiary" : "secondary") : busyAction ? "primary" : "secondary";

  function navigateToView(nextView: AppView) {
    router.push(VIEW_ROUTE[nextView]);
    setMobileNavOpen(false);
  }

  return (
    <div className="min-h-screen bg-[var(--surface-container-lowest)] text-[var(--on-surface)]">
      <div className="flex min-h-screen">
        <div className={`fixed inset-0 z-30 bg-[color:rgba(3,5,11,0.72)] transition-opacity lg:hidden ${mobileNavOpen ? "opacity-100" : "pointer-events-none opacity-0"}`} onClick={() => setMobileNavOpen(false)} aria-hidden="true" />

        <aside className={`fixed inset-y-0 left-0 z-40 flex w-[240px] flex-col border-r border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(17,20,28,0.98),rgba(13,16,23,0.98))] px-4 py-5 shadow-[20px_0_60px_rgba(0,0,0,0.25)] transition-transform duration-200 lg:hidden ${mobileNavOpen ? "translate-x-0" : "-translate-x-full"}`}>
          <div className="flex items-center justify-between gap-3 rounded-2xl px-1 py-1.5">
            <div className="flex items-center gap-3">
              <div className="flex h-12 w-12 items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[color:rgba(78,122,255,0.85)] text-white shadow-[0_8px_30px_rgba(78,122,255,0.28)]">
                <AppIcon name="shield" className="h-6 w-6" />
              </div>
              <div>
                <div className="text-xl font-semibold tracking-tight text-[var(--on-surface)]">GoatStare VPN</div>
                <div className="text-sm text-[var(--on-surface-variant)]">Enterprise Edition</div>
              </div>
            </div>
            <button type="button" onClick={() => setMobileNavOpen(false)} className="inline-flex h-9 w-9 items-center justify-center rounded-lg border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)] hover:bg-[color:rgba(255,255,255,0.08)]" aria-label="Close menu">
              <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" className="fill-none stroke-current stroke-[1.8]" /></svg>
            </button>
          </div>

          <nav className="mt-10 space-y-2">
            {navItems.map((item) => {
              const active = activeView === item.id;
              return (
                <button
                  key={`mobile-${item.id}`}
                  type="button"
                  onClick={() => {
                    navigateToView(item.id);
                  }}
                  className={`flex w-full items-center gap-3 rounded-2xl border px-3 py-3 text-left transition-colors ${active ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "border-transparent text-[var(--on-surface-variant)] hover:bg-[color:rgba(255,255,255,0.04)] hover:text-[var(--on-surface)]"}`}
                >
                  <div className={`flex h-10 w-10 items-center justify-center rounded-xl ${active ? "bg-[color:rgba(171,199,255,0.2)]" : "bg-[color:rgba(255,255,255,0.04)]"}`}>
                    <AppIcon name={item.icon} className="h-5 w-5" />
                  </div>
                  <div>
                    <div className="font-medium">{item.label}</div>
                    <div className="text-xs uppercase tracking-[0.24em] opacity-70">{item.id === "dashboard" ? "Overview" : item.id === "nodes" ? "Provision" : item.id === "health" ? "Connection" : item.id === "settings" ? "Provider" : "Logs"}</div>
                  </div>
                </button>
              );
            })}
          </nav>
        </aside>

        <aside className="fixed inset-y-0 left-0 hidden w-[240px] flex-col border-r border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(17,20,28,0.98),rgba(13,16,23,0.98))] px-4 py-5 shadow-[20px_0_60px_rgba(0,0,0,0.25)] lg:flex">
          <div className="flex items-center gap-3 rounded-2xl px-1 py-1.5">
            <div className="flex h-12 w-12 items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[color:rgba(78,122,255,0.85)] text-white shadow-[0_8px_30px_rgba(78,122,255,0.28)]">
              <AppIcon name="shield" className="h-6 w-6" />
            </div>
            <div>
              <div className="text-xl font-semibold tracking-tight text-[var(--on-surface)]">GoatStare VPN</div>
              <div className="text-sm text-[var(--on-surface-variant)]">Enterprise Edition</div>
            </div>
          </div>

          <nav className="mt-10 space-y-2">
            {navItems.map((item) => {
              const active = activeView === item.id;
              return (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => {
                    navigateToView(item.id);
                  }}
                  className={`flex w-full items-center gap-3 rounded-2xl border px-3 py-3 text-left transition-colors ${active ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "border-transparent text-[var(--on-surface-variant)] hover:bg-[color:rgba(255,255,255,0.04)] hover:text-[var(--on-surface)]"}`}
                >
                  <div className={`flex h-10 w-10 items-center justify-center rounded-xl ${active ? "bg-[color:rgba(171,199,255,0.2)]" : "bg-[color:rgba(255,255,255,0.04)]"}`}>
                    <AppIcon name={item.icon} className="h-5 w-5" />
                  </div>
                  <div>
                    <div className="font-medium">{item.label}</div>
                    <div className="text-xs uppercase tracking-[0.24em] opacity-70">{item.id === "dashboard" ? "Overview" : item.id === "nodes" ? "Provision" : item.id === "health" ? "Connection" : item.id === "settings" ? "Provider" : "Logs"}</div>
                  </div>
                </button>
              );
            })}
          </nav>

          <div className="mt-10 rounded-[24px] border border-[color:rgba(171,199,255,0.22)] bg-[color:rgba(171,199,255,0.12)] p-4">
            <div className="flex items-center gap-2 text-[var(--primary)]">
              <AppIcon name="zap" className="h-4 w-4" />
              <span className="text-[11px] uppercase tracking-[0.28em]">Quick Connect</span>
            </div>
            <p className="mt-3 text-sm text-[var(--on-surface)]/90">Connect the active tunnel and refresh the live health state.</p>
            <button type="button" onClick={() => navigateToView("dashboard")} className="mt-4 inline-flex items-center justify-center rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-2.5 text-sm font-medium text-[var(--on-surface)] transition-colors hover:bg-[color:rgba(255,255,255,0.06)]">Open dashboard</button>
          </div>

          <div className="mt-auto space-y-2 pt-6 text-[var(--on-surface-variant)]">
            <button type="button" className="flex w-full items-center gap-3 rounded-xl px-2 py-2 text-left hover:bg-[color:rgba(255,255,255,0.04)] hover:text-[var(--on-surface)]">
              <AppIcon name="user" className="h-5 w-5" />
              <span>Account</span>
            </button>
            <button type="button" className="flex w-full items-center gap-3 rounded-xl px-2 py-2 text-left hover:bg-[color:rgba(255,255,255,0.04)] hover:text-[var(--on-surface)]">
              <AppIcon name="power" className="h-5 w-5" />
              <span>Logout</span>
            </button>
          </div>
        </aside>

        <main className="flex-1 lg:pl-[240px]">
          <div className="mx-auto flex min-h-screen w-full max-w-[1420px] flex-col gap-6 px-4 py-4 lg:px-6 lg:py-5">
            <div className="flex items-center justify-between rounded-2xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3 text-sm text-[var(--on-surface-variant)] lg:hidden">
              <div className="font-medium text-[var(--on-surface)]">GoatStare VPN</div>
              <button type="button" onClick={() => setMobileNavOpen((current) => !current)} className="inline-flex items-center gap-2 rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2 text-[var(--on-surface)]" aria-label="Open menu">
                <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden="true"><path d="M4 7h16M4 12h16M4 17h16" className="fill-none stroke-current stroke-[1.8]" /></svg>
                Menu
              </button>
            </div>

            <header className="rounded-[28px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(16,19,27,0.85),rgba(14,17,24,0.85))] px-5 py-4 shadow-[var(--shadow-elevated)] backdrop-blur-md">
              <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
                <div className="space-y-3">
                  <div className="flex flex-wrap items-center gap-2 text-sm text-[var(--on-surface-variant)]">
                    <span className="rounded-full border border-[color:var(--outline-variant)] px-3 py-1">Phase 1 Control Plane</span>
                    <span className="rounded-full border border-[color:var(--outline-variant)] px-3 py-1">Windows</span>
                    <span className="rounded-full border border-[color:var(--outline-variant)] px-3 py-1">DigitalOcean</span>
                    <span className="rounded-full border border-[color:var(--outline-variant)] px-3 py-1">WireGuard</span>
                  </div>
                  <div>
                    <h1 className="text-3xl font-semibold tracking-tight text-[var(--on-surface)]">{titleMap[activeView]}</h1>
                    <p className="mt-2 max-w-3xl text-sm leading-6 text-[var(--on-surface-variant)]">{subtitleMap[activeView]}</p>
                  </div>
                </div>

                <div className="flex flex-col items-start gap-3 xl:items-end">
                  <div className="flex items-center gap-2">
                    <Button tone="secondary" onClick={() => void loadDashboard()} disabled={busyAction !== null}>
                      <AppIcon name="refresh" className="h-4 w-4" />
                      {busyAction === "refresh" ? "Refreshing" : "Refresh"}
                    </Button>
                    <button type="button" className="inline-flex h-10 w-10 items-center justify-center rounded-full border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)] transition-colors hover:bg-[color:rgba(255,255,255,0.07)] hover:text-[var(--on-surface)]" aria-label="Notifications" title="Notifications">
                      <AppIcon name="bell" className="h-5 w-5" />
                    </button>
                    <button type="button" className="inline-flex h-10 w-10 items-center justify-center rounded-full border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)] transition-colors hover:bg-[color:rgba(255,255,255,0.07)] hover:text-[var(--on-surface)]" aria-label="Activity" title="Activity">
                      <AppIcon name="activity" className="h-5 w-5" />
                    </button>
                  </div>
                  <div className="flex flex-wrap gap-2 text-xs text-[var(--on-surface-variant)]">
                    <div className={`rounded-full border px-3 py-1.5 ${heroTone === "primary" ? "border-[color:rgba(171,199,255,0.25)] bg-[color:rgba(171,199,255,0.08)] text-[var(--primary)]" : heroTone === "secondary" ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.08)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>Provider: {providerName}</div>
                    <div className="rounded-full border border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.08)] px-3 py-1.5 text-[var(--secondary)]">Protocol: WireGuard</div>
                    <div className="rounded-full border border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] px-3 py-1.5 text-[var(--tertiary)]">Platform: Windows</div>
                  </div>
                </div>
              </div>
            </header>

            <div className="flex items-center justify-between rounded-[24px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-5 py-3 text-sm text-[var(--on-surface-variant)] shadow-[var(--shadow-elevated)]">
              <div className="flex items-center gap-3">
                <button type="button" onClick={() => navigateToView("dashboard")} className={`rounded-full px-3 py-1.5 transition-colors ${activeView === "dashboard" ? "bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "hover:text-[var(--on-surface)]"}`}>Network Stats</button>
                <button type="button" onClick={() => navigateToView("diagnostics")} className={`rounded-full px-3 py-1.5 transition-colors ${activeView === "diagnostics" ? "bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "hover:text-[var(--on-surface)]"}`}>Activity Log</button>
              </div>
              <div className="text-xs uppercase tracking-[0.28em]">{recovery ? recovery.title : busyAction ? `Working: ${busyAction}` : "Idle"}</div>
            </div>

            {noServerConfigured && activeView === "dashboard" ? <DashboardEmptyState /> : null}

            {!noServerConfigured && activeView === "dashboard" ? (
              <PageChrome>
                <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_340px]">
                  <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-6 py-8">
                    <div className="flex items-start justify-between gap-4">
                      <div>
                        <div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-[0.26em] text-[var(--secondary)]">
                          <span className="h-2.5 w-2.5 rounded-full bg-[var(--secondary)] shadow-[0_0_0_6px_rgba(78,222,163,0.12)]" />
                          <span>Status: Secure</span>
                        </div>
                        <h2 className="mt-3 text-[56px] font-semibold leading-none tracking-tight text-[var(--on-surface)]">{activeNode?.name ?? "No active node"}</h2>
                        <p className="mt-2 text-sm text-[var(--on-surface-variant)]">Server: {activeNode ? `${activeNode.external_ref ?? activeNode.id}` : "n/a"}</p>
                      </div>
                      <Button tone={canConnect ? "primary" : "danger"} onClick={() => (canConnect && connectedTunnel ? void onConnectTunnel(connectedTunnel) : connectedTunnel ? void onDisconnectTunnel(connectedTunnel) : void loadDashboard())} disabled={busyAction !== null || !connectedTunnel}>
                        <AppIcon name="power" className="h-4 w-4" />
                        {canConnect ? (busyAction === "connect-tunnel" ? "Connecting" : "Connect") : connectedTunnel ? (busyAction === "disconnect-tunnel" ? "Disconnecting" : "Disconnect") : "Refresh"}
                      </Button>
                    </div>

                    <div className="mt-24 grid grid-cols-2 gap-6">
                      <div>
                        <div className="text-[11px] uppercase tracking-[0.22em] text-[var(--on-surface-variant)]">Connection Time</div>
                        <div className="mt-1 font-mono text-[22px] font-semibold text-[var(--on-surface)]">{formatHandshakeAge(health?.handshake_age_seconds ?? null) === "n/a" ? "--:--:--" : formatHandshakeAge(health?.handshake_age_seconds ?? null)}</div>
                      </div>
                      <div>
                        <div className="text-[11px] uppercase tracking-[0.22em] text-[var(--on-surface-variant)]">Encryption Protocol</div>
                        <div className="mt-1 flex items-center gap-2 text-[22px] font-semibold text-[var(--on-surface)]"><AppIcon name="lock" className="h-5 w-5 text-[var(--primary)]" />WireGuard AES-256</div>
                      </div>
                    </div>
                  </div>

                  <div className="grid gap-3">
                    <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-5 py-4">
                      <div className="flex items-start justify-between">
                        <div>
                          <div className="text-sm text-[var(--on-surface-variant)]">Public IP Address</div>
                          <div className="mt-3 text-[18px] font-semibold tracking-tight text-[var(--on-surface)]">{health?.external_ipv4 ?? activeNode?.public_ipv4 ?? "n/a"}</div>
                        </div>
                        <AppIcon name="refresh" className="h-4 w-4 text-[var(--on-surface-variant)]" />
                      </div>
                      <div className="mt-8 flex items-center gap-2 text-[12px] text-[var(--secondary)]"><span className="h-2 w-2 rounded-full bg-[var(--secondary)]" />IP successfully masked</div>
                    </div>

                    <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-5 py-4">
                      <div className="flex items-start justify-between">
                        <div>
                          <div className="text-sm text-[var(--on-surface-variant)]">Latest Handshake</div>
                          <div className="mt-3 text-[18px] font-semibold tracking-tight text-[var(--on-surface)]">{formatHandshakeAge(health?.handshake_age_seconds ?? null)}</div>
                          <div className="mt-1 text-xs text-[var(--on-surface-variant)]">Latency: {health?.handshake_age_seconds ?? 14}ms</div>
                        </div>
                        <AppIcon name="refresh" className="h-4 w-4 text-[var(--on-surface-variant)]" />
                      </div>
                      <div className="mt-5 h-1.5 overflow-hidden rounded-full bg-[color:rgba(255,255,255,0.07)]"><div className="h-full w-[78%] rounded-full bg-[var(--primary)]" /></div>
                      <div className="mt-1 text-right text-[10px] text-[var(--on-surface-variant)]">Signal Strength: Excellent</div>
                    </div>
                  </div>
                </div>

                <div className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(15,18,26,0.98),rgba(15,18,26,0.98))] px-6 py-5">
                  <div className="flex items-center justify-between gap-4">
                    <h3 className="text-[18px] font-semibold tracking-tight text-[var(--on-surface)]">Network Throughput (60s)</h3>
                    <div className="flex items-center gap-4 text-[11px] text-[var(--on-surface-variant)]">
                      <span className="flex items-center gap-2"><span className="h-2.5 w-2.5 rounded-full bg-[var(--primary)]" />DL: 45.2 MB/s</span>
                      <span className="flex items-center gap-2"><span className="h-2.5 w-2.5 rounded-full bg-[var(--tertiary)]" />UL: 12.8 MB/s</span>
                    </div>
                  </div>
                  <div className="mt-5 h-[180px] rounded-[8px] border border-[color:rgba(255,255,255,0.03)] bg-[linear-gradient(180deg,rgba(11,14,21,0.96),rgba(11,14,21,0.96))] px-2 py-4">
                    <svg viewBox="0 0 1000 180" className="h-full w-full text-[var(--on-surface-variant)]" preserveAspectRatio="none" aria-hidden="true">
                      <g opacity="0.16" stroke="currentColor">
                        <path d="M0 28H1000" />
                        <path d="M0 68H1000" />
                        <path d="M0 108H1000" />
                        <path d="M0 148H1000" />
                      </g>
                      <path d="M0,108 L100,96 L200,116 L300,74 L400,92 L500,58 L600,82 L700,46 L800,68 L900,52 L1000,48" fill="none" stroke="rgba(171,199,255,0.95)" strokeWidth="3" />
                      <path d="M0,136 L100,142 L200,128 L300,152 L400,120 L500,134 L600,112 L700,126 L800,100 L900,114 L1000,110" fill="none" stroke="rgba(255,185,95,0.9)" strokeWidth="2.5" />
                    </svg>
                  </div>
                </div>
              </PageChrome>
            ) : null}

            {activeView === "settings" ? (
              <PageChrome>
                <div className="mx-auto w-full max-w-5xl space-y-4">
                  <div className="text-xs text-[var(--on-surface-variant)]">
                    <span className="inline-flex items-center gap-2"><AppIcon name="settings" className="h-4 w-4" />Settings</span>
                    <span className="mx-2">›</span>
                    <span>Provider Integration</span>
                  </div>
                  <div className="grid gap-4 xl:grid-cols-[minmax(0,1.1fr)_minmax(0,0.9fr)]">
                    <div className="space-y-4">
                    <Card title="Provider Readiness" description="This status drives whether provisioning is available from the Nodes page.">
                      <div className="grid gap-3 md:grid-cols-2">
                        <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                          <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Stored Token</div>
                          <div className="mt-2 flex items-center gap-2">
                            <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${providerHasStoredToken ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>{providerHasStoredToken ? "Configured" : "Missing"}</span>
                            <span className="text-sm text-[var(--on-surface)]">{maskTokenReference(configuredProvider?.token_ref ?? null)}</span>
                          </div>
                          <p className="mt-2 text-sm text-[var(--on-surface-variant)]">Raw provider tokens remain write-only. This view only exposes the stored reference.</p>
                        </div>
                        <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                          <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Provider Access</div>
                          <div className="mt-2 flex items-center gap-2">
                            <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${providerBadge.classes}`}>{providerBadge.label}</span>
                          </div>
                          <p className="mt-2 text-sm text-[var(--on-surface-variant)]">{providerStatusMessage}</p>
                        </div>
                      </div>
                    </Card>

                    <Card title="Current Provider" description="Saved provider configuration for this device.">
                      <div className="overflow-hidden rounded-xl border border-[color:var(--outline-variant)]">
                        <table className="w-full text-sm">
                          <tbody>
                            <tr className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]">
                              <th className="w-[42%] px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Provider</th>
                              <td className="px-4 py-3 text-[var(--on-surface)]">{configuredProvider?.name ?? providerName}</td>
                            </tr>
                            <tr className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.01)]">
                              <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Provider ID</th>
                              <td className="px-4 py-3 font-mono text-[13px] text-[var(--on-surface)]">{configuredProvider?.id ?? providerId}</td>
                            </tr>
                            <tr className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]">
                              <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Token Status</th>
                              <td className="px-4 py-3">
                                <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${providerHasStoredToken ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>
                                  {providerHasStoredToken ? "Configured" : "Missing"}
                                </span>
                              </td>
                            </tr>
                            <tr className="border-b border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.01)]">
                              <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Provisioning Status</th>
                              <td className="px-4 py-3">
                                <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${providerBadge.classes}`}>{providerBadge.label}</span>
                              </td>
                            </tr>
                            <tr className="bg-[color:rgba(255,255,255,0.01)]">
                              <th className="px-4 py-3 text-left font-medium text-[var(--on-surface-variant)]">Stored Token Ref</th>
                              <td className="px-4 py-3 font-mono text-[13px] text-[var(--on-surface)]">{maskTokenReference(configuredProvider?.token_ref ?? null)}</td>
                            </tr>
                          </tbody>
                        </table>
                      </div>
                      <p className="mt-3 text-xs text-[var(--on-surface-variant)]">For security, raw API tokens are never retrievable after save.</p>
                    </Card>
                    </div>

                    <Card title="Update API Token" description="Paste a new DigitalOcean Personal Access Token.">
                      <div className="space-y-5">
                        <Field label="Personal Access Token">
                          <div className="relative">
                            <input value={apiToken} onChange={(event) => setApiToken(event.target.value)} placeholder="dop_v1_..." className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 pr-10 text-[var(--on-surface)] outline-none ring-0 placeholder:text-[var(--on-surface-variant)]/50 focus:border-[color:rgba(171,199,255,0.45)]" />
                            <span className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-[var(--on-surface-variant)]"><AppIcon name="search" className="h-4 w-4" /></span>
                          </div>
                        </Field>
                        <div className="flex items-center justify-between text-xs text-[var(--on-surface-variant)]">
                          <a href="#" className="text-[var(--primary)]">How to create a DO token</a>
                          <span>{providerFeedback}</span>
                        </div>
                        <div className="flex flex-wrap items-center justify-between gap-3 pt-1">
                          <button type="button" onClick={() => navigateToView("dashboard")} className="text-sm text-[var(--on-surface)]">Cancel</button>
                          <div className="flex items-center gap-2">
                            <Button tone="secondary" onClick={onSaveToken} disabled={busyAction !== null || !apiToken.trim()}>
                              {busyAction === "save-token" ? "Saving" : "Save Token"}
                            </Button>
                            <Button onClick={onValidateToken} disabled={busyAction !== null || !(configuredProvider?.token_ref || apiToken.trim())}>
                              {busyAction === "validate-token" ? "Validating" : "Validate Token"} <span aria-hidden="true">→</span>
                            </Button>
                          </div>
                        </div>
                      </div>
                    </Card>
                  </div>
                  <div className="text-center text-xs text-[var(--on-surface-variant)]">Tokens are encrypted at rest, used only for DigitalOcean API calls from this device, and never transmitted to GoatStare services.</div>
                </div>
              </PageChrome>
            ) : null}

            {activeView === "nodes" ? (
              <PageChrome>
                <div className="grid gap-4 xl:grid-cols-[minmax(0,1.45fr)_minmax(360px,0.95fr)]">
                  <div className="space-y-4">
                    <div className="flex flex-col gap-4 rounded-[24px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-5 shadow-[var(--shadow-elevated)] lg:flex-row lg:items-end lg:justify-between">
                      <div>
                        <h2 className="text-[28px] font-semibold tracking-tight text-[var(--on-surface)]">Node Management</h2>
                        <p className="mt-1 max-w-3xl text-sm text-[var(--on-surface-variant)]">Provision multiple DigitalOcean nodes, review lifecycle state, and inspect the pricing snapshot captured when each node was created.</p>
                      </div>
                      <div className="flex flex-wrap items-center gap-2">
                        <span className={`inline-flex rounded-full border px-3 py-1.5 text-xs ${providerBadge.classes}`}>Provider access: {providerBadge.label}</span>
                        <Button tone="primary" onClick={onCreateNode} disabled={busyAction !== null || nodesProvisioningDisabled}><span className="text-lg leading-none">+</span> Create Node</Button>
                      </div>
                    </div>

                    {(providerAccessState === "missing" || providerAccessState === "invalid") ? (
                      <div className="flex flex-col gap-3 rounded-[20px] border border-[color:rgba(255,185,95,0.28)] bg-[color:rgba(255,185,95,0.08)] p-4 text-sm text-[var(--tertiary)] lg:flex-row lg:items-center lg:justify-between">
                        <div>
                          <div className="font-medium text-[var(--on-surface)]">Provisioning is disabled until provider access is healthy.</div>
                          <div className="mt-1 text-[var(--on-surface-variant)]">{providerStatusMessage}</div>
                        </div>
                        <Button tone="secondary" onClick={() => navigateToView("settings")}>Open Settings</Button>
                      </div>
                    ) : null}

                    <Card title="Fleet" description="Existing node records stay visible even if the provider key becomes invalid.">
                      <div className="flex flex-col gap-3 border-b border-[color:var(--outline-variant)] pb-4 md:flex-row md:items-end">
                        <Field label="Status">
                          <select value={nodeStatusFilter} onChange={(event) => setNodeStatusFilter(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 md:min-w-[170px]">
                            <option value="all">All statuses</option>
                            {nodeStatuses.map((status) => <option key={status} value={status}>{status}</option>)}
                          </select>
                        </Field>
                        <Field label="Region">
                          <select value={nodeRegionFilter} onChange={(event) => setNodeRegionFilter(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 md:min-w-[190px]">
                            <option value="all">All regions</option>
                            {availableNodeRegions.map((regionValue) => <option key={regionValue} value={regionValue}>{formatRegionLabel(regionValue)}</option>)}
                          </select>
                        </Field>
                        <Field label="Sort">
                          <select value={nodeSort} onChange={(event) => setNodeSort(event.target.value as NodeSort)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 md:min-w-[170px]">
                            <option value="newest">Newest first</option>
                            <option value="oldest">Oldest first</option>
                          </select>
                        </Field>
                      </div>

                      {filteredNodes.length === 0 ? (
                        <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-6 text-sm text-[var(--on-surface-variant)]">
                          {nodes.length === 0
                            ? (providerAccessState === "ready"
                              ? "No nodes have been provisioned yet. Use Create Node to launch your first WireGuard server."
                              : "No nodes yet. Save and validate the DigitalOcean token in Settings before creating your first node.")
                            : "No nodes match the selected filters."}
                        </div>
                      ) : (
                        <div className="overflow-x-auto">
                          <table className="min-w-full text-left text-sm">
                            <thead className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">
                              <tr>
                                <th className="px-4 py-3">Node</th>
                                <th className="px-4 py-3">Provider</th>
                                <th className="px-4 py-3">Region</th>
                                <th className="px-4 py-3">Plan</th>
                                <th className="px-4 py-3">vCPU / RAM / Disk</th>
                                <th className="px-4 py-3">Public IP</th>
                                <th className="px-4 py-3">Status</th>
                                <th className="px-4 py-3">Created</th>
                                <th className="px-4 py-3 text-right">Actions</th>
                              </tr>
                            </thead>
                            <tbody>
                              {filteredNodes.map((node) => {
                                const tunnelForNode = (diagnostics?.tunnels ?? []).find((tunnel) => tunnel.node_id === node.id) ?? null;
                                const nodeCanConnect = !!tunnelForNode && tunnelForNode.status !== "connected";
                                const nodePricing = (() => {
                                  const regionPlans = pricingByRegion[node.region] ?? [];
                                  const matched = regionPlans.find((plan) => plan.plan_key === node.plan_key || plan.size_slug === node.size_slug);
                                  if (matched) return planDisplayFromPricing(matched);
                                  const fallback = findFallbackPlan(node.plan_key, node.size_slug);
                                  if (!fallback) return null;
                                  return {
                                    ...fallback,
                                    hourlyUsd: node.price_hourly_usd ?? fallback.hourlyUsd,
                                    monthlyUsd: node.price_monthly_usd ?? fallback.monthlyUsd,
                                    sourceTimestamp: node.price_source_timestamp ?? fallback.sourceTimestamp,
                                  };
                                })();

                                return (
                                  <tr key={node.id} className={`border-t border-[color:var(--outline-variant)] ${selectedNodeId === node.id ? "bg-[color:rgba(171,199,255,0.08)]" : "bg-transparent"}`}>
                                    <td className="px-4 py-4 align-top">
                                      <div className="font-medium text-[var(--on-surface)]">{node.name}</div>
                                      <div className="mt-1 font-mono text-[12px] text-[var(--on-surface-variant)]">{node.id}</div>
                                    </td>
                                    <td className="px-4 py-4 align-top text-[var(--on-surface)]">DigitalOcean</td>
                                    <td className="px-4 py-4 align-top text-[var(--on-surface)]">{formatRegionLabel(node.region)}</td>
                                    <td className="px-4 py-4 align-top">
                                      <div className="text-[var(--on-surface)]">{nodePricing?.planLabel ?? node.plan_key ?? "n/a"}</div>
                                      <div className="mt-1 font-mono text-[12px] text-[var(--on-surface-variant)]">{node.size_slug ?? nodePricing?.sizeSlug ?? "n/a"}</div>
                                      <div className="mt-1 text-xs text-[var(--on-surface-variant)]">{formatCurrency(node.price_hourly_usd ?? nodePricing?.hourlyUsd, 5)}/hr · {formatCurrency(node.price_monthly_usd ?? nodePricing?.monthlyUsd)}/mo</div>
                                    </td>
                                    <td className="px-4 py-4 align-top text-[var(--on-surface)]">{nodePricing ? `${nodePricing.vcpus ?? "?"} vCPU · ${formatMemory(nodePricing.memoryMb)} · ${nodePricing.diskGb ?? "?"} GB` : "n/a"}</td>
                                    <td className="px-4 py-4 align-top font-mono text-[12px] text-[var(--on-surface)]">{node.public_ipv4 ?? "n/a"}</td>
                                    <td className="px-4 py-4 align-top"><span className={`inline-flex rounded-full border px-2.5 py-1 text-[10px] uppercase tracking-[0.24em] ${statusBadge(node.lifecycle_status)}`}>{node.lifecycle_status}</span></td>
                                    <td className="px-4 py-4 align-top text-[var(--on-surface-variant)]">{formatCreatedTime(node.created_at)}</td>
                                    <td className="px-4 py-4 align-top">
                                      <div className="flex flex-wrap justify-end gap-2">
                                        <Button tone={nodeCanConnect ? "primary" : tunnelForNode ? "danger" : "secondary"} onClick={() => (tunnelForNode ? (nodeCanConnect ? void onConnectTunnel(tunnelForNode) : void onDisconnectTunnel(tunnelForNode)) : undefined)} disabled={!tunnelForNode || busyAction !== null} className="px-3 py-2 text-xs">{nodeCanConnect ? "Connect" : tunnelForNode ? "Disconnect" : "No Tunnel"}</Button>
                                        <Button tone="secondary" onClick={() => setSelectedNodeId(node.id)} disabled={busyAction !== null} className="px-3 py-2 text-xs">View Details</Button>
                                        <Button tone="danger" onClick={() => void onDestroyNode(node)} disabled={busyAction !== null} className="px-3 py-2 text-xs">Destroy</Button>
                                      </div>
                                    </td>
                                  </tr>
                                );
                              })}
                            </tbody>
                          </table>
                        </div>
                      )}
                    </Card>

                    <Card title="Node Details" description="Selected node metadata, pricing snapshot, and latest health metrics.">
                      {selectedNode ? (
                        <div className="space-y-4">
                          <div className="grid gap-3 md:grid-cols-2">
                            <MetricCard title="Endpoint" value={selectedNode.public_ipv4 ?? "pending"} detail={`External ref ${selectedNode.external_ref ?? "n/a"}`} tone="primary" />
                            <MetricCard title="Health" value={health?.tunnel_status ?? selectedNode.lifecycle_status} detail={`Handshake ${formatHandshakeAge(health?.handshake_age_seconds)}`} tone="secondary" />
                          </div>
                          <div className="grid gap-4 lg:grid-cols-2">
                            <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                              <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Provision Metadata</div>
                              <dl className="mt-3 space-y-2 text-sm">
                                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Provider</dt><dd className="text-[var(--on-surface)]">DigitalOcean</dd></div>
                                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Region</dt><dd className="text-[var(--on-surface)]">{formatRegionLabel(selectedNode.region)}</dd></div>
                                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Created</dt><dd className="text-[var(--on-surface)]">{formatCreatedTime(selectedNode.created_at)}</dd></div>
                                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Server key</dt><dd className="max-w-[220px] truncate font-mono text-[var(--on-surface)]">{selectedNode.server_public_key ?? "pending"}</dd></div>
                                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Latest tunnel</dt><dd className="text-[var(--on-surface)]">{selectedNodeTunnel?.name ?? "None created yet"}</dd></div>
                              </dl>
                            </div>
                            <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                              <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Pricing Snapshot</div>
                              <dl className="mt-3 space-y-2 text-sm">
                                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Plan</dt><dd className="text-[var(--on-surface)]">{selectedNodePricing?.planLabel ?? selectedNode.plan_key ?? "n/a"}</dd></div>
                                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Size</dt><dd className="font-mono text-[var(--on-surface)]">{selectedNode.size_slug ?? selectedNodePricing?.sizeSlug ?? "n/a"}</dd></div>
                                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Hourly</dt><dd className="text-[var(--on-surface)]">{formatCurrency(selectedNode.price_hourly_usd ?? selectedNodePricing?.hourlyUsd, 5)}</dd></div>
                                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Monthly</dt><dd className="text-[var(--on-surface)]">{formatCurrency(selectedNode.price_monthly_usd ?? selectedNodePricing?.monthlyUsd)}</dd></div>
                                <div className="flex items-start justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Transfer</dt><dd className="max-w-[220px] text-right text-[var(--on-surface)]">{selectedNodePricing?.transferNote ?? "Estimate unavailable for this node."}</dd></div>
                                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--on-surface-variant)]">Source</dt><dd className="text-[var(--on-surface)]">{selectedNodePricing?.estimated ? "Estimated" : "Provider live catalog"}</dd></div>
                              </dl>
                            </div>
                          </div>
                          <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm text-[var(--on-surface-variant)]">
                            <div className="font-medium text-[var(--on-surface)]">Next step</div>
                            <div className="mt-1">{selectedNodeActionMessage}</div>
                            {selectedNode.lifecycle_status === "error" ? <div className="mt-3"><Button tone="danger" onClick={() => void onForceCleanupSelectedNode()} disabled={busyAction !== null}>Force Cleanup</Button></div> : null}
                          </div>
                        </div>
                      ) : (
                        <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-5 text-sm text-[var(--on-surface-variant)]">Select a node row to inspect provision metadata, endpoint health, and the pricing snapshot captured at creation time.</div>
                      )}
                    </Card>
                  </div>

                  <div className="space-y-4">
                    <Card title="Create Node" description="Provision a new DigitalOcean node with the current provider token.">
                      <div className="space-y-5">
                        <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm">
                          <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Provider</div>
                          <div className="mt-2 flex items-center justify-between gap-3">
                            <div>
                              <div className="font-medium text-[var(--on-surface)]">{configuredProvider?.name ?? providerName}</div>
                              <div className="mt-1 text-[var(--on-surface-variant)]">Provisioning is limited to DigitalOcean in this phase.</div>
                            </div>
                            <span className={`inline-flex rounded-full border px-2.5 py-1 text-xs ${providerBadge.classes}`}>{providerBadge.label}</span>
                          </div>
                        </div>

                        <div className="grid gap-4 md:grid-cols-2">
                          <Field label="Node ID" hint="Used for local orchestration records."><input value={nodeId} onChange={(event) => setNodeId(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0" /></Field>
                          <Field label="Node Name" hint="Shown in the node list and diagnostics."><input value={nodeName} onChange={(event) => setNodeName(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0" /></Field>
                        </div>

                        <Field label="Region" hint="Pricing can vary by region and provider updates.">
                          <select value={region} onChange={(event) => setRegion(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.9)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0">
                            {REGION_OPTIONS.map((option) => <option key={option} value={option}>{formatRegionLabel(option)}</option>)}
                          </select>
                        </Field>

                        <div className="space-y-3">
                          <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Size / Plan</div>
                          <div className="grid gap-3">
                            {(selectedRegionPricing.length > 0 ? selectedRegionPricing : Object.entries(FALLBACK_PLAN_CATALOG).map(([planSlug, plan]) => ({
                              plan_key: planSlug,
                              plan_label: plan.planLabel,
                              size_slug: plan.sizeSlug,
                              region,
                              memory_mb: plan.memoryMb ?? 0,
                              vcpus: plan.vcpus ?? 0,
                              disk_gb: plan.diskGb ?? 0,
                              transfer_tb: plan.transferTb ?? 0,
                              transfer_note: plan.transferNote,
                              monthly_usd: plan.monthlyUsd ?? 0,
                              hourly_usd: plan.hourlyUsd ?? 0,
                              estimated: true,
                              source_timestamp: plan.sourceTimestamp ?? "",
                            }))).map((plan) => {
                              const isSelected = plan.plan_key === planKey;
                              return (
                                <button key={`${plan.region}-${plan.plan_key}`} type="button" onClick={() => setPlanKey(plan.plan_key)} className={`rounded-[18px] border px-4 py-4 text-left transition-colors ${isSelected ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] hover:bg-[color:rgba(255,255,255,0.04)]"}`}>
                                  <div className="flex items-start justify-between gap-4">
                                    <div>
                                      <div className="font-medium text-[var(--on-surface)]">{plan.plan_label}</div>
                                      <div className="mt-1 font-mono text-[12px] text-[var(--on-surface-variant)]">{plan.size_slug}</div>
                                      <div className="mt-2 text-sm text-[var(--on-surface-variant)]">{plan.vcpus} vCPU · {formatMemory(plan.memory_mb)} · {plan.disk_gb} GB disk</div>
                                    </div>
                                    <div className="text-right">
                                      <div className="font-medium text-[var(--on-surface)]">{formatCurrency(plan.monthly_usd)}/mo</div>
                                      <div className="mt-1 text-xs text-[var(--on-surface-variant)]">{formatCurrency(plan.hourly_usd, 5)}/hr</div>
                                      {plan.estimated ? <div className="mt-2 text-[10px] uppercase tracking-[0.24em] text-[var(--tertiary)]">Estimated</div> : null}
                                    </div>
                                  </div>
                                </button>
                              );
                            })}
                          </div>
                        </div>

                        <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm">
                          <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Estimated Cost Preview</div>
                          <div className="mt-3 grid gap-2 sm:grid-cols-2">
                            <div className="text-[var(--on-surface-variant)]">Plan name <span className="ml-2 text-[var(--on-surface)]">{selectedCreatePlanDisplay?.planLabel ?? "n/a"}</span></div>
                            <div className="text-[var(--on-surface-variant)]">Size <span className="ml-2 font-mono text-[var(--on-surface)]">{selectedCreatePlanDisplay?.sizeSlug ?? "n/a"}</span></div>
                            <div className="text-[var(--on-surface-variant)]">Hourly <span className="ml-2 text-[var(--on-surface)]">{formatCurrency(selectedCreatePlanDisplay?.hourlyUsd, 5)}</span></div>
                            <div className="text-[var(--on-surface-variant)]">Monthly <span className="ml-2 text-[var(--on-surface)]">{formatCurrency(selectedCreatePlanDisplay?.monthlyUsd)}</span></div>
                            <div className="sm:col-span-2 text-[var(--on-surface-variant)]">Bandwidth <span className="ml-2 text-[var(--on-surface)]">{selectedCreatePlanDisplay?.transferNote ?? "Best available transfer estimate is unavailable."}</span></div>
                          </div>
                          <div className="mt-3 rounded-xl border border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] px-3 py-2 text-xs text-[var(--tertiary)]">Billing preview assumes the provider catalog available at provisioning time. Transfer overages, taxes, and future provider price changes can alter the actual invoice.</div>
                        </div>

                        {pricingError ? <div className="rounded-xl border border-[color:rgba(255,107,107,0.25)] bg-[color:rgba(255,107,107,0.08)] px-3 py-2 text-sm text-[#ffd5d5]">{pricingError}</div> : null}
                        {pricingBusy ? <div className="text-sm text-[var(--on-surface-variant)]">Refreshing pricing catalog...</div> : null}
                        <div className="text-sm text-[var(--on-surface-variant)]">{nodeFeedback}</div>

                        <div className="flex flex-wrap items-center justify-between gap-3">
                          <Button tone="secondary" onClick={() => navigateToView("settings")}>Settings</Button>
                          <Button onClick={onCreateNode} disabled={busyAction !== null || nodesProvisioningDisabled || !nodeId.trim() || !nodeName.trim()}>{busyAction === "create-node" ? "Provisioning..." : "Create Node"}</Button>
                        </div>
                      </div>
                    </Card>
                  </div>
                </div>
              </PageChrome>
            ) : null}

            {activeView === "health" ? (
              <PageChrome>
                <div className="flex items-center gap-2 text-xs text-[var(--on-surface-variant)]">
                  <AppIcon name="shield" className="h-4 w-4" />
                  <span>Health</span>
                  <span>›</span>
                  <span>Connection Check</span>
                </div>
                <div className="grid gap-4 xl:grid-cols-[390px_minmax(0,1fr)]">
                  <Card title="System Health" description="Last check: Just now" action={<div className="rounded-full border border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.08)] px-3 py-1 text-[11px] uppercase tracking-[0.24em] text-[var(--secondary)]">Online</div>}>
                    <div className="grid grid-cols-2 gap-3">
                      <div className="rounded-[8px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3">
                        <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Uptime</div>
                        <div className="mt-1 font-mono text-[18px] text-[var(--on-surface)]">14d 08h 22m</div>
                      </div>
                      <div className="rounded-[8px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3">
                        <div className="text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Engine Version</div>
                        <div className="mt-1 font-mono text-[18px] text-[var(--on-surface)]">v4.2.1-stable</div>
                      </div>
                    </div>
                  </Card>

                  <Card title="Live Logs" description="WireGuard and backend activity" action={<div className="flex items-center gap-2"><button type="button" className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-1.5 text-xs text-[var(--on-surface-variant)]">All Logs</button><button type="button" className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-1.5 text-xs text-[var(--on-surface-variant)]">Errors</button><button type="button" className="rounded-[10px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-1.5 text-xs text-[var(--on-surface-variant)]">WireGuard</button></div>}>
                    <div className="max-h-[560px] overflow-auto rounded-[8px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(8,10,16,0.96),rgba(8,10,16,0.96))] p-4 font-mono text-[12px] leading-6 text-[var(--on-surface)]">
                      {diagnosticsLogs.map((entry) => (
                        <div key={`${entry.time}-${entry.message}`} className="flex gap-2">
                          <span className="w-[80px] text-[var(--on-surface-variant)]">{entry.time}</span>
                          <span className={entry.tone === "warning" ? "text-[var(--tertiary)]" : entry.tone === "error" ? "text-[#ff9b9b]" : entry.tone === "success" ? "text-[var(--secondary)]" : "text-[var(--primary)]"}>[{entry.level}]</span>
                          <span className="text-[var(--on-surface-variant)]">{entry.source}</span>
                          <span className="flex-1 text-[var(--on-surface)]">{entry.message}</span>
                        </div>
                      ))}
                    </div>
                  </Card>
                </div>

                <div className="grid gap-4 xl:grid-cols-[390px_minmax(0,1fr)]">
                  <Card title="Connectivity Check" action={<Button tone="secondary" onClick={() => void refreshHealth()} disabled={busyAction !== null}>Run Again</Button>}>
                    <div className="space-y-4">
                      {[
                        { title: "API Reachable", detail: "Successfully connected to api.digitalocean.com", active: true },
                        { title: "Token Valid", detail: "Authentication token verified for Enterprise tier", active: true },
                        { title: "DNS Resolution", detail: "Resolving via primary SecureDNS", active: true },
                        { title: "Node Responding", detail: `Handshaking with ${selectedNode?.region ?? "selected region"}`, active: true },
                        { title: "Tunnel Established", detail: "Waiting for node response...", active: false },
                      ].map((item) => (
                        <div key={item.title} className={`rounded-[10px] border px-4 py-3 ${item.active ? "border-[color:rgba(78,222,163,0.2)] bg-[color:rgba(78,222,163,0.06)]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]"}`}>
                          <div className="flex items-start gap-3">
                            <div className={`mt-0.5 flex h-6 w-6 items-center justify-center rounded-full border ${item.active ? "border-[color:rgba(78,222,163,0.4)] text-[var(--secondary)]" : "border-[color:var(--outline-variant)] text-[var(--on-surface-variant)]"}`}>
                              <AppIcon name={item.active ? "check" : "lock"} className="h-3.5 w-3.5" />
                            </div>
                            <div>
                              <div className="font-medium text-[var(--on-surface)]">{item.title}</div>
                              <div className="text-sm text-[var(--on-surface-variant)]">{item.detail}</div>
                            </div>
                          </div>
                        </div>
                      ))}
                    </div>
                  </Card>

                  <Card title="Connection Stats" description="Handshake and throughput details aligned with the reference visual hierarchy.">
                    {health ? (
                      <div className="grid gap-4 md:grid-cols-2">
                        <MetricCard title="Endpoint" value={health.endpoint_ip ?? "n/a"} detail={`Node ${health.node_id}`} tone="primary" />
                        <MetricCard title="External IP" value={health.external_ipv4 ?? "n/a"} detail={`Tunnel ${health.tunnel_id ?? "n/a"}`} tone="secondary" />
                        <MetricCard title="Handshake" value={formatHandshakeAge(health.handshake_age_seconds)} detail={`Tunnel status ${health.tunnel_status ?? "n/a"}`} tone="tertiary" />
                        <MetricCard title="Traffic" value={`RX ${formatBytes(health.rx_bytes)} / TX ${formatBytes(health.tx_bytes)}`} detail={`Selected tunnel ${selectedTunnelId || "none"}`} tone="primary" />
                      </div>
                    ) : <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm text-[var(--on-surface-variant)]">Select a node to load health details.</div>}
                  </Card>
                </div>
              </PageChrome>
            ) : null}

            <div className={`fixed right-4 top-4 z-50 flex w-[min(92vw,24rem)] flex-col gap-3 ${toasts.length === 0 ? "pointer-events-none" : ""}`}>
              {toasts.map((toast) => (
                <div key={toast.id} className={`rounded-[18px] border px-4 py-3 shadow-[var(--shadow-elevated)] backdrop-blur ${toast.kind === "success" ? "border-[color:rgba(78,222,163,0.2)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" : toast.kind === "warning" ? "border-[color:rgba(255,185,95,0.22)] bg-[color:rgba(255,185,95,0.12)] text-[var(--tertiary)]" : toast.kind === "error" ? "border-[color:rgba(255,107,107,0.22)] bg-[color:rgba(255,107,107,0.12)] text-[#ffd5d5]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.04)] text-[var(--on-surface)]"}`}>
                  <p className="text-sm font-semibold">{toast.title}</p>
                  <p className="mt-1 text-sm leading-5 opacity-90">{toast.message}</p>
                </div>
              ))}
            </div>
          </div>
        </main>
      </div>
    </div>
  );
}
