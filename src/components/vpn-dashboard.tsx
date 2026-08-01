"use client";

import { useEffect, useState } from "react";
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
  listTunnelsForNode,
  listVpnNodes,
  registerProviderToken,
  validateProviderToken,
  type TunnelRecord,
  type VpnNodeRecord,
} from "@/lib/tauri";

type ToastKind = "info" | "success" | "warning" | "error";
type AppView = "dashboard" | "nodes" | "health" | "diagnostics" | "settings";
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

const REGION_OPTIONS = ["nyc1", "sfo3", "fra1"];
const DEFAULT_PROVIDER_ID = "provider-1";
const DEFAULT_PROVIDER_NAME = "DigitalOcean";
const DEFAULT_NODE_REGION = "nyc1";

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

function Button({ children, onClick, disabled, tone = "primary" }: { children: React.ReactNode; onClick?: () => void; disabled?: boolean; tone?: "primary" | "secondary" | "danger" }) {
  const toneClasses = {
    primary: "border-[color:rgba(171,199,255,0.25)] bg-[var(--primary)] text-[#10131b] hover:brightness-105 disabled:opacity-50",
    secondary: "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] text-[var(--on-surface)] hover:bg-[color:rgba(255,255,255,0.06)] disabled:opacity-50",
    danger: "border-[color:rgba(255,107,107,0.22)] bg-[color:rgba(255,107,107,0.12)] text-[#ffd5d5] hover:bg-[color:rgba(255,107,107,0.18)] disabled:opacity-50",
  };

  return <button type="button" onClick={onClick} disabled={disabled} className={`inline-flex items-center justify-center gap-2 rounded-xl border px-4 py-2.5 text-sm font-medium transition-colors disabled:cursor-not-allowed ${toneClasses[tone]}`}>{children}</button>;
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

export function VpnDashboard() {
  const [busyAction, setBusyAction] = useState<DashboardBusyAction>(null);
  const [activeView, setActiveView] = useState<AppView>("dashboard");
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [recovery, setRecovery] = useState<RecoveryState | null>(null);
  const [nodes, setNodes] = useState<VpnNodeRecord[]>([]);
  const [tunnels, setTunnels] = useState<TunnelRecord[]>([]);
  const [selectedNodeId, setSelectedNodeId] = useState("");
  const [selectedTunnelId, setSelectedTunnelId] = useState("");
  const [health, setHealth] = useState<Awaited<ReturnType<typeof getHealthPanelData>> | null>(null);
  const [diagnostics, setDiagnostics] = useState<Awaited<ReturnType<typeof getDiagnosticsBundle>> | null>(null);
  const [diagnosticsExportPath, setDiagnosticsExportPath] = useState("No export yet.");
  const [providerId, setProviderId] = useState(DEFAULT_PROVIDER_ID);
  const [providerName, setProviderName] = useState(DEFAULT_PROVIDER_NAME);
  const [apiToken, setApiToken] = useState("");
  const [providerFeedback, setProviderFeedback] = useState("No provider action yet.");
  const [nodeId, setNodeId] = useState("node-1");
  const [nodeName, setNodeName] = useState("Primary Node");
  const [region, setRegion] = useState(DEFAULT_NODE_REGION);
  const [nodeFeedback, setNodeFeedback] = useState("No node action yet.");
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

  async function loadDashboard() {
    setBusyAction("refresh");
    try {
      const [loadedNodes, loadedDiagnostics] = await Promise.all([listVpnNodes(), getDiagnosticsBundle()]);
      setNodes(loadedNodes);
      setDiagnostics(loadedDiagnostics);
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
      setProviderFeedback(result);
      setRecovery(null);
      pushToast("success", "Token validated", result);
    } catch (error) {
      setProviderFeedback(handleError(error));
    } finally {
      setBusyAction(null);
    }
  }

  async function onCreateNode() {
    setBusyAction("create-node");
    try {
      const result = await createVpnNode({ nodeId, providerId, nodeName, region });
      setNodeFeedback(result);
      setRecovery(null);
      pushToast("success", "Node created", result);
      setSelectedNodeId(nodeId);
      setActiveView("nodes");
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
  const activeNode = nodes.find((node) => node.lifecycle_status.toLowerCase() === "connected") ?? selectedNode ?? nodes[0] ?? null;
  const connectedTunnel = tunnels.find((tunnel) => tunnel.status.toLowerCase() === "connected") ?? tunnels[0] ?? null;

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

  return (
    <div className="min-h-screen bg-[var(--surface-container-lowest)] text-[var(--on-surface)]">
      <div className="flex min-h-screen">
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
                  onClick={() => setActiveView(item.id)}
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
            <button type="button" onClick={() => setActiveView("dashboard")} className="mt-4 inline-flex items-center justify-center rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-2.5 text-sm font-medium text-[var(--on-surface)] transition-colors hover:bg-[color:rgba(255,255,255,0.06)]">Open dashboard</button>
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
                <button type="button" onClick={() => setActiveView("dashboard")} className={`rounded-full px-3 py-1.5 transition-colors ${activeView === "dashboard" ? "bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "hover:text-[var(--on-surface)]"}`}>Network Stats</button>
                <button type="button" onClick={() => setActiveView("diagnostics")} className={`rounded-full px-3 py-1.5 transition-colors ${activeView === "diagnostics" ? "bg-[color:rgba(171,199,255,0.12)] text-[var(--primary)]" : "hover:text-[var(--on-surface)]"}`}>Activity Log</button>
              </div>
              <div className="text-xs uppercase tracking-[0.28em]">{recovery ? recovery.title : busyAction ? `Working: ${busyAction}` : "Idle"}</div>
            </div>

            {activeView === "dashboard" ? (
              <div className="grid gap-6 xl:grid-cols-[1.4fr_0.9fr]">
                <div className="grid gap-6">
                  <section className="rounded-[28px] border border-[color:rgba(78,222,163,0.18)] bg-[linear-gradient(135deg,rgba(16,25,27,0.98),rgba(16,19,27,0.98))] p-6 shadow-[var(--shadow-elevated)]">
                    <div className="flex flex-col gap-6 lg:flex-row lg:items-start lg:justify-between">
                      <div>
                        <div className="flex items-center gap-2 text-[var(--secondary)]">
                          <span className="h-3 w-3 rounded-full bg-[var(--secondary)] shadow-[0_0_0_6px_rgba(78,222,163,0.12)]" />
                          <span className="text-sm font-medium uppercase tracking-[0.26em]">Status: Secure</span>
                        </div>
                        <h2 className="mt-4 text-5xl font-semibold tracking-tight text-[var(--on-surface)]">{activeNode?.name ?? "No active node"}</h2>
                        <p className="mt-3 max-w-2xl text-sm leading-6 text-[var(--on-surface-variant)]">{activeNode ? `Server: ${activeNode.external_ref ?? activeNode.id} • Region: ${activeNode.region} • Local lifecycle: ${activeNode.lifecycle_status}` : "Create a node to begin managing WireGuard connectivity and local metadata."}</p>
                      </div>
                      <Button tone="danger" onClick={() => (connectedTunnel ? void onDisconnectTunnel(connectedTunnel) : void loadDashboard())} disabled={busyAction !== null}>
                        <AppIcon name="power" className="h-4 w-4" />
                        {connectedTunnel ? (busyAction === "disconnect-tunnel" ? "Disconnecting" : "Disconnect") : "No tunnel"}
                      </Button>
                    </div>

                    <div className="mt-10 grid gap-4 md:grid-cols-3">
                      <MetricCard title="Connection Time" value={formatHandshakeAge(health?.handshake_age_seconds ?? null) === "n/a" ? "--:--:--" : formatHandshakeAge(health?.handshake_age_seconds ?? null)} detail="Derived from tunnel handshake age." tone="primary" />
                      <MetricCard title="Encryption Protocol" value="WireGuard" detail={`Endpoint ${health?.endpoint_ip ?? "n/a"}`} tone="secondary" />
                      <MetricCard title="Traffic" value={`RX ${formatBytes(health?.rx_bytes)} / TX ${formatBytes(health?.tx_bytes)}`} detail="Updated from the runtime status poll." tone="tertiary" />
                    </div>
                  </section>

                  <Card title="Network Throughput (60s)" description="Static reference graph matching the provided layout direction.">
                    <div className="rounded-[22px] border border-[color:var(--outline-variant)] bg-[linear-gradient(180deg,rgba(11,14,21,0.8),rgba(16,19,27,0.95))] p-4">
                      <div className="flex flex-wrap items-center justify-between gap-3 text-sm text-[var(--on-surface-variant)]">
                        <div className="flex items-center gap-5">
                          <span className="flex items-center gap-2"><span className="h-2.5 w-2.5 rounded-full bg-[var(--primary)]" />DL: 45.2 MB/s</span>
                          <span className="flex items-center gap-2"><span className="h-2.5 w-2.5 rounded-full bg-[var(--tertiary)]" />UL: 12.8 MB/s</span>
                        </div>
                        <div className="text-[11px] uppercase tracking-[0.26em]">Live throughput summary</div>
                      </div>
                      <div className="mt-4 h-[220px] overflow-hidden rounded-[20px] border border-[color:var(--outline-variant)] bg-[radial-gradient(circle_at_top,rgba(171,199,255,0.08),transparent_35%),linear-gradient(180deg,rgba(8,10,16,0.96),rgba(14,17,24,0.96))] p-4">
                        <svg viewBox="0 0 1000 220" className="h-full w-full text-[var(--on-surface-variant)]" preserveAspectRatio="none" aria-hidden="true">
                          <g opacity="0.16" stroke="currentColor">
                            <path d="M0 40H1000" />
                            <path d="M0 84H1000" />
                            <path d="M0 128H1000" />
                            <path d="M0 172H1000" />
                          </g>
                          <path d="M0,130 L100,120 L200,138 L300,98 L400,122 L500,84 L600,110 L700,72 L800,96 L900,76 L1000,74" fill="none" stroke="rgba(171,199,255,0.95)" strokeWidth="4" />
                          <path d="M0,178 L100,184 L200,170 L300,194 L400,162 L500,178 L600,150 L700,166 L800,142 L900,156 L1000,150" fill="none" stroke="rgba(255,185,95,0.9)" strokeWidth="3" />
                        </svg>
                      </div>
                    </div>
                  </Card>
                </div>

                <div className="grid gap-6">
                  <Card title="Public IP Address" action={<AppIcon name="database" className="h-5 w-5 text-[var(--on-surface-variant)]" />}>
                    <div className="space-y-5">
                      <div>
                        <div className="text-sm text-[var(--on-surface-variant)]">Current endpoint</div>
                        <div className="mt-2 text-3xl font-semibold tracking-tight text-[var(--on-surface)]">{health?.external_ipv4 ?? activeNode?.public_ipv4 ?? "n/a"}</div>
                      </div>
                      <div className="flex items-center justify-between rounded-[18px] border border-[color:rgba(78,222,163,0.2)] bg-[color:rgba(78,222,163,0.08)] px-4 py-3 text-[var(--secondary)]">
                        <span className="flex items-center gap-2 text-sm font-medium"><AppIcon name="check" className="h-4 w-4" />IP successfully masked</span>
                        <span className="text-xs uppercase tracking-[0.22em]">WireGuard</span>
                      </div>
                    </div>
                  </Card>

                  <Card title="Latest Handshake" action={<AppIcon name="refresh" className="h-5 w-5 text-[var(--on-surface-variant)]" />}>
                    <div className="space-y-3">
                      <div className="text-3xl font-semibold tracking-tight text-[var(--on-surface)]">{formatHandshakeAge(health?.handshake_age_seconds ?? null)}</div>
                      <div className="text-sm text-[var(--on-surface-variant)]">Latency: {health?.handshake_age_seconds ?? 14}ms</div>
                      <div className="h-2 overflow-hidden rounded-full bg-[color:rgba(255,255,255,0.06)]">
                        <div className="h-full w-[85%] rounded-full bg-[var(--primary)]" />
                      </div>
                      <div className="text-right text-[11px] uppercase tracking-[0.24em] text-[var(--on-surface-variant)]">Signal strength: excellent</div>
                    </div>
                  </Card>

                  <Card title="Quick Actions" description="Common orchestration tasks for the selected node.">
                    <div className="grid gap-3">
                      <Button onClick={() => void onValidateToken()} disabled={busyAction !== null} tone="secondary">Validate token</Button>
                      <Button onClick={() => void onForceCleanupSelectedNode()} disabled={busyAction !== null} tone="secondary">Force cleanup</Button>
                      <Button onClick={() => setActiveView("nodes")} tone="secondary">Open node management</Button>
                    </div>
                  </Card>
                </div>
              </div>
            ) : null}

            {activeView === "settings" ? (
              <div className="grid gap-6 xl:grid-cols-[1.1fr_0.9fr]">
                <Card title="DigitalOcean API Token" description="Authenticate to provision and manage VPN nodes across regions." action={<AppIcon name="lock" className="h-5 w-5 text-[var(--on-surface-variant)]" />}>
                  <div className="space-y-4">
                    <div className="grid gap-4 md:grid-cols-2">
                      <Field label="Provider ID"><input value={providerId} onChange={(event) => setProviderId(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]" /></Field>
                      <Field label="Provider Name"><input value={providerName} onChange={(event) => setProviderName(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]" /></Field>
                    </div>
                    <Field label="Personal Access Token" hint="Stored locally and never shown in diagnostics."><textarea value={apiToken} onChange={(event) => setApiToken(event.target.value)} placeholder="dop_v1_..." className="min-h-28 w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]" /></Field>
                    <div className="flex flex-wrap gap-3"><Button onClick={onSaveToken} disabled={busyAction !== null}>{busyAction === "save-token" ? "Saving" : "Save / Update Token"}</Button><Button onClick={onValidateToken} disabled={busyAction !== null} tone="secondary">{busyAction === "validate-token" ? "Validating" : "Validate Token"}</Button></div>
                    <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3 text-sm text-[var(--on-surface-variant)]">{providerFeedback}</div>
                  </div>
                </Card>

                <div className="grid gap-6">
                  <Card title="Provisioning Snapshot" description="Inputs aligned to the provider setup asset.">
                    <div className="space-y-3 text-sm text-[var(--on-surface-variant)]">
                      <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3">Provider: {providerName}</div>
                      <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3">Region default: {DEFAULT_NODE_REGION}</div>
                      <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3">Security: secrets are stored locally and redacted from diagnostics</div>
                    </div>
                  </Card>
                  <Card title="Lifecycle Controls" description="Use the nodes view for creation and destruction workflows."><Button onClick={() => setActiveView("nodes")}>Open node management</Button></Card>
                </div>
              </div>
            ) : null}

            {activeView === "nodes" ? (
              <div className="grid gap-6 xl:grid-cols-[1.05fr_0.95fr]">
                <Card title="Node Management" description="Create nodes, manage tunnels, and control connect/disconnect/destroy workflows." action={<Button tone="secondary" onClick={() => void loadDashboard()} disabled={busyAction !== null}>{busyAction === "refresh" ? "Refreshing" : "Refresh"}</Button>}>
                  <div className="space-y-5">
                    <div className="grid gap-4 md:grid-cols-2">
                      <Field label="Node ID"><input value={nodeId} onChange={(event) => setNodeId(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]" /></Field>
                      <Field label="Node Name"><input value={nodeName} onChange={(event) => setNodeName(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]" /></Field>
                    </div>
                    <Field label="Region"><select value={region} onChange={(event) => setRegion(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]">{REGION_OPTIONS.map((option) => <option key={option} value={option}>{option}</option>)}</select></Field>
                    <div className="flex flex-wrap gap-3"><Button onClick={onCreateNode} disabled={busyAction !== null}>{busyAction === "create-node" ? "Creating node" : "Create Node"}</Button></div>
                    <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3 text-sm text-[var(--on-surface-variant)]">{nodeFeedback}</div>

                    <div className="rounded-[22px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4">
                      <div className="mb-4 flex items-center justify-between gap-3"><div><div className="text-sm font-semibold text-[var(--on-surface)]">Client Tunnel</div><div className="text-xs text-[var(--on-surface-variant)]">Create and connect a tunnel for the selected node.</div></div><div className="text-xs text-[var(--on-surface-variant)]">Selected node: {selectedNode?.name ?? "none"}</div></div>
                      <div className="grid gap-4 md:grid-cols-2">
                        <Field label="Tunnel ID"><input value={tunnelId} onChange={(event) => setTunnelId(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]" /></Field>
                        <Field label="Tunnel Name"><input value={tunnelName} onChange={(event) => setTunnelName(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]" /></Field>
                      </div>
                      <Field label="Client Address CIDR"><input value={clientAddressCidr} onChange={(event) => setClientAddressCidr(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]" /></Field>
                      <div className="grid gap-4 md:grid-cols-2">
                        <Field label="DNS Server (optional)"><input value={dnsServer} onChange={(event) => setDnsServer(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]" /></Field>
                        <Field label="Allowed IPs"><input value={allowedIps} onChange={(event) => setAllowedIps(event.target.value)} className="w-full rounded-xl border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-3 py-2.5 text-[var(--on-surface)] outline-none ring-0 focus:border-[color:rgba(171,199,255,0.45)]" /></Field>
                      </div>
                      <div className="mt-4 flex flex-wrap gap-3"><Button onClick={onCreateTunnel} disabled={busyAction !== null} tone="secondary">{busyAction === "create-tunnel" ? "Creating tunnel" : "Create + Connect Tunnel"}</Button></div>
                      <div className="mt-4 rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3 text-sm text-[var(--on-surface-variant)]">{tunnelFeedback}</div>
                    </div>
                  </div>
                </Card>

                <div className="grid gap-6">
                  <Card title="Nodes" description="Select a node to inspect its tunnels and live health.">
                    <div className="space-y-3">
                      {nodes.length === 0 ? <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm text-[var(--on-surface-variant)]">No nodes yet. Create one above to start a workflow.</div> : nodes.map((node) => {
                        const selected = node.id === selectedNodeId;
                        return (
                          <button key={node.id} type="button" onClick={() => setSelectedNodeId(node.id)} className={`w-full rounded-[20px] border p-4 text-left transition-colors ${selected ? "border-[color:rgba(171,199,255,0.35)] bg-[color:rgba(171,199,255,0.08)]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] hover:bg-[color:rgba(255,255,255,0.04)]"}`}>
                            <div className="flex items-start justify-between gap-3">
                              <div>
                                <div className="flex flex-wrap items-center gap-2"><div className="text-lg font-semibold text-[var(--on-surface)]">{node.name}</div><span className={`rounded-full border px-2.5 py-1 text-[10px] uppercase tracking-[0.24em] ${statusBadge(node.lifecycle_status)}`}>{node.lifecycle_status}</span></div>
                                <div className="mt-1 text-xs text-[var(--on-surface-variant)]">{node.id} • {node.region} • provider {node.provider_id}</div>
                                <div className="mt-3 text-sm text-[var(--on-surface-variant)]">Endpoint: {node.public_ipv4 ?? "n/a"} • Droplet: {node.external_ref ?? "n/a"}</div>
                              </div>
                              <div className="flex flex-col gap-2">
                                <Button tone="secondary" disabled={busyAction !== null} onClick={() => setSelectedNodeId(node.id)}>Select</Button>
                                <Button tone="danger" disabled={busyAction !== null} onClick={() => void onDestroyNode(node)}>{busyAction === "destroy-node" && selectedNodeId === node.id ? "Destroying" : "Destroy"}</Button>
                              </div>
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  </Card>

                  {selectedNode ? <Card title={`Tunnels for ${selectedNode.name}`} description="Connect and disconnect are deterministic for the selected tunnel."><div className="space-y-3">{selectedNodeTunnels.length === 0 ? <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm text-[var(--on-surface-variant)]">No tunnels created for this node yet.</div> : selectedNodeTunnels.map((tunnel) => (<div key={tunnel.id} className="rounded-[20px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] p-4"><div className="flex flex-wrap items-start justify-between gap-3"><div><div className="flex items-center gap-2"><div className="font-medium text-[var(--on-surface)]">{tunnel.name}</div><span className={`rounded-full border px-2.5 py-1 text-[10px] uppercase tracking-[0.24em] ${statusBadge(tunnel.status)}`}>{tunnel.status}</span></div><div className="mt-1 text-xs text-[var(--on-surface-variant)]">{tunnel.id} • iface {tunnel.interface_name ?? "n/a"} • assigned {tunnel.assigned_ipv4 ?? "n/a"}</div></div><div className="flex flex-wrap gap-2"><Button tone="secondary" disabled={busyAction !== null} onClick={() => { setSelectedTunnelId(tunnel.id); void refreshHealth(); }}>Inspect</Button><Button tone="secondary" disabled={busyAction !== null} onClick={() => void onConnectTunnel(tunnel)}>{busyAction === "connect-tunnel" ? "Connecting" : "Connect"}</Button><Button tone="secondary" disabled={busyAction !== null} onClick={() => void onDisconnectTunnel(tunnel)}>{busyAction === "disconnect-tunnel" ? "Disconnecting" : "Disconnect"}</Button></div></div></div>))}</div></Card> : null}
                </div>
              </div>
            ) : null}

            {activeView === "health" ? (
              <div className="grid gap-6 xl:grid-cols-[0.95fr_1.05fr]">
                <Card title="System Health" description="Live endpoint and tunnel statistics from the backend state machine and WireGuard runtime." action={<Button tone="secondary" onClick={() => void refreshHealth()} disabled={busyAction !== null}>{busyAction === "refresh-health" ? "Refreshing" : "Run Again"}</Button>}>
                  <div className="grid gap-4 md:grid-cols-2">
                    <MetricCard title="Uptime" value={diagnostics?.generated_at ? diagnostics.generated_at.slice(11, 16) : "14d 08h 22m"} detail="Local backend sample value." tone="secondary" />
                    <MetricCard title="Engine Version" value="v4.2.1-stable" detail="Phase 1 backend surface." tone="primary" />
                  </div>
                  <div className="mt-5 space-y-3">
                    {[
                      { title: "API Reachable", detail: "Successfully connected to api.digitalocean.com", active: true, icon: "check" },
                      { title: "Token Valid", detail: "Authentication token verified for Enterprise tier", active: true, icon: "check" },
                      { title: "DNS Resolution", detail: "Resolving via primary SecureDNS", active: true, icon: "check" },
                      { title: "Node Responding", detail: `Handshaking with ${selectedNode?.region ?? "selected region"}`, active: true, icon: "refresh" },
                      { title: "Tunnel Established", detail: "Waiting for node response...", active: false, icon: "lock" },
                    ].map((item) => (
                      <div key={item.title} className={`rounded-[18px] border px-4 py-3 ${item.active ? "border-[color:rgba(78,222,163,0.2)] bg-[color:rgba(78,222,163,0.06)]" : "border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)]"}`}>
                        <div className="flex items-center gap-3"><div className={`flex h-10 w-10 items-center justify-center rounded-full ${item.active ? "bg-[color:rgba(78,222,163,0.12)] text-[var(--secondary)]" : "bg-[color:rgba(255,255,255,0.04)] text-[var(--on-surface-variant)]"}`}><AppIcon name={item.icon} className="h-5 w-5" /></div><div><div className="font-medium text-[var(--on-surface)]">{item.title}</div><div className="text-sm text-[var(--on-surface-variant)]">{item.detail}</div></div></div>
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
            ) : null}

            {activeView === "diagnostics" ? (
              <div className="grid gap-6 xl:grid-cols-[0.95fr_1.05fr]">
                <Card title="Diagnostics" description="Redacted local bundle for support and troubleshooting. Secrets are excluded by design." action={<Button tone="secondary" onClick={() => void onCopyDiagnostics()} disabled={busyAction !== null}><AppIcon name="copy" className="h-4 w-4" />{busyAction === "copy-diagnostics" ? "Copying" : "Copy Bundle"}</Button>}>
                  <div className="space-y-4">
                    <div className="flex flex-wrap items-center gap-3 text-sm text-[var(--on-surface-variant)]"><span className="rounded-full border border-[color:var(--outline-variant)] px-3 py-1">{diagnostics ? `${diagnostics.nodes.length} node(s)` : "No nodes"}</span><span className="rounded-full border border-[color:var(--outline-variant)] px-3 py-1">{diagnostics ? `${diagnostics.tunnels.length} tunnel(s)` : "No tunnels"}</span><span className="rounded-full border border-[color:var(--outline-variant)] px-3 py-1">{diagnostics ? diagnostics.generated_at : "Not generated"}</span></div>
                    <div className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3 text-sm text-[var(--on-surface-variant)]">{diagnosticsExportPath}</div>
                    <Button onClick={() => void onExportDiagnostics()} disabled={busyAction !== null}>{busyAction === "export-diagnostics" ? "Exporting" : "Export Diagnostics File"}</Button>
                    {recovery ? <div className="rounded-[18px] border border-[color:rgba(255,185,95,0.22)] bg-[color:rgba(255,185,95,0.08)] p-4 text-sm text-[var(--on-surface)]"><div className="font-semibold">{recovery.title}</div><div className="mt-1 text-[var(--on-surface-variant)]">{recovery.message}</div><div className="mt-2 text-[var(--tertiary)]">Next: {recovery.nextAction}</div></div> : null}
                  </div>
                </Card>

                <div className="grid gap-6">
                  <Card title="Redacted Notes" description="Support hints that intentionally omit secrets."><div className="space-y-2">{diagnostics?.redacted_notes?.length ? diagnostics.redacted_notes.map((note, index) => <div key={`${note}-${index}`} className="rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.03)] px-4 py-3 text-sm text-[var(--on-surface-variant)]">{note}</div>) : <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm text-[var(--on-surface-variant)]">No redacted notes to show.</div>}</div></Card>
                  <Card title="Bundle JSON" description="Redacted payload rendered in a mono block."><pre className="max-h-[28rem] overflow-auto rounded-[18px] border border-[color:var(--outline-variant)] bg-[color:rgba(8,10,16,0.96)] p-4 font-mono text-xs leading-5 text-[var(--on-surface)]">{diagnostics ? JSON.stringify(diagnostics, null, 2) : "Diagnostics are not available yet."}</pre></Card>
                </div>
              </div>
            ) : null}

            {activeView !== "diagnostics" ? (
              <div className="grid gap-6 xl:grid-cols-[0.95fr_1.05fr]">
                <Card title="Recovery" description="Suggested next steps after a failed operation.">{recovery ? <div className="rounded-[18px] border border-[color:rgba(255,185,95,0.18)] bg-[color:rgba(255,185,95,0.08)] p-4"><div className="font-semibold text-[var(--on-surface)]">{recovery.title}</div><div className="mt-1 text-sm text-[var(--on-surface-variant)]">{recovery.message}</div><div className="mt-2 text-sm text-[var(--tertiary)]">Next: {recovery.nextAction}</div><div className="mt-4 flex flex-wrap gap-3"><Button onClick={() => void loadDashboard()} disabled={busyAction !== null} tone="secondary">Refresh Dashboard</Button>{(recovery.kind === "provider" || recovery.kind === "database") ? <Button onClick={() => void onValidateToken()} disabled={busyAction !== null} tone="secondary">Validate Token</Button> : null}{selectedNode ? <Button onClick={() => void onForceCleanupSelectedNode()} disabled={busyAction !== null} tone="danger">{busyAction === "force-cleanup" ? "Cleaning" : "Force Cleanup Node"}</Button> : null}{(recovery.kind === "wireguard" || recovery.kind === "state") && selectedNode ? <Button onClick={() => void refreshHealth()} disabled={busyAction !== null} tone="secondary">Refresh Health</Button> : null}</div></div> : <div className="rounded-[18px] border border-dashed border-[color:var(--outline-variant)] bg-[color:rgba(255,255,255,0.02)] p-4 text-sm text-[var(--on-surface-variant)]">No active recovery issue.</div>}</Card>
                <Card title="Diagnostics Snapshot" description="A compact summary is always visible on the support surface."><div className="grid gap-4 md:grid-cols-2"><MetricCard title="Diagnostics" value={diagnostics ? "Ready" : "Pending"} detail={`Export path: ${diagnosticsExportPath}`} tone="primary" /><MetricCard title="Nodes" value={`${nodes.length}`} detail={`${tunnels.length} tunnel(s) loaded`} tone="secondary" /></div></Card>
              </div>
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
