"use client";

import {
  checkDockerReadiness,
  connectTunnel as connectTunnelCommand,
  createAndActivateClientTunnel,
  deleteProvider as deleteProviderCommand,
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
  resumeProvider as resumeProviderCommand,
  suspendProvider as suspendProviderCommand,
  validateProviderToken,
  type DockerReadiness,
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

type ThroughputSample = {
  timestamp: number;
  downloadBytesPerSecond: number;
  uploadBytesPerSecond: number;
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
  selectProviderId: (providerId: string) => void;
  providerAccessState: ProviderAccessState;
  providerStatusMessage: string;
  providerFeedback: string;
  dockerDevEnabled: boolean;
  dockerReadinessState: ProviderAccessState;
  dockerReadinessMessage: string;
  dockerReadiness: DockerReadiness | null;
  dockerFauxTokenConfigured: boolean;
  dockerFauxTokenConfiguredAt: string | null;
  configuredProvider: ProviderRecord | null;
  providerHasStoredToken: boolean;
  selectedNode: VpnNodeRecord | null;
  activeNode: VpnNodeRecord | null;
  connectedTunnel: TunnelRecord | null;
  noServerConfigured: boolean;
  canConnect: boolean;
  throughputSamples: ThroughputSample[];
  loadDashboard: (preferredSelectedNodeId?: string) => Promise<void>;
  refreshHealth: () => Promise<void>;
  saveToken: (apiToken: string) => Promise<void>;
  validateToken: () => Promise<void>;
  suspendProvider: (provider: ProviderRecord) => Promise<void>;
  resumeProvider: (provider: ProviderRecord) => Promise<void>;
  deleteProvider: (provider: ProviderRecord) => Promise<void>;
  saveDockerFauxToken: (value: string) => Promise<void>;
  validateDockerReadiness: (options?: { silent?: boolean }) => Promise<void>;
  createNode: (input: CreateNodeInput) => Promise<void>;
  installHostForNode: (node: VpnNodeRecord) => Promise<void>;
  uninstallHostForNode: (node: VpnNodeRecord) => Promise<void>;
  connectPrimary: () => Promise<void>;
  connectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  disconnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  destroyNode: (node: VpnNodeRecord) => Promise<void>;
  connectNode: (node: VpnNodeRecord) => Promise<void>;
  forceCleanupSelectedNode: () => Promise<void>;
  copyDiagnostics: () => Promise<void>;
  exportDiagnostics: () => Promise<void>;
};

const DashboardContext = createContext<DashboardContextValue | null>(null);

const DOCKER_FAUX_TOKEN_STORAGE_KEY = "goatstare.docker.fauxTokenConfiguredAt";

export function DashboardProvider({ children }: { children: ReactNode }) {
  const dockerDevEnabled = process.env.NEXT_PUBLIC_DEV_DOCKER === "1";
  const [busyAction, setBusyAction] = useState<DashboardBusyAction>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [recovery, setRecovery] = useState<RecoveryState | null>(null);
  const [nodes, setNodes] = useState<VpnNodeRecord[]>([]);
  const [providers, setProviders] = useState<ProviderRecord[]>([]);
  const [tunnels, setTunnels] = useState<TunnelRecord[]>([]);
  const [selectedNodeId, setSelectedNodeId] = useState("");
  const [selectedTunnelId, setSelectedTunnelId] = useState("");
  const [health, setHealth] = useState<HealthPanelData | null>(null);
  const [throughputSamples, setThroughputSamples] = useState<ThroughputSample[]>([]);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsBundle | null>(null);
  const [diagnosticsExportPath, setDiagnosticsExportPath] = useState("No export yet.");
  const [providerId, setProviderId] = useState(DEFAULT_PROVIDER_ID);
  const [providerName, setProviderName] = useState(DEFAULT_PROVIDER_NAME);
  const [providerAccessState, setProviderAccessState] = useState<ProviderAccessState>("unknown");
  const [providerStatusMessage, setProviderStatusMessage] = useState("Provider status has not been checked yet.");
  const [providerFeedback, setProviderFeedback] = useState("No provider action yet.");
  const [dockerReadinessState, setDockerReadinessState] = useState<ProviderAccessState>(
    dockerDevEnabled ? "checking" : "unknown",
  );
  const [dockerReadinessMessage, setDockerReadinessMessage] = useState(
    dockerDevEnabled
      ? "Docker readiness has not been checked yet."
      : "Docker development mode is disabled.",
  );
  const [dockerReadiness, setDockerReadiness] = useState<DockerReadiness | null>(null);
  const [dockerFauxTokenConfigured, setDockerFauxTokenConfigured] = useState(() => {
    if (!dockerDevEnabled || typeof window === "undefined") {
      return false;
    }

    return !!window.localStorage.getItem(DOCKER_FAUX_TOKEN_STORAGE_KEY);
  });
  const [dockerFauxTokenConfiguredAt, setDockerFauxTokenConfiguredAt] = useState<string | null>(() => {
    if (!dockerDevEnabled || typeof window === "undefined") {
      return null;
    }

    return window.localStorage.getItem(DOCKER_FAUX_TOKEN_STORAGE_KEY);
  });
  const providerIdRef = useRef(providerId);
  const selectedNodeIdRef = useRef(selectedNodeId);
  const selectedTunnelIdRef = useRef(selectedTunnelId);
  const lastTrafficSampleRef = useRef<{
    timestamp: number;
    nodeId: string;
    tunnelId: string;
    rxBytes: number;
    txBytes: number;
  } | null>(null);

  const selectedNode = nodes.find((node) => node.id === selectedNodeId) ?? null;
  const activeNode = nodes.find((node) => node.lifecycle_status.toLowerCase() === "connected") ?? selectedNode ?? nodes[0] ?? null;
  const connectedTunnel = tunnels.find((tunnel) => tunnel.status.toLowerCase() === "connected") ?? tunnels[0] ?? null;
  const noServerConfigured = nodes.length === 0;
  const canConnect = !!activeNode && (!connectedTunnel || connectedTunnel.status.toLowerCase() !== "connected");

  useEffect(() => {
    providerIdRef.current = providerId;
    selectedNodeIdRef.current = selectedNodeId;
    selectedTunnelIdRef.current = selectedTunnelId;
  }, [providerId, selectedNodeId, selectedTunnelId]);

  const pushToast = useCallback((kind: ToastKind, title: string, message: string) => {
    const id = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    setToasts((current) => [...current, { id, kind, title, message }]);
    window.setTimeout(() => {
      setToasts((current) => current.filter((toast) => toast.id !== id));
    }, 5000);
  }, []);

  const handleError = useCallback((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    const explicitNextIndex = message.indexOf(" Next: ");
    const explicitNext = explicitNextIndex >= 0
      ? message.slice(explicitNextIndex + " Next: ".length).trim()
      : null;
    const baseMessage = explicitNextIndex >= 0
      ? message.slice(0, explicitNextIndex).trim()
      : message;
    const classification = categorizeError(message);
    setRecovery({
      ...classification,
      message: baseMessage,
      nextAction: explicitNext ?? classification.nextAction,
    });
    pushToast(
      classification.kind === "state" ? "warning" : "error",
      classification.title,
      explicitNext
        ? `${baseMessage} Next: ${explicitNext}`
        : `${baseMessage} ${classification.nextAction}`,
    );
    return explicitNext
      ? `${baseMessage} Next: ${explicitNext}`
      : `${baseMessage} ${classification.nextAction}`;
  }, [pushToast]);

  const clearThroughputSamples = useCallback(() => {
    lastTrafficSampleRef.current = null;
    setThroughputSamples([]);
  }, []);

  const recordThroughputSample = useCallback((snapshot: HealthPanelData) => {
    const now = Date.now();
    const nodeId = snapshot.node_id;
    const tunnelId = snapshot.tunnel_id ?? "";
    const rxBytes = snapshot.rx_bytes ?? 0;
    const txBytes = snapshot.tx_bytes ?? 0;
    const previous = lastTrafficSampleRef.current;

    let downloadBytesPerSecond = 0;
    let uploadBytesPerSecond = 0;

    if (
      previous
      && previous.nodeId === nodeId
      && previous.tunnelId === tunnelId
      && now > previous.timestamp
    ) {
      const elapsedSeconds = (now - previous.timestamp) / 1000;
      if (elapsedSeconds > 0) {
        downloadBytesPerSecond = Math.max(0, (rxBytes - previous.rxBytes) / elapsedSeconds);
        uploadBytesPerSecond = Math.max(0, (txBytes - previous.txBytes) / elapsedSeconds);
      }
    }

    lastTrafficSampleRef.current = {
      timestamp: now,
      nodeId,
      tunnelId,
      rxBytes,
      txBytes,
    };

    setThroughputSamples((current) => [
      ...current,
      {
        timestamp: now,
        downloadBytesPerSecond,
        uploadBytesPerSecond,
      },
    ].slice(-12));
  }, []);

  const refreshProviderAccessStatus = useCallback(async (provider: ProviderRecord | null) => {
    const providerLabel = provider?.name ?? "provider";
    if (!provider?.token_ref) {
      setProviderAccessState("missing");
      setProviderStatusMessage(`No ${providerLabel} token is stored yet. Save and validate a token in Settings to provision nodes.`);
      return;
    }

    if (provider.status === "suspended") {
      setProviderAccessState("invalid");
      setProviderStatusMessage(`Provider ${providerLabel} is suspended. Resume it in Settings before running this operation.`);
      return;
    }

    if (provider.slug === "docker") {
      setProviderAccessState("checking");
      setProviderStatusMessage("Checking Docker daemon readiness for local provider operations...");
      try {
        const readiness = await checkDockerReadiness();
        if (readiness.installed && readiness.daemon_reachable) {
          setProviderAccessState("ready");
        } else {
          setProviderAccessState("invalid");
        }
        setProviderStatusMessage(readiness.message);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setProviderAccessState("invalid");
        setProviderStatusMessage(message);
      }
      return;
    }

    setProviderAccessState("checking");
    setProviderStatusMessage(`Checking stored provider access for ${providerLabel}...`);

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

      const configuredProvider = loadedProviders.find((provider) => provider.id === providerIdRef.current)
        ?? loadedProviders[0]
        ?? null;
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
      clearThroughputSamples();
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
        clearThroughputSamples();
      }

      const nextHealth = await getHealthPanelData(nodeIdValue, loadedTunnels[0]?.id ?? null);
      setHealth(nextHealth);
      if (nextHealth.tunnel_id) {
        recordThroughputSample(nextHealth);
      } else {
        clearThroughputSamples();
      }
    } catch (error) {
      handleError(error);
    }
  }, [clearThroughputSamples, handleError, recordThroughputSample]);

  const refreshHealthSnapshot = useCallback(async (silent = false) => {
    if (!selectedNodeIdRef.current) {
      return;
    }

    if (!silent) {
      setBusyAction("refresh-health");
    }

    try {
      const snapshot = await getHealthPanelData(selectedNodeIdRef.current, selectedTunnelIdRef.current || null);
      setHealth(snapshot);
      if (snapshot.tunnel_id) {
        recordThroughputSample(snapshot);
      }
    } catch (error) {
      if (!silent) {
        handleError(error);
      }
    } finally {
      if (!silent) {
        setBusyAction(null);
      }
    }
  }, [handleError, recordThroughputSample]);

  const refreshHealth = useCallback(async () => {
    await refreshHealthSnapshot(false);
  }, [refreshHealthSnapshot]);

  const saveToken = useCallback(async (apiToken: string) => {
    setBusyAction("save-token");
    try {
      const activeProvider = providers.find((provider) => provider.id === providerId) ?? null;
      setProviderFeedback("Saving token reference locally...");
      const result = await registerProviderToken(
        providerId,
        providerName,
        apiToken,
        activeProvider?.slug ?? "digitalocean",
      );
      setProviderFeedback(result);
      setRecovery(null);
      pushToast("success", "Token saved", result);
      await loadDashboard();
    } catch (error) {
      setProviderFeedback(handleError(error));
    } finally {
      setBusyAction(null);
    }
  }, [loadDashboard, providerId, providerName, providers, pushToast, handleError]);

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
      const message = handleError(error);
      setProviderStatusMessage(message);
      setProviderFeedback(message);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, providerId, pushToast]);

  const saveDockerFauxToken = useCallback(async (value: string) => {
    if (!dockerDevEnabled) {
      return;
    }

    setBusyAction("save-docker-token");
    try {
      const trimmed = value.trim();
      if (!trimmed) {
        throw new Error("Docker faux token cannot be empty in development mode.");
      }

      if (!(trimmed.startsWith("dkr_v1_") || trimmed.startsWith("docker_"))) {
        throw new Error("Docker faux token must start with 'dkr_v1_' or 'docker_'.");
      }

      const activeProvider = providers.find((provider) => provider.id === providerId) ?? null;
      const result = await registerProviderToken(
        providerId,
        providerName,
        trimmed,
        activeProvider?.slug ?? "docker",
      );

      const configuredAt = new Date().toISOString();
      window.localStorage.setItem(DOCKER_FAUX_TOKEN_STORAGE_KEY, configuredAt);
      setDockerFauxTokenConfigured(true);
      setDockerFauxTokenConfiguredAt(configuredAt);
      setProviderFeedback(result);
      setDockerReadinessState("checking");
      setDockerReadinessMessage("Faux token stored for development emulation. Validate Docker readiness next.");
      pushToast("success", "Docker faux token saved", result);
      await loadDashboard();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [dockerDevEnabled, handleError, loadDashboard, providerId, providerName, providers, pushToast]);

  const validateDockerReadiness = useCallback(async (options?: { silent?: boolean }) => {
    const silent = options?.silent ?? false;

    if (!dockerDevEnabled) {
      return;
    }

    setBusyAction("validate-docker");
    setDockerReadinessState("checking");
    try {
      if (!dockerFauxTokenConfigured) {
        setDockerReadinessState("missing");
        setDockerReadinessMessage("Save the Docker faux token first to emulate provider access validation.");
        if (!silent) {
          pushToast("warning", "Docker faux token missing", "Save a development faux token before validating Docker readiness.");
        }
        return;
      }

      const readiness = await checkDockerReadiness();
      setDockerReadiness(readiness);

      if (readiness.installed && readiness.daemon_reachable) {
        setDockerReadinessState("ready");
      } else {
        setDockerReadinessState("invalid");
      }

      setDockerReadinessMessage(readiness.message);
      if (!silent) {
        pushToast("info", "Docker readiness checked", readiness.message);
      }
    } catch (error) {
      setDockerReadinessState("invalid");
      if (silent) {
        const message = error instanceof Error ? error.message : String(error);
        setDockerReadinessMessage(message);
      } else {
        setDockerReadinessMessage(handleError(error));
      }
    } finally {
      setBusyAction(null);
    }
  }, [dockerDevEnabled, dockerFauxTokenConfigured, handleError, pushToast]);

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

  const installHostForNode = useCallback(async (node: VpnNodeRecord) => {
    if (node.lifecycle_status !== "new") {
      pushToast("warning", "Host already provisioned", `Node ${node.name} is already in ${node.lifecycle_status} state.`);
      return;
    }

    const planKey = node.plan_key ?? node.size_slug;
    if (!planKey) {
      pushToast("error", "Missing plan", `Node ${node.name} is missing a plan key and cannot be installed.`);
      return;
    }

    setBusyAction("create-node");
    try {
      const result = await createVpnNode({
        nodeId: node.id,
        providerId: node.provider_id,
        nodeName: node.name,
        region: node.region,
        planKey,
      });

      setRecovery(null);
      pushToast("success", "Host installed", result);
      await loadDashboard(node.id);
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, pushToast]);

  const connectNode = useCallback(async (node: VpnNodeRecord) => {
    if (!node) {
      pushToast("warning", "No node selected", "Select a node before connecting.");
      return;
    }

    setBusyAction("connect-tunnel");
    try {
      let tunnelForNode = tunnels.find((tunnel) => tunnel.node_id === node.id) ?? null;

      if (!tunnelForNode) {
        setBusyAction("create-tunnel");
        const tunnelId = `tunnel-${crypto.randomUUID()}`;
        const sanitizedName = node.name
          .trim()
          .replace(/[^A-Za-z0-9_-]+/g, "-")
          .replace(/-{2,}/g, "-")
          .replace(/^-|-$/g, "")
          .slice(0, 28) || "primary";

        await createAndActivateClientTunnel({
          tunnelId,
          nodeId: node.id,
          tunnelName: `${sanitizedName}-wg`,
          clientAddressCidr: "10.66.0.2/32",
          dnsServer: "1.1.1.1",
          allowedIps: ["0.0.0.0/0", "::/0"],
        });

        const refreshedTunnels = await listTunnelsForNode(node.id);
        tunnelForNode = refreshedTunnels[0] ?? null;
      }

      if (!tunnelForNode) {
        throw new Error("Primary tunnel could not be resolved after creation.");
      }

      if (tunnelForNode.status.toLowerCase() !== "connected") {
        setBusyAction("connect-tunnel");
        await connectTunnelCommand(node.id, tunnelForNode.id);
      }

      setRecovery(null);
      pushToast("success", "Tunnel connected", `${tunnelForNode.name} is now connected.`);
      await loadDashboard(node.id);
      await refreshHealthSnapshot(true);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const staleOrMissingService = message.includes("was not found locally")
        || message.includes("does not exist on this machine")
        || message.includes("OpenService FAILED 1060")
        || message.includes("Could not complete WireGuard tunnel service operation")
        || message.includes("WireGuard command failed for 'sc start WireGuardTunnel$");

      if (staleOrMissingService) {
        const recoverableTunnel = tunnels.find((tunnel) => tunnel.node_id === node.id) ?? null;
        if (recoverableTunnel) {
          try {
            setBusyAction("create-tunnel");
            await createAndActivateClientTunnel({
              tunnelId: recoverableTunnel.id,
              nodeId: recoverableTunnel.node_id,
              tunnelName: recoverableTunnel.name,
              clientAddressCidr: "10.66.0.2/32",
              dnsServer: "1.1.1.1",
              allowedIps: ["0.0.0.0/0", "::/0"],
            });

            setRecovery(null);
            pushToast("success", "Tunnel recovered", `${recoverableTunnel.name} was re-imported and connected.`);
            await loadDashboard(node.id);
            await refreshHealthSnapshot(true);
            return;
          } catch (recoveryError) {
            handleError(recoveryError);
            return;
          }
        }
      }

      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, pushToast, refreshHealthSnapshot, tunnels]);

  const connectPrimary = useCallback(async () => {
    if (!activeNode) {
      pushToast("warning", "No node selected", "Select a node before connecting.");
      return;
    }

    void connectNode(activeNode);
  }, [activeNode, connectNode, pushToast]);

  const connectTunnel = useCallback(async (tunnel: TunnelRecord) => {
    setBusyAction("connect-tunnel");
    try {
      await connectTunnelCommand(tunnel.node_id, tunnel.id);
      setRecovery(null);
      pushToast("success", "Tunnel connected", `${tunnel.name} is now connected.`);
      await loadDashboard(tunnel.node_id);
      await refreshHealth();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const missingService = message.includes("was not found locally") || message.includes("does not exist on this machine");

      if (missingService) {
        try {
          setBusyAction("create-tunnel");
          await createAndActivateClientTunnel({
            tunnelId: tunnel.id,
            nodeId: tunnel.node_id,
            tunnelName: tunnel.name,
            clientAddressCidr: "10.66.0.2/32",
            dnsServer: "1.1.1.1",
            allowedIps: ["0.0.0.0/0", "::/0"],
          });

          setRecovery(null);
          pushToast("success", "Tunnel recovered", `${tunnel.name} was re-imported and connected.`);
          await loadDashboard(tunnel.node_id);
          await refreshHealth();
          return;
        } catch (recoveryError) {
          handleError(recoveryError);
          return;
        }
      }

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

  const uninstallHostForNode = useCallback(async (node: VpnNodeRecord) => {
    await destroyNode(node);
  }, [destroyNode]);

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
      void refreshHealthSnapshot(true);
    }, 0);

    return () => window.clearTimeout(timeoutId);
  }, [refreshHealthSnapshot, selectedNodeId, selectedTunnelId]);

  useEffect(() => {
    if (!selectedNodeId) {
      return;
    }

    const intervalId = window.setInterval(() => {
      void refreshHealthSnapshot(true);
    }, 5000);

    return () => window.clearInterval(intervalId);
  }, [refreshHealthSnapshot, selectedNodeId]);

  useEffect(() => {
    if (!dockerDevEnabled) {
      return;
    }

    const timeoutId = window.setTimeout(() => {
      void validateDockerReadiness({ silent: true });
    }, 0);

    return () => window.clearTimeout(timeoutId);
  }, [dockerDevEnabled, validateDockerReadiness]);

  const selectProviderId = useCallback((nextProviderId: string) => {
    const nextProvider = providers.find((provider) => provider.id === nextProviderId) ?? null;
    if (!nextProvider) {
      return;
    }

    setProviderId(nextProvider.id);
    setProviderName(nextProvider.name);
    void refreshProviderAccessStatus(nextProvider);
  }, [providers, refreshProviderAccessStatus]);

  const suspendProvider = useCallback(async (provider: ProviderRecord) => {
    setBusyAction("suspend-provider");
    try {
      await suspendProviderCommand(provider.id);
      setProviders((current) => current.map((item) => (
        item.id === provider.id
          ? { ...item, status: "suspended" }
          : item
      )));
      setRecovery(null);
      pushToast("warning", "Provider suspended", `${provider.name} is now suspended.`);
      await loadDashboard();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, pushToast]);

  const resumeProvider = useCallback(async (provider: ProviderRecord) => {
    setBusyAction("resume-provider");
    try {
      await resumeProviderCommand(provider.id);
      setRecovery(null);
      pushToast("success", "Provider resumed", `${provider.name} can be used again.`);
      await loadDashboard();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, pushToast]);

  const deleteProvider = useCallback(async (provider: ProviderRecord) => {
    setBusyAction("delete-provider");
    try {
      await deleteProviderCommand(provider.id);
      if (provider.slug === "docker" && typeof window !== "undefined") {
        window.localStorage.removeItem(DOCKER_FAUX_TOKEN_STORAGE_KEY);
        setDockerFauxTokenConfigured(false);
        setDockerFauxTokenConfiguredAt(null);
        setDockerReadiness(null);
        setDockerReadinessState("missing");
        setDockerReadinessMessage("Save the Docker faux token first to emulate provider access validation.");
      }

      setRecovery(null);
      pushToast("warning", "Provider key deleted", `${provider.name} credential was removed from this device.`);
      await loadDashboard();
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, pushToast]);

  const configuredProvider = providers.find((provider) => provider.id === providerId) ?? providers[0] ?? null;
  const providerHasStoredToken = !!configuredProvider?.token_ref && providerAccessState !== "invalid";

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
    selectProviderId,
    providerAccessState,
    providerStatusMessage,
    providerFeedback,
    dockerDevEnabled,
    dockerReadinessState,
    dockerReadinessMessage,
    dockerReadiness,
    dockerFauxTokenConfigured,
    dockerFauxTokenConfiguredAt,
    configuredProvider,
    providerHasStoredToken,
    selectedNode,
    activeNode,
    connectedTunnel,
    noServerConfigured,
    canConnect,
    throughputSamples,
    loadDashboard,
    refreshHealth,
    saveToken,
    validateToken,
    suspendProvider,
    resumeProvider,
    deleteProvider,
    saveDockerFauxToken,
    validateDockerReadiness,
    createNode,
    installHostForNode,
    uninstallHostForNode,
    connectNode,
    connectPrimary,
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