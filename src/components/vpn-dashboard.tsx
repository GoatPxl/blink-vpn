"use client";

import { useEffect, useState } from "react";
import {
  connectTunnel,
  createAndActivateClientTunnel,
  createVpnNode,
  destroyVpnNode,
  exportDiagnosticsBundle,
  getDiagnosticsBundle,
  getHealthPanelData,
  listTunnelsForNode,
  listVpnNodes,
  disconnectTunnel,
  registerProviderToken,
  TunnelRecord,
  VpnNodeRecord,
  validateProviderToken,
} from "@/lib/tauri";

type ToastKind = "info" | "success" | "warning" | "error";

type Toast = {
  id: string;
  kind: ToastKind;
  title: string;
  message: string;
};

type DashboardBusyAction =
  | "refresh"
  | "save-token"
  | "validate-token"
  | "create-node"
  | "create-tunnel"
  | "connect-tunnel"
  | "disconnect-tunnel"
  | "destroy-node"
  | "copy-diagnostics"
  | "refresh-health"
  | null;

type ErrorCategory = "provider" | "database" | "wireguard" | "state" | "unknown";

const REGION_OPTIONS = ["nyc1", "sfo3", "fra1"];
const DEFAULT_PROVIDER_ID = "provider-1";
const DEFAULT_PROVIDER_NAME = "DigitalOcean";
const DEFAULT_NODE_REGION = "nyc1";

function statusBadge(status: string) {
  const normalized = status.toLowerCase();
  if (normalized === "ready" || normalized === "connected") {
    return "border-emerald-200 bg-emerald-50 text-emerald-700";
  }
  if (normalized === "bootstrapping" || normalized === "provisioning" || normalized === "deleting") {
    return "border-amber-200 bg-amber-50 text-amber-700";
  }
  if (normalized === "error") {
    return "border-rose-200 bg-rose-50 text-rose-700";
  }
  return "border-slate-200 bg-slate-50 text-slate-700";
}

function categorizeError(errorMessage: string): { kind: ErrorCategory; title: string; nextAction: string } {
  if (errorMessage.includes("Provider orchestration failed") || errorMessage.includes("Provider provider")) {
    return {
      kind: "provider",
      title: "Provider issue",
      nextAction: "Verify the DigitalOcean token and try again.",
    };
  }
  if (errorMessage.includes("Database operation failed") || errorMessage.includes("was not found")) {
    return {
      kind: "database",
      title: "Local state issue",
      nextAction: "Refresh the dashboard and confirm the local SQLite state.",
    };
  }
  if (errorMessage.includes("WireGuard operation failed") || errorMessage.includes("WireGuard is not installed")) {
    return {
      kind: "wireguard",
      title: "WireGuard issue",
      nextAction: "Install or repair WireGuard for Windows and retry.",
    };
  }
  if (errorMessage.includes("Node create flow cannot proceed") || errorMessage.includes("InvalidState")) {
    return {
      kind: "state",
      title: "Lifecycle issue",
      nextAction: "Resolve the node or tunnel state mismatch and retry.",
    };
  }

  return {
    kind: "unknown",
    title: "Operation failed",
    nextAction: "Retry the action or refresh the dashboard.",
  };
}

function formatBytes(value: number | null | undefined): string {
  if (value === null || value === undefined) {
    return "n/a";
  }

  if (value < 1024) {
    return `${value} B`;
  }

  const kib = value / 1024;
  if (kib < 1024) {
    return `${kib.toFixed(1)} KiB`;
  }

  const mib = kib / 1024;
  return `${mib.toFixed(1)} MiB`;
}

function formatHandshakeAge(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined) {
    return "n/a";
  }
  if (seconds === 0) {
    return "just now";
  }
  if (seconds < 60) {
    return `${seconds}s ago`;
  }
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m ago`;
  }
  const hours = Math.floor(minutes / 60);
  return `${hours}h ago`;
}

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-2xl border border-slate-200 bg-white/90 p-5 shadow-sm backdrop-blur">
      <div className="mb-4">
        <h2 className="text-lg font-semibold text-slate-900">{title}</h2>
        <p className="text-sm text-slate-600">{description}</p>
      </div>
      {children}
    </section>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block space-y-1 text-sm font-medium text-slate-700">
      <span>{label}</span>
      {children}
    </label>
  );
}

function Button({
  children,
  onClick,
  disabled,
  tone = "primary",
}: {
  children: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  tone?: "primary" | "secondary" | "danger";
}) {
  const toneClasses = {
    primary: "bg-slate-900 text-white hover:bg-slate-800 disabled:bg-slate-500",
    secondary: "border border-slate-300 bg-white text-slate-800 hover:bg-slate-50 disabled:bg-slate-100",
    danger: "bg-rose-600 text-white hover:bg-rose-500 disabled:bg-rose-300",
  };

  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`inline-flex items-center justify-center rounded-xl px-4 py-2 text-sm font-medium transition-colors disabled:cursor-not-allowed ${toneClasses[tone]}`}
    >
      {children}
    </button>
  );
}

export function VpnDashboard() {
  const [busyAction, setBusyAction] = useState<DashboardBusyAction>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [nodes, setNodes] = useState<VpnNodeRecord[]>([]);
  const [tunnels, setTunnels] = useState<TunnelRecord[]>([]);
  const [selectedNodeId, setSelectedNodeId] = useState<string>("");
  const [selectedTunnelId, setSelectedTunnelId] = useState<string>("");
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
    window.setTimeout(() => {
      setToasts((current) => current.filter((toast) => toast.id !== id));
    }, 5000);
  }

  function handleError(error: unknown) {
    const message = error instanceof Error ? error.message : String(error);
    const classification = categorizeError(message);
    pushToast(classification.kind === "unknown" ? "error" : classification.kind === "state" ? "warning" : "error", classification.title, `${message} ${classification.nextAction}`);
    return message;
  }

  async function loadDashboard() {
    setBusyAction("refresh");
    try {
      const [loadedNodes, loadedDiagnostics] = await Promise.all([listVpnNodes(), getDiagnosticsBundle()]);
      setNodes(loadedNodes);
      setDiagnostics(loadedDiagnostics);

      if (!selectedNodeId && loadedNodes.length > 0) {
        setSelectedNodeId(loadedNodes[0].id);
      }
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

      const healthData = await getHealthPanelData(nodeIdValue, loadedTunnels[0]?.id ?? null);
      setHealth(healthData);
    } catch (error) {
      handleError(error);
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
    if (!selectedNodeId) {
      return;
    }

    void refreshHealth();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedTunnelId]);

  async function refreshHealth() {
    if (!selectedNodeId) {
      return;
    }

    setBusyAction("refresh-health");
    try {
      const value = await getHealthPanelData(selectedNodeId, selectedTunnelId || null);
      setHealth(value);
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }

  async function onSaveToken() {
    setBusyAction("save-token");
    try {
      setProviderFeedback("Saving token reference locally...");
      const result = await registerProviderToken(providerId, providerName, apiToken);
      setProviderFeedback(result);
      pushToast("success", "Token saved", result);
      setApiToken("");
      await loadDashboard();
    } catch (error) {
      const message = handleError(error);
      setProviderFeedback(message);
    } finally {
      setBusyAction(null);
    }
  }

  async function onValidateToken() {
    setBusyAction("validate-token");
    try {
      const result = await validateProviderToken(providerId);
      setProviderFeedback(result);
      pushToast("success", "Token validated", result);
    } catch (error) {
      const message = handleError(error);
      setProviderFeedback(message);
    } finally {
      setBusyAction(null);
    }
  }

  async function onCreateNode() {
    setBusyAction("create-node");
    try {
      const result = await createVpnNode({
        nodeId,
        providerId,
        nodeName,
        region,
      });
      setNodeFeedback(result);
      pushToast("success", "Node created", result);
      setSelectedNodeId(nodeId);
      await loadDashboard();
    } catch (error) {
      const message = handleError(error);
      setNodeFeedback(message);
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
        allowedIps: allowedIps
          .split(",")
          .map((value) => value.trim())
          .filter(Boolean),
      });
      setTunnelFeedback(result);
      pushToast("success", "Tunnel created", result);
      await loadDashboard();
    } catch (error) {
      const message = handleError(error);
      setTunnelFeedback(message);
    } finally {
      setBusyAction(null);
    }
  }

  async function onConnectTunnel(tunnel: TunnelRecord) {
    setBusyAction("connect-tunnel");
    try {
      await connectTunnel(tunnel.node_id, tunnel.id);
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
    if (!confirmed) {
      return;
    }

    setBusyAction("destroy-node");
    try {
      await destroyVpnNode(node.id);
      pushToast("warning", "Node destroyed", `${node.name} has been removed.`);
      if (selectedNodeId === node.id) {
        setSelectedNodeId("");
      }
      await loadDashboard();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }

  async function onCopyDiagnostics() {
    if (!diagnostics) {
      return;
    }

    setBusyAction("copy-diagnostics");
    try {
      const bundleText = JSON.stringify(diagnostics, null, 2);
      await navigator.clipboard.writeText(bundleText);
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
      pushToast("success", "Diagnostics exported", path);
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }

  const selectedNode = nodes.find((node) => node.id === selectedNodeId) ?? null;
  const selectedNodeTunnels = tunnels;

  return (
    <div className="relative min-h-screen overflow-hidden bg-[linear-gradient(180deg,#f8fafc_0%,#eef2ff_100%)] text-slate-900">
      <div className="absolute inset-x-0 top-0 h-72 bg-[radial-gradient(circle_at_top_right,rgba(37,99,235,0.15),transparent_60%)]" />
      <div className="mx-auto flex w-full max-w-7xl flex-col gap-6 px-4 py-6 md:px-6 lg:px-8">
        <header className="rounded-3xl border border-slate-200 bg-white/90 px-6 py-5 shadow-sm backdrop-blur">
          <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
            <div>
              <p className="text-xs font-semibold uppercase tracking-[0.22em] text-slate-500">
                Phase 1 Control Plane
              </p>
              <h1 className="mt-2 text-3xl font-semibold tracking-tight text-slate-950">
                GoatStare VPN Manager
              </h1>
              <p className="mt-2 max-w-3xl text-sm leading-6 text-slate-600">
                Windows desktop orchestration for DigitalOcean and WireGuard with a simple, state-driven UI.
              </p>
            </div>
            <div className="flex items-center gap-2 text-xs text-slate-600">
              <span className="rounded-full border border-slate-200 bg-slate-50 px-3 py-1">Provider: DigitalOcean</span>
              <span className="rounded-full border border-slate-200 bg-slate-50 px-3 py-1">Protocol: WireGuard</span>
              <span className="rounded-full border border-slate-200 bg-slate-50 px-3 py-1">Platform: Windows</span>
            </div>
          </div>
        </header>

        <div className="grid gap-6 lg:grid-cols-2">
          <Section
            title="Provider Token"
            description="Add or update the DigitalOcean token, then validate it before provisioning nodes."
          >
            <div className="grid gap-3 md:grid-cols-2">
              <Field label="Provider ID">
                <input
                  value={providerId}
                  onChange={(event) => setProviderId(event.target.value)}
                  className="rounded-xl border border-slate-300 bg-white px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
                />
              </Field>
              <Field label="Provider Name">
                <input
                  value={providerName}
                  onChange={(event) => setProviderName(event.target.value)}
                  className="rounded-xl border border-slate-300 bg-white px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
                />
              </Field>
            </div>
            <Field label="DigitalOcean API Token">
              <textarea
                value={apiToken}
                onChange={(event) => setApiToken(event.target.value)}
                className="min-h-28 rounded-xl border border-slate-300 bg-white px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
                placeholder="dop_v1_..."
              />
            </Field>
            <div className="mt-4 flex flex-wrap gap-3">
              <Button onClick={onSaveToken} disabled={busyAction !== null}>
                {busyAction === "save-token" ? "Saving..." : "Save / Update Token"}
              </Button>
              <Button onClick={onValidateToken} disabled={busyAction !== null} tone="secondary">
                {busyAction === "validate-token" ? "Validating..." : "Validate Token"}
              </Button>
            </div>
            <p className="mt-4 rounded-xl border border-slate-200 bg-slate-50 px-3 py-2 text-sm text-slate-700">
              {providerFeedback}
            </p>
          </Section>

          <Section
            title="Node Management"
            description="Create nodes, manage tunnels, and control connect/disconnect/destroy workflows."
          >
            <div className="grid gap-3 md:grid-cols-2">
              <Field label="Node ID">
                <input
                  value={nodeId}
                  onChange={(event) => setNodeId(event.target.value)}
                  className="rounded-xl border border-slate-300 bg-white px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
                />
              </Field>
              <Field label="Node Name">
                <input
                  value={nodeName}
                  onChange={(event) => setNodeName(event.target.value)}
                  className="rounded-xl border border-slate-300 bg-white px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
                />
              </Field>
            </div>
            <Field label="Region">
              <select
                value={region}
                onChange={(event) => setRegion(event.target.value)}
                className="rounded-xl border border-slate-300 bg-white px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
              >
                {REGION_OPTIONS.map((option) => (
                  <option key={option} value={option}>
                    {option}
                  </option>
                ))}
              </select>
            </Field>
            <div className="mt-4 flex flex-wrap gap-3">
              <Button onClick={onCreateNode} disabled={busyAction !== null}>
                {busyAction === "create-node" ? "Creating node..." : "Create Node"}
              </Button>
            </div>
            <p className="mt-4 rounded-xl border border-slate-200 bg-slate-50 px-3 py-2 text-sm text-slate-700">
              {nodeFeedback}
            </p>

            <div className="mt-5 rounded-2xl border border-slate-200 bg-slate-50 p-4">
              <div className="mb-3 flex items-center justify-between gap-3">
                <div>
                  <p className="text-sm font-semibold text-slate-900">Client Tunnel</p>
                  <p className="text-xs text-slate-600">Create and connect a tunnel for the selected node.</p>
                </div>
                <p className="text-xs text-slate-500">Selected node: {selectedNode?.name ?? "none"}</p>
              </div>
              <div className="grid gap-3 md:grid-cols-2">
                <Field label="Tunnel ID">
                  <input
                    value={tunnelId}
                    onChange={(event) => setTunnelId(event.target.value)}
                    className="rounded-xl border border-slate-300 bg-white px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
                  />
                </Field>
                <Field label="Tunnel Name">
                  <input
                    value={tunnelName}
                    onChange={(event) => setTunnelName(event.target.value)}
                    className="rounded-xl border border-slate-300 bg-white px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
                  />
                </Field>
              </div>
              <Field label="Client Address CIDR">
                <input
                  value={clientAddressCidr}
                  onChange={(event) => setClientAddressCidr(event.target.value)}
                  className="rounded-xl border border-slate-300 bg-white px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
                />
              </Field>
              <div className="grid gap-3 md:grid-cols-2">
                <Field label="DNS Server (optional)">
                  <input
                    value={dnsServer}
                    onChange={(event) => setDnsServer(event.target.value)}
                    className="rounded-xl border border-slate-300 bg-white px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
                  />
                </Field>
                <Field label="Allowed IPs (comma separated)">
                  <input
                    value={allowedIps}
                    onChange={(event) => setAllowedIps(event.target.value)}
                    className="rounded-xl border border-slate-300 bg-white px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
                  />
                </Field>
              </div>
              <div className="mt-4 flex flex-wrap gap-3">
                <Button onClick={onCreateTunnel} disabled={busyAction !== null} tone="secondary">
                  {busyAction === "create-tunnel" ? "Creating tunnel..." : "Create + Connect Tunnel"}
                </Button>
              </div>
              <p className="mt-4 rounded-xl border border-slate-200 bg-white px-3 py-2 text-sm text-slate-700">
                {tunnelFeedback}
              </p>
            </div>
          </Section>
        </div>

        <div className="grid gap-6 xl:grid-cols-2">
          <Section title="Nodes" description="Select a node to inspect its tunnels and live health, then use connect/disconnect or destroy actions.">
            <div className="mb-4 flex items-center justify-between gap-3">
              <p className="text-sm text-slate-600">{nodes.length} node(s) loaded</p>
              <Button onClick={loadDashboard} disabled={busyAction !== null} tone="secondary">
                {busyAction === "refresh" ? "Refreshing..." : "Refresh"}
              </Button>
            </div>
            <div className="space-y-3">
              {nodes.length === 0 ? (
                <div className="rounded-xl border border-dashed border-slate-300 bg-slate-50 p-4 text-sm text-slate-600">
                  No nodes yet. Create one above to start a VPN node workflow.
                </div>
              ) : (
                nodes.map((node) => (
                  <button
                    key={node.id}
                    type="button"
                    onClick={() => setSelectedNodeId(node.id)}
                    className={`w-full rounded-2xl border p-4 text-left transition-colors ${
                      node.id === selectedNodeId ? "border-slate-900 bg-slate-100" : "border-slate-200 bg-white hover:bg-slate-50"
                    }`}
                  >
                    <div className="flex flex-wrap items-start justify-between gap-3">
                      <div>
                        <div className="flex items-center gap-2">
                          <h3 className="font-semibold text-slate-900">{node.name}</h3>
                          <span className={`rounded-full border px-2.5 py-0.5 text-xs font-medium ${statusBadge(node.lifecycle_status)}`}>
                            {node.lifecycle_status}
                          </span>
                        </div>
                        <p className="mt-1 text-xs text-slate-500">{node.id} • {node.region} • provider {node.provider_id}</p>
                        <p className="mt-2 text-sm text-slate-600">
                          Endpoint: {node.public_ipv4 ?? "n/a"} • Droplet: {node.external_ref ?? "n/a"}
                        </p>
                      </div>
                      <div className="flex flex-wrap gap-2">
                        <Button
                          tone="secondary"
                          disabled={busyAction !== null}
                          onClick={() => setSelectedNodeId(node.id)}
                        >
                          Select
                        </Button>
                        <Button
                          tone="danger"
                          disabled={busyAction !== null}
                          onClick={() => void onDestroyNode(node)}
                        >
                          {busyAction === "destroy-node" && selectedNodeId === node.id ? "Destroying..." : "Destroy"}
                        </Button>
                      </div>
                    </div>
                  </button>
                ))
              )}
            </div>
            {selectedNode ? (
              <div className="mt-5 rounded-2xl border border-slate-200 bg-slate-50 p-4">
                <div className="mb-3 flex items-center justify-between gap-3">
                  <div>
                    <p className="text-sm font-semibold text-slate-900">Tunnels for {selectedNode.name}</p>
                    <p className="text-xs text-slate-600">Connect and disconnect are deterministic for the selected tunnel.</p>
                  </div>
                  <p className="text-xs text-slate-500">{selectedNodeTunnels.length} tunnel(s)</p>
                </div>
                <div className="space-y-2">
                  {selectedNodeTunnels.length === 0 ? (
                    <p className="rounded-xl border border-dashed border-slate-300 bg-white p-3 text-sm text-slate-600">
                      No tunnels created for this node yet.
                    </p>
                  ) : (
                    selectedNodeTunnels.map((tunnel) => (
                      <div key={tunnel.id} className="rounded-xl border border-slate-200 bg-white p-3">
                        <div className="flex flex-wrap items-center justify-between gap-3">
                          <div>
                            <div className="flex items-center gap-2">
                              <p className="font-medium text-slate-900">{tunnel.name}</p>
                              <span className={`rounded-full border px-2.5 py-0.5 text-xs font-medium ${statusBadge(tunnel.status)}`}>
                                {tunnel.status}
                              </span>
                            </div>
                            <p className="mt-1 text-xs text-slate-500">
                              {tunnel.id} • iface {tunnel.interface_name ?? "n/a"} • assigned {tunnel.assigned_ipv4 ?? "n/a"}
                            </p>
                          </div>
                          <div className="flex flex-wrap gap-2">
                            <Button
                              tone="secondary"
                              disabled={busyAction !== null}
                              onClick={() => {
                                setSelectedTunnelId(tunnel.id);
                                void refreshHealth();
                              }}
                            >
                              Inspect
                            </Button>
                            <Button
                              tone="secondary"
                              disabled={busyAction !== null}
                              onClick={() => void onConnectTunnel(tunnel)}
                            >
                              {busyAction === "connect-tunnel" ? "Connecting..." : "Connect"}
                            </Button>
                            <Button
                              tone="secondary"
                              disabled={busyAction !== null}
                              onClick={() => void onDisconnectTunnel(tunnel)}
                            >
                              {busyAction === "disconnect-tunnel" ? "Disconnecting..." : "Disconnect"}
                            </Button>
                          </div>
                        </div>
                      </div>
                    ))
                  )}
                </div>
              </div>
            ) : null}
          </Section>

          <Section title="Health" description="Live endpoint and tunnel statistics from the backend state machine and WireGuard runtime.">
            <div className="mb-4 flex items-center justify-between gap-3">
              <p className="text-sm text-slate-600">Selected node health</p>
              <Button onClick={() => void refreshHealth()} disabled={busyAction !== null} tone="secondary">
                {busyAction === "refresh-health" ? "Refreshing..." : "Refresh Health"}
              </Button>
            </div>
            {health ? (
              <div className="grid gap-3 md:grid-cols-2">
                <div className="rounded-2xl border border-slate-200 bg-white p-4">
                  <p className="text-xs uppercase tracking-[0.2em] text-slate-500">Endpoint</p>
                  <p className="mt-2 text-lg font-semibold text-slate-900">{health.endpoint_ip ?? "n/a"}</p>
                  <p className="mt-1 text-sm text-slate-600">Node: {health.node_id}</p>
                </div>
                <div className="rounded-2xl border border-slate-200 bg-white p-4">
                  <p className="text-xs uppercase tracking-[0.2em] text-slate-500">External IP</p>
                  <p className="mt-2 text-lg font-semibold text-slate-900">{health.external_ipv4 ?? "n/a"}</p>
                  <p className="mt-1 text-sm text-slate-600">Tunnel: {health.tunnel_id ?? "n/a"}</p>
                </div>
                <div className="rounded-2xl border border-slate-200 bg-white p-4">
                  <p className="text-xs uppercase tracking-[0.2em] text-slate-500">Handshake</p>
                  <p className="mt-2 text-lg font-semibold text-slate-900">{formatHandshakeAge(health.handshake_age_seconds)}</p>
                  <p className="mt-1 text-sm text-slate-600">Tunnel status: {health.tunnel_status ?? "n/a"}</p>
                  <p className="mt-1 text-xs text-slate-500">Selected tunnel: {selectedTunnelId || "none"}</p>
                </div>
                <div className="rounded-2xl border border-slate-200 bg-white p-4">
                  <p className="text-xs uppercase tracking-[0.2em] text-slate-500">Traffic</p>
                  <p className="mt-2 text-lg font-semibold text-slate-900">
                    RX {formatBytes(health.rx_bytes)} / TX {formatBytes(health.tx_bytes)}
                  </p>
                  <p className="mt-1 text-sm text-slate-600">Updated by runtime status poll</p>
                </div>
              </div>
            ) : (
              <div className="rounded-xl border border-dashed border-slate-300 bg-slate-50 p-4 text-sm text-slate-600">
                Select a node to load health details.
              </div>
            )}
          </Section>
        </div>

        <Section title="Diagnostics" description="Redacted local bundle for support and troubleshooting. Secrets are excluded by design.">
          <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
            <div className="text-sm text-slate-600">
              {diagnostics ? (
                <span>
                  Generated at <span className="font-medium text-slate-900">{diagnostics.generated_at}</span> • {diagnostics.nodes.length} node(s) • {diagnostics.tunnels.length} tunnel(s)
                </span>
              ) : (
                <span>No diagnostics bundle loaded.</span>
              )}
            </div>
            <Button onClick={() => void onCopyDiagnostics()} disabled={busyAction !== null} tone="secondary">
              {busyAction === "copy-diagnostics" ? "Copying..." : "Copy Diagnostics Bundle"}
            </Button>
            <Button onClick={() => void onExportDiagnostics()} disabled={busyAction !== null}>
              {busyAction === "export-diagnostics" ? "Exporting..." : "Export Diagnostics File"}
            </Button>
          </div>
          <p className="mb-4 rounded-xl border border-slate-200 bg-white px-3 py-2 text-sm text-slate-700">
            {diagnosticsExportPath}
          </p>
          {diagnostics ? (
            <div className="grid gap-4 lg:grid-cols-2">
              <div className="rounded-2xl border border-slate-200 bg-slate-50 p-4">
                <p className="text-sm font-semibold text-slate-900">Redacted notes</p>
                <ul className="mt-3 space-y-2 text-sm text-slate-700">
                  {diagnostics.redacted_notes.length === 0 ? (
                    <li className="rounded-xl border border-dashed border-slate-300 bg-white p-3 text-slate-600">No redacted notes to show.</li>
                  ) : (
                    diagnostics.redacted_notes.map((note, index) => (
                      <li key={`${note}-${index}`} className="rounded-xl border border-slate-200 bg-white p-3">
                        {note}
                      </li>
                    ))
                  )}
                </ul>
              </div>
              <div className="rounded-2xl border border-slate-200 bg-slate-900 p-4 text-slate-100">
                <p className="text-sm font-semibold text-white">Bundle JSON</p>
                <pre className="mt-3 max-h-[26rem] overflow-auto whitespace-pre-wrap break-words text-xs leading-5 text-slate-200">
                  {JSON.stringify(diagnostics, null, 2)}
                </pre>
              </div>
            </div>
          ) : (
            <div className="rounded-xl border border-dashed border-slate-300 bg-slate-50 p-4 text-sm text-slate-600">
              Diagnostics are not available yet.
            </div>
          )}
        </Section>
      </div>

      <div className="fixed right-4 top-4 z-50 flex w-[min(92vw,24rem)] flex-col gap-3">
        {toasts.map((toast) => (
          <div
            key={toast.id}
            className={`rounded-2xl border px-4 py-3 shadow-lg backdrop-blur ${
              toast.kind === "success"
                ? "border-emerald-200 bg-emerald-50 text-emerald-900"
                : toast.kind === "warning"
                  ? "border-amber-200 bg-amber-50 text-amber-900"
                  : toast.kind === "error"
                    ? "border-rose-200 bg-rose-50 text-rose-900"
                    : "border-slate-200 bg-white text-slate-900"
            }`}
          >
            <p className="text-sm font-semibold">{toast.title}</p>
            <p className="mt-1 text-sm leading-5 opacity-90">{toast.message}</p>
          </div>
        ))}
      </div>
    </div>
  );
}
