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
import { AppIcon, Button, type AppView, type DashboardBusyAction, type ProviderAccessState, type RecoveryState, type Toast, type ToastKind, type NodeSort, type PlanDisplayModel, type HealthPanelData, type DiagnosticsBundle, type LogEntry } from "./dashboard/shared";
import { DashboardOverview } from "./dashboard/dashboard-overview";
import { NodesView } from "./dashboard/nodes-view";
import { HealthView } from "./dashboard/health-view";
import { SettingsView } from "./dashboard/settings-view";
import { DiagnosticsView } from "./dashboard/diagnostics-view";
import { DEFAULT_NODE_REGION, DEFAULT_PLAN_KEY, DEFAULT_PROVIDER_ID, DEFAULT_PROVIDER_NAME, VIEW_ROUTE, FALLBACK_PLAN_CATALOG, categorizeError, providerAccessBadge } from "./dashboard/shared";

export type { AppView } from "./dashboard/shared";

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
  const [health, setHealth] = useState<HealthPanelData | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsBundle | null>(null);
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
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadNodeDetails(selectedNodeId);
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [selectedNodeId]);

  useEffect(() => {
    if (!selectedNodeId) return;
    const timeoutId = window.setTimeout(() => {
      void refreshHealth();
    }, 0);
    return () => window.clearTimeout(timeoutId);
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
      setNodeFeedback("Select a node first.");
      return;
    }
    setBusyAction("create-tunnel");
    try {
      const result = await createAndActivateClientTunnel({
        tunnelId: "tunnel-1",
        nodeId: selectedNodeId,
        tunnelName: "wg-node-1",
        clientAddressCidr: "10.44.0.2/32",
        dnsServer: "1.1.1.1",
        allowedIps: ["0.0.0.0/0", "::/0"],
      });
      setNodeFeedback(result);
      setRecovery(null);
      pushToast("success", "Tunnel created", result);
      await loadDashboard();
    } catch (error) {
      setNodeFeedback(handleError(error));
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
  const selectedCreatePlanDisplay = selectedCreatePlan ? { planLabel: selectedCreatePlan.plan_label, sizeSlug: selectedCreatePlan.size_slug, vcpus: selectedCreatePlan.vcpus, memoryMb: selectedCreatePlan.memory_mb, diskGb: selectedCreatePlan.disk_gb, transferNote: selectedCreatePlan.transfer_note, transferTb: selectedCreatePlan.transfer_tb, hourlyUsd: selectedCreatePlan.hourly_usd, monthlyUsd: selectedCreatePlan.monthly_usd, estimated: selectedCreatePlan.estimated, sourceTimestamp: selectedCreatePlan.source_timestamp } : null;
  const selectedNodePricing = (() => {
    if (!selectedNode) return null;
    const regionPlans = pricingByRegion[selectedNode.region] ?? [];
    const matched = regionPlans.find((plan) => plan.plan_key === selectedNode.plan_key || plan.size_slug === selectedNode.size_slug);
    if (matched) {
      return { planLabel: matched.plan_label, sizeSlug: matched.size_slug, vcpus: matched.vcpus, memoryMb: matched.memory_mb, diskGb: matched.disk_gb, transferNote: matched.transfer_note, transferTb: matched.transfer_tb, hourlyUsd: matched.hourly_usd, monthlyUsd: matched.monthly_usd, estimated: matched.estimated, sourceTimestamp: matched.source_timestamp };
    }
    const fallback = Object.values(FALLBACK_PLAN_CATALOG).find((entry) => entry.sizeSlug === selectedNode.size_slug) ?? null;
    if (!fallback) return null;
    return { ...fallback, hourlyUsd: selectedNode.price_hourly_usd ?? fallback.hourlyUsd, monthlyUsd: selectedNode.price_monthly_usd ?? fallback.monthlyUsd, sourceTimestamp: selectedNode.price_source_timestamp ?? fallback.sourceTimestamp };
  })();
  const selectedNodeTunnel = (diagnostics?.tunnels ?? []).find((tunnel) => tunnel.node_id === selectedNodeId) ?? null;
  const selectedNodeActionMessage = selectedNode ? selectedNode.lifecycle_status === "error" ? `${selectedNode.error_reason ?? "Provisioning failed."} Next: retry validation, recreate the node, or use force cleanup if partial resources remain.` : selectedNode.lifecycle_status === "provisioning" ? "Provisioning is in progress. Avoid submitting another create request until this record moves to Ready or Error." : selectedNode.lifecycle_status === "bootstrapping" ? "Bootstrap is still running. Keep this page open or refresh to follow the progress updates." : "Use View Details to inspect endpoint health, pricing snapshot, and recovery actions for this node." : "Select a node to inspect health, pricing, and recovery options.";
  const diagnosticsLogs: LogEntry[] = diagnostics ? [
    { time: diagnostics.generated_at.slice(11, 19), level: "INFO", source: "NetGuard VPN Engine", message: "Diagnostics bundle generated", tone: "success" },
    { time: "14:02:11", level: "INFO", source: "Backend", message: `Nodes captured: ${diagnostics.nodes.length}`, tone: "info" },
    { time: "14:02:12", level: "WG", source: "WireGuard", message: `Tunnels captured: ${diagnostics.tunnels.length}`, tone: "neutral" },
    { time: "14:02:13", level: "WARN", source: "Recovery", message: recovery ? recovery.nextAction : "No recovery issue active", tone: recovery ? "warning" : "neutral" },
  ] : [
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
  const titleMap = { dashboard: "Network Stats", nodes: "Node Management", health: "Connection Health", diagnostics: "Diagnostics", settings: "Provider Integration" } as const;
  const subtitleMap = { dashboard: "Monitor the active tunnel, throughput, and local node state.", nodes: "Provision and manage DigitalOcean nodes and client tunnels.", health: "Inspect runtime handshake, endpoint, and tunnel health.", diagnostics: "Review redacted logs, recovery suggestions, and exportable bundles.", settings: "Register and validate the DigitalOcean token used for orchestration." } as const;
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
              <div className="flex h-12 w-12 items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[color:rgba(78,122,255,0.85)] text-white shadow-[0_8px_30px_rgba(78,122,255,0.28)]"><AppIcon name="shield" className="h-6 w-6" /></div>
              <div>
                <div className="text-xl font-semibold tracking-tight text-[var(--on-surface)]">GoatStare VPN</div>
                <div className="text-sm text-[var(--on-surface-variant)]">Enterprise Edition</div>
              </div>
            </div>
            <button type="button" onClick={() => setMobileNavOpen(false)} className="inline-flex h-9 w-9 items-center justify-center rounded-lg border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)] hover:bg-[color:rgba(255,255,255,0.08)]" aria-label="Close menu"><svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" className="fill-none stroke-current stroke-[1.8]" /></svg></button>
          </div>
          <nav className="mt-10 space-y-2">
            {navItems.map((item) => {
              const active = activeView === item.id;
              return <button key={`mobile-${item.id}`} type="button" onClick={() => navigateToView(item.id)} className={`flex w-full items-center gap-3 rounded-2xl border px-3 py-3 text-left transition-colors ${active ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "border-transparent text-[var(--on-surface-variant)] hover:bg-[color:rgba(255,255,255,0.04)] hover:text-[var(--on-surface)]"}`}><div className={`flex h-10 w-10 items-center justify-center rounded-xl ${active ? "bg-[color:rgba(171,199,255,0.2)]" : "bg-[color:rgba(255,255,255,0.04)]"}`}><AppIcon name={item.icon} className="h-5 w-5" /></div><div><div className="font-medium">{item.label}</div><div className="text-xs uppercase tracking-[0.24em] opacity-70">{item.id === "dashboard" ? "Overview" : item.id === "nodes" ? "Provision" : item.id === "health" ? "Connection" : item.id === "settings" ? "Provider" : "Logs"}</div></div></button>;
            })}
          </nav>
        </aside>
        <aside className="fixed inset-y-0 left-0 hidden w-[240px] flex-col border-r border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(17,20,28,0.98),rgba(13,16,23,0.98))] px-4 py-5 shadow-[20px_0_60px_rgba(0,0,0,0.25)] lg:flex">
          <div className="flex items-center gap-3 rounded-2xl px-1 py-1.5"><div className="flex h-12 w-12 items-center justify-center rounded-xl border border-[color:rgba(171,199,255,0.25)] bg-[color:rgba(78,122,255,0.85)] text-white shadow-[0_8px_30px_rgba(78,122,255,0.28)]"><AppIcon name="shield" className="h-6 w-6" /></div><div><div className="text-xl font-semibold tracking-tight text-[var(--on-surface)]">GoatStare VPN</div><div className="text-sm text-[var(--on-surface-variant)]">Enterprise Edition</div></div></div>
          <nav className="mt-10 space-y-2">{navItems.map((item) => { const active = activeView === item.id; return <button key={item.id} type="button" onClick={() => navigateToView(item.id)} className={`flex w-full items-center gap-3 rounded-2xl border px-3 py-3 text-left transition-colors ${active ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "border-transparent text-[var(--on-surface-variant)] hover:bg-[color:rgba(255,255,255,0.04)] hover:text-[var(--on-surface)]"}`}><div className={`flex h-10 w-10 items-center justify-center rounded-xl ${active ? "bg-[color:rgba(171,199,255,0.2)]" : "bg-[color:rgba(255,255,255,0.04)]"}`}><AppIcon name={item.icon} className="h-5 w-5" /></div><div><div className="font-medium">{item.label}</div><div className="text-xs uppercase tracking-[0.24em] opacity-70">{item.id === "dashboard" ? "Overview" : item.id === "nodes" ? "Provision" : item.id === "health" ? "Connection" : item.id === "settings" ? "Provider" : "Logs"}</div></div></button>; })}</nav>
          <div className="mt-10 rounded-[24px] border border-[color:rgba(171,199,255,0.22)] bg-[color:rgba(171,199,255,0.12)] p-4"><div className="flex items-center gap-2 text-[var(--primary)]"><AppIcon name="shield" className="h-4 w-4" /><span className="text-[11px] uppercase tracking-[0.28em]">Quick Connect</span></div><p className="mt-3 text-sm text-[var(--on-surface)]/90">Connect the active tunnel and refresh the live health state.</p><button type="button" onClick={() => navigateToView("dashboard")} className="mt-4 inline-flex items-center justify-center rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-2.5 text-sm font-medium text-[var(--on-surface)] transition-colors hover:bg-[color:rgba(255,255,255,0.06)]">Open dashboard</button></div>
          <div className="mt-auto space-y-2 pt-6 text-[var(--on-surface-variant)]"><button type="button" className="flex w-full items-center gap-3 rounded-xl px-2 py-2 text-left hover:bg-[color:rgba(255,255,255,0.04)] hover:text-[var(--on-surface)]"><AppIcon name="user" className="h-5 w-5" /><span>Account</span></button><button type="button" className="flex w-full items-center gap-3 rounded-xl px-2 py-2 text-left hover:bg-[color:rgba(255,255,255,0.04)] hover:text-[var(--on-surface)]"><AppIcon name="power" className="h-5 w-5" /><span>Logout</span></button></div>
        </aside>
        <main className="flex-1 lg:pl-[240px]"><div className="mx-auto flex min-h-screen w-full max-w-[1420px] flex-col gap-6 px-4 py-4 lg:px-6 lg:py-5"><div className="flex items-center justify-between rounded-2xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-4 py-3 text-sm text-[var(--on-surface-variant)] lg:hidden"><div className="font-medium text-[var(--on-surface)]">GoatStare VPN</div><button type="button" onClick={() => setMobileNavOpen((current) => !current)} className="inline-flex items-center gap-2 rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2 text-[var(--on-surface)]" aria-label="Open menu"><svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden="true"><path d="M4 7h16M4 12h16M4 17h16" className="fill-none stroke-current stroke-[1.8]" /></svg>Menu</button></div><header className="rounded-[28px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(16,19,27,0.85),rgba(14,17,24,0.85))] px-5 py-4 shadow-[var(--shadow-elevated)] backdrop-blur-md"><div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between"><div className="space-y-3"><div className="flex flex-wrap items-center gap-2 text-sm text-[var(--on-surface-variant)]"><span className="rounded-full border border-[color:var(--outline-variant)] px-3 py-1">Phase 1 Control Plane</span><span className="rounded-full border border-[color:var(--outline-variant)] px-3 py-1">Windows</span><span className="rounded-full border border-[color:var(--outline-variant)] px-3 py-1">DigitalOcean</span><span className="rounded-full border border-[color:var(--outline-variant)] px-3 py-1">WireGuard</span></div><div><h1 className="text-3xl font-semibold tracking-tight text-[var(--on-surface)]">{titleMap[activeView]}</h1><p className="mt-2 max-w-3xl text-sm leading-6 text-[var(--on-surface-variant)]">{subtitleMap[activeView]}</p></div></div><div className="flex flex-col items-start gap-3 xl:items-end"><div className="flex items-center gap-2"><Button tone="secondary" onClick={() => void loadDashboard()} disabled={busyAction !== null}><AppIcon name="refresh" className="h-4 w-4" />{busyAction === "refresh" ? "Refreshing" : "Refresh"}</Button><button type="button" className="inline-flex h-10 w-10 items-center justify-center rounded-full border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)] transition-colors hover:bg-[color:rgba(255,255,255,0.07)] hover:text-[var(--on-surface)]" aria-label="Notifications" title="Notifications"><AppIcon name="bell" className="h-5 w-5" /></button><button type="button" className="inline-flex h-10 w-10 items-center justify-center rounded-full border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface-variant)] transition-colors hover:bg-[color:rgba(255,255,255,0.07)] hover:text-[var(--on-surface)]" aria-label="Activity" title="Activity"><AppIcon name="activity" className="h-5 w-5" /></button></div><div className="flex flex-wrap gap-2 text-xs text-[var(--on-surface-variant)]"><div className={`rounded-full border px-3 py-1.5 ${heroTone === "primary" ? "border-[color:rgba(171,199,255,0.25)] bg-[color:rgba(171,199,255,0.08)] text-[var(--primary)]" : heroTone === "secondary" ? "border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.08)] text-[var(--secondary)]" : "border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] text-[var(--tertiary)]"}`}>Provider: {providerName}</div><div className="rounded-full border border-[color:rgba(78,222,163,0.25)] bg-[color:rgba(78,222,163,0.08)] px-3 py-1.5 text-[var(--secondary)]">Protocol: WireGuard</div><div className="rounded-full border border-[color:rgba(255,185,95,0.25)] bg-[color:rgba(255,185,95,0.08)] px-3 py-1.5 text-[var(--tertiary)]">Platform: Windows</div></div></div></div></header><div className="flex items-center justify-between rounded-[24px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] px-5 py-3 text-sm text-[var(--on-surface-variant)] shadow-[var(--shadow-elevated)]"><div className="flex items-center gap-3"><button type="button" onClick={() => navigateToView("dashboard")} className={`rounded-full px-3 py-1.5 transition-colors ${activeView === "dashboard" ? "bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "hover:text-[var(--on-surface)]"}`}>Network Stats</button><button type="button" onClick={() => navigateToView("diagnostics")} className={`rounded-full px-3 py-1.5 transition-colors ${activeView === "diagnostics" ? "bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "hover:text-[var(--on-surface)]"}`}>Activity Log</button></div><div className="text-xs uppercase tracking-[0.28em]">{recovery ? recovery.title : busyAction ? `Working: ${busyAction}` : "Idle"}</div></div>{activeView === "dashboard" ? <DashboardOverview activeView={activeView} noServerConfigured={noServerConfigured} activeNode={activeNode} connectedTunnel={connectedTunnel} health={health} canConnect={canConnect} recovery={recovery} busyAction={busyAction} onConnectTunnel={onConnectTunnel} onDisconnectTunnel={onDisconnectTunnel} onRefresh={loadDashboard} /> : null}{activeView === "settings" ? <SettingsView providerAccessState={providerAccessState} providerStatusMessage={providerStatusMessage} providerFeedback={providerFeedback} providerHasStoredToken={providerHasStoredToken} providerBadge={providerBadge} configuredProvider={configuredProvider} providerId={providerId} providerName={providerName} apiToken={apiToken} setApiToken={setApiToken} onSaveToken={onSaveToken} onValidateToken={onValidateToken} busyAction={busyAction} onNavigateToView={navigateToView} /> : null}{activeView === "nodes" ? <NodesView providerAccessState={providerAccessState} providerBadge={providerBadge} providerStatusMessage={providerStatusMessage} providerName={providerName} configuredProvider={configuredProvider} nodes={filteredNodes} diagnostics={diagnostics} pricingByRegion={pricingByRegion} pricingBusy={pricingBusy} pricingError={pricingError} nodeStatusFilter={nodeStatusFilter} setNodeStatusFilter={setNodeStatusFilter} nodeRegionFilter={nodeRegionFilter} setNodeRegionFilter={setNodeRegionFilter} nodeSort={nodeSort} setNodeSort={setNodeSort} selectedNodeId={selectedNodeId} setSelectedNodeId={setSelectedNodeId} selectedNode={selectedNode} health={health} selectedNodePricing={selectedNodePricing} selectedNodeActionMessage={selectedNodeActionMessage} selectedNodeTunnel={selectedNodeTunnel} busyAction={busyAction} onCreateNode={onCreateNode} onConnectTunnel={onConnectTunnel} onDisconnectTunnel={onDisconnectTunnel} onDestroyNode={onDestroyNode} onForceCleanupSelectedNode={onForceCleanupSelectedNode} onNavigateToView={navigateToView} setRegion={setRegion} region={region} setPlanKey={setPlanKey} planKey={planKey} setNodeId={setNodeId} nodeId={nodeId} setNodeName={setNodeName} nodeName={nodeName} nodeFeedback={nodeFeedback} providerId={providerId} nodesProvisioningDisabled={nodesProvisioningDisabled} nodeRegions={availableNodeRegions} nodeStatuses={nodeStatuses} pricingCatalog={selectedRegionPricing} setPricingByRegion={() => undefined} selectedCreatePlanDisplay={selectedCreatePlanDisplay} selectedCreatePlan={selectedCreatePlan} providerAccessStateValue={providerAccessState} /> : null}{activeView === "health" ? <HealthView diagnosticsLogs={diagnosticsLogs} health={health} selectedNode={selectedNode} selectedTunnelId={selectedTunnelId} busyAction={busyAction} onRefreshHealth={refreshHealth} /> : null}{activeView === "diagnostics" ? <DiagnosticsView diagnostics={diagnostics} diagnosticsExportPath={diagnosticsExportPath} recovery={recovery} busyAction={busyAction} onCopyDiagnostics={onCopyDiagnostics} onExportDiagnostics={onExportDiagnostics} /> : null}<div className={`fixed right-4 top-4 z-50 flex w-[min(92vw,24rem)] flex-col gap-3 ${toasts.length === 0 ? "pointer-events-none" : ""}`}>{toasts.map((toast) => <div key={toast.id} className={`rounded-[18px] border px-4 py-3 shadow-[var(--shadow-elevated)] backdrop-blur ${toast.kind === "success" ? "border-[color:rgba(78,222,163,0.2)] bg-[color:rgba(78,222,163,0.1)] text-[var(--secondary)]" : toast.kind === "warning" ? "border-[color:rgba(255,185,95,0.22)] bg-[color:rgba(255,185,95,0.12)] text-[var(--tertiary)]" : toast.kind === "error" ? "border-[color:rgba(255,107,107,0.22)] bg-[color:rgba(255,107,107,0.12)] text-[#ffd5d5]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.04)] text-[var(--on-surface)]"}`}><p className="text-sm font-semibold">{toast.title}</p><p className="mt-1 text-sm leading-5 opacity-90">{toast.message}</p></div>)}</div></div></main></div></div>);
}
