"use client";

import {
  connectTunnel as connectTunnelCommand,
  createVpnNode,
  destroyVpnNode,
  disconnectTunnel as disconnectTunnelCommand,
  exportDiagnosticsBundle,
  forceCleanupVpnNode,
  getDiagnosticsBundle,
  getHealthPanelData,
  listProviders,
  listTunnelsForNode,
  listVpnNodes,
  registerProviderToken,
  validateProviderToken,
  type ProviderRecord,
  type TunnelRecord,
  type VpnNodeRecord,
} from "@/lib/tauri";
import {
  useCallback,
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  categorizeError,
  DEFAULT_PROVIDER_ID,
  DEFAULT_PROVIDER_NAME,
  type DashboardBusyAction,
  type DiagnosticsBundle,
  type HealthPanelData,
  type ProviderAccessState,
  type RecoveryState,
  type Toast,
  type ToastKind,
} from "./shared";

type CreateNodeInput = {
  nodeId: string;
  nodeName: string;
  region: string;
  planKey: string;
};

type DashboardContextValue = {
  busyAction: DashboardBusyAction;
  toasts: Toast[];
  recovery: RecoveryState | null;
  nodes: VpnNodeRecord[];
  providers: ProviderRecord[];
  tunnels: TunnelRecord[];
  selectedNodeId: string;
  setSelectedNodeId: (value: string) => void;
  selectedTunnelId: string;
  setSelectedTunnelId: (value: string) => void;
  health: HealthPanelData | null;
  diagnostics: DiagnosticsBundle | null;
  diagnosticsExportPath: string;
  providerId: string;
  providerName: string;
  providerAccessState: ProviderAccessState;
  providerStatusMessage: string;
  providerFeedback: string;
  configuredProvider: ProviderRecord | null;
  providerHasStoredToken: boolean;
  selectedNode: VpnNodeRecord | null;
  activeNode: VpnNodeRecord | null;
  connectedTunnel: TunnelRecord | null;
  noServerConfigured: boolean;
  canConnect: boolean;
  loadDashboard: (preferredSelectedNodeId?: string) => Promise<void>;
  refreshHealth: () => Promise<void>;
  saveToken: (apiToken: string) => Promise<void>;
  validateToken: () => Promise<void>;
  createNode: (input: CreateNodeInput) => Promise<void>;
  connectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  disconnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  destroyNode: (node: VpnNodeRecord) => Promise<void>;
  forceCleanupSelectedNode: () => Promise<void>;
  copyDiagnostics: () => Promise<void>;
  exportDiagnostics: () => Promise<void>;
};

const DashboardContext = createContext<DashboardContextValue | null>(null);

export function DashboardProvider({ children }: { children: ReactNode }) {
  const [busyAction, setBusyAction] = useState<DashboardBusyAction>(null);
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
  const [providerFeedback, setProviderFeedback] = useState("No provider action yet.");
  const selectedNodeIdRef = useRef(selectedNodeId);
  const selectedTunnelIdRef = useRef(selectedTunnelId);

  const selectedNode = nodes.find((node) => node.id === selectedNodeId) ?? null;

  useEffect(() => {
    selectedNodeIdRef.current = selectedNodeId;
    selectedTunnelIdRef.current = selectedTunnelId;
  }, [selectedNodeId, selectedTunnelId]);

  const pushToast = useCallback((kind: ToastKind, title: string, message: string) => {
    const id = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    setToasts((current) => [...current, { id, kind, title, message }]);
    window.setTimeout(() => {
      setToasts((current) => current.filter((toast) => toast.id !== id));
    }, 5000);
  }, []);

  const handleError = useCallback((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    const classification = categorizeError(message);
    setRecovery({ ...classification, message });
    pushToast(
      classification.kind === "state" ? "warning" : "error",
      classification.title,
      `${message} ${classification.nextAction}`,
    );
    return message;
  }, [pushToast]);

  const refreshProviderAccessStatus = useCallback(async (provider: ProviderRecord | null) => {
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
  }, []);

  const loadDashboard = useCallback(async (preferredSelectedNodeId?: string) => {
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

      const preferredExists = preferredSelectedNodeId
        ? loadedNodes.some((node) => node.id === preferredSelectedNodeId)
        : false;
      const currentExists = loadedNodes.some((node) => node.id === selectedNodeIdRef.current);
      const nextSelectedNodeId = preferredExists
        ? preferredSelectedNodeId ?? ""
        : currentExists
          ? selectedNodeIdRef.current
          : loadedNodes[0]?.id ?? "";

      setSelectedNodeId(nextSelectedNodeId);

      if (!nextSelectedNodeId) {
        setTunnels([]);
        setSelectedTunnelId("");
        setHealth(null);
      }
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, refreshProviderAccessStatus]);

  const loadNodeDetails = useCallback(async (nodeIdValue: string) => {
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
        const selectedExists = loadedTunnels.some((tunnel) => tunnel.id === selectedTunnelIdRef.current);
        setSelectedTunnelId(selectedExists ? selectedTunnelIdRef.current : loadedTunnels[0].id);
      } else {
        setSelectedTunnelId("");
      }

      setHealth(await getHealthPanelData(nodeIdValue, loadedTunnels[0]?.id ?? null));
    } catch (error) {
      handleError(error);
    }
  }, [handleError]);

  const refreshHealth = useCallback(async () => {
    if (!selectedNodeIdRef.current) {
      return;
    }

    setBusyAction("refresh-health");
    try {
      setHealth(await getHealthPanelData(selectedNodeIdRef.current, selectedTunnelIdRef.current || null));
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError]);

  const saveToken = useCallback(async (apiToken: string) => {
    setBusyAction("save-token");
    try {
      setProviderFeedback("Saving token reference locally...");
      const result = await registerProviderToken(providerId, providerName, apiToken);
      setProviderFeedback(result);
      setRecovery(null);
      pushToast("success", "Token saved", result);
      await loadDashboard();
    } catch (error) {
      setProviderFeedback(handleError(error));
    } finally {
      setBusyAction(null);
    }
  }, [loadDashboard, providerId, providerName, pushToast, handleError]);

  const validateToken = useCallback(async () => {
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
  }, [handleError, providerId, pushToast]);

  const createNode = useCallback(async (input: CreateNodeInput) => {
    setBusyAction("create-node");
    try {
      const result = await createVpnNode({
        nodeId: input.nodeId,
        providerId,
        nodeName: input.nodeName,
        region: input.region,
        planKey: input.planKey,
      });
      setRecovery(null);
      pushToast("success", "Node created", result);
      await loadDashboard(input.nodeId);
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, providerId, pushToast]);

  const connectTunnel = useCallback(async (tunnel: TunnelRecord) => {
    setBusyAction("connect-tunnel");
    try {
      await connectTunnelCommand(tunnel.node_id, tunnel.id);
      setRecovery(null);
      pushToast("success", "Tunnel connected", `${tunnel.name} is now connected.`);
      await loadDashboard(tunnel.node_id);
      await refreshHealth();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, pushToast, refreshHealth]);

  const disconnectTunnel = useCallback(async (tunnel: TunnelRecord) => {
    setBusyAction("disconnect-tunnel");
    try {
      await disconnectTunnelCommand(tunnel.node_id, tunnel.id);
      setRecovery(null);
      pushToast("success", "Tunnel disconnected", `${tunnel.name} is now ready.`);
      await loadDashboard(tunnel.node_id);
      await refreshHealth();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, pushToast, refreshHealth]);

  const destroyNode = useCallback(async (node: VpnNodeRecord) => {
    const confirmed = window.confirm(`Destroy node ${node.name} (${node.id})? This removes local state and attempts droplet cleanup.`);
    if (!confirmed) {
      return;
    }

    setBusyAction("destroy-node");
    try {
      await destroyVpnNode(node.id);
      setRecovery(null);
      pushToast("warning", "Node destroyed", `${node.name} has been removed.`);
      await loadDashboard();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, pushToast]);

  const forceCleanupSelectedNode = useCallback(async () => {
    if (!selectedNode) {
      pushToast("warning", "No node selected", "Select a node before forcing cleanup.");
      return;
    }

    setBusyAction("force-cleanup");
    try {
      const result = await forceCleanupVpnNode(selectedNode.id);
      setRecovery(null);
      pushToast(
        "success",
        "Force cleanup complete",
        `Removed ${result.local_tunnels_removed} tunnel(s) for ${selectedNode.name}.`,
      );
      await loadDashboard(selectedNode.id);
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, pushToast, selectedNode]);

  const copyDiagnostics = useCallback(async () => {
    if (!diagnostics) {
      return;
    }

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
  }, [diagnostics, handleError, pushToast]);

  const exportDiagnostics = useCallback(async () => {
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
  }, [handleError, pushToast]);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadDashboard();
    }, 0);

    return () => window.clearTimeout(timeoutId);
  }, [loadDashboard]);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadNodeDetails(selectedNodeId);
    }, 0);

    return () => window.clearTimeout(timeoutId);
  }, [loadNodeDetails, selectedNodeId]);

  useEffect(() => {
    if (!selectedNodeId) {
      return;
    }

    const timeoutId = window.setTimeout(() => {
      void refreshHealth();
    }, 0);

    return () => window.clearTimeout(timeoutId);
  }, [refreshHealth, selectedNodeId, selectedTunnelId]);

  const configuredProvider = providers.find((provider) => provider.id === providerId) ?? providers[0] ?? null;
  const providerHasStoredToken = !!configuredProvider?.token_ref && providerAccessState !== "invalid";
  const activeNode = nodes.find((node) => node.lifecycle_status.toLowerCase() === "connected") ?? selectedNode ?? nodes[0] ?? null;
  const connectedTunnel = tunnels.find((tunnel) => tunnel.status.toLowerCase() === "connected") ?? tunnels[0] ?? null;
  const noServerConfigured = nodes.length === 0;
  const canConnect = !!connectedTunnel && connectedTunnel.status.toLowerCase() !== "connected";

  const value: DashboardContextValue = {
    busyAction,
    toasts,
    recovery,
    nodes,
    providers,
    tunnels,
    selectedNodeId,
    setSelectedNodeId,
    selectedTunnelId,
    setSelectedTunnelId,
    health,
    diagnostics,
    diagnosticsExportPath,
    providerId,
    providerName,
    providerAccessState,
    providerStatusMessage,
    providerFeedback,
    configuredProvider,
    providerHasStoredToken,
    selectedNode,
    activeNode,
    connectedTunnel,
    noServerConfigured,
    canConnect,
    loadDashboard,
    refreshHealth,
    saveToken,
    validateToken,
    createNode,
    connectTunnel,
    disconnectTunnel,
    destroyNode,
    forceCleanupSelectedNode,
    copyDiagnostics,
    exportDiagnostics,
  };

  return <DashboardContext.Provider value={value}>{children}</DashboardContext.Provider>;
}

export function useDashboard() {
  const context = useContext(DashboardContext);

  if (!context) {
    throw new Error("useDashboard must be used within DashboardProvider.");
  }

  return context;
}