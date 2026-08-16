"use client";

import {
  checkDockerReadiness,
  checkWireGuardReadiness,
  connectTunnel as connectTunnelCommand,
  createAndActivateClientTunnel,
  deleteProvider as deleteProviderCommand,
  createVpnNode,
  destroyVpnNode,
  disconnectTunnel as disconnectTunnelCommand,
  forceDisconnectNodeService as forceDisconnectNodeServiceCommand,
  exportDiagnosticsBundle,
  forceCleanupVpnNode,
  getCurrentNodeId,
  setCurrentNodeId,
  getTunnelSettings,
  setTunnelSettings as setTunnelSettingsCommand,
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
  type TunnelMetricsSource,
  type TunnelRecord,
  type TunnelSettings,
  type VpnNodeRecord,
  type WireGuardReadiness,
  toCommandError,
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

/**
 * Whether a snapshot describes a live tunnel.
 *
 * Only a connected tunnel has throughput to plot. Sampling a disconnected one fed the chart a
 * run of zeros that looked like a connected tunnel sitting idle.
 */
function snapshotIsConnected(snapshot: HealthPanelData): boolean {
  return snapshot.tunnel_status?.toLowerCase() === "connected";
}

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
  configuredProvider: ProviderRecord | null;
  providerHasStoredToken: boolean;
  selectedNode: VpnNodeRecord | null;
  activeNode: VpnNodeRecord | null;
  connectedTunnel: TunnelRecord | null;
  noServerConfigured: boolean;
  /** Live service state, not the stored row. The one value every connection widget keys off. */
  tunnelConnected: boolean;
  /** The node the app connects to by default, or null when several exist and none was chosen. */
  currentNodeId: string | null;
  setCurrentNode: (nodeId: string) => Promise<void>;
  /** Clears the recovery banner. It otherwise persists until the next successful operation. */
  dismissRecovery: () => void;
  /** Local WireGuard install probe, refreshed with the dashboard rather than on its own timer. */
  wireguardReadiness: WireGuardReadiness | null;
  refreshWireGuardReadiness: () => Promise<void>;
  /** When the last health snapshot landed, so a page can show a real "last checked". */
  healthUpdatedAt: number | null;
  /** Re-fetches the diagnostics bundle without the busy state a full dashboard reload sets. */
  refreshDiagnostics: () => Promise<void>;
  /** Client tunnel parameters. Applied when a tunnel next starts. */
  tunnelSettings: TunnelSettings | null;
  /** Resolves true when the settings were accepted and stored. */
  saveTunnelSettings: (settings: TunnelSettings) => Promise<boolean>;
  throughputSamples: ThroughputSample[];
  loadDashboard: (preferredSelectedNodeId?: string) => Promise<void>;
  refreshHealth: () => Promise<void>;
  /** Saves and validates in one step. Resolves true only when the key was stored. */
  saveToken: (apiToken: string, targetProviderId?: string) => Promise<boolean>;
  validateToken: () => Promise<void>;
  suspendProvider: (provider: ProviderRecord) => Promise<void>;
  resumeProvider: (provider: ProviderRecord) => Promise<void>;
  deleteProvider: (provider: ProviderRecord) => Promise<void>;
  validateDockerReadiness: (options?: { silent?: boolean }) => Promise<void>;
  createNode: (input: CreateNodeInput) => Promise<void>;
  installHostForNode: (node: VpnNodeRecord) => Promise<void>;
  connectPrimary: () => Promise<void>;
  connectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  disconnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  forceDisconnectNodeService: (node: VpnNodeRecord) => Promise<void>;
  destroyNode: (node: VpnNodeRecord) => Promise<void>;
  connectNode: (node: VpnNodeRecord) => Promise<void>;
  forceCleanupSelectedNode: () => Promise<void>;
  copyDiagnostics: () => Promise<void>;
  exportDiagnostics: () => Promise<void>;
};

const DashboardContext = createContext<DashboardContextValue | null>(null);

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
  // Named to keep the React setter distinct from the `setCurrentNodeId` Tauri command.
  const [currentNodeId, setCurrentNodeIdState] = useState<string | null>(null);
  const [wireguardReadiness, setWireguardReadiness] = useState<WireGuardReadiness | null>(null);
  const [healthUpdatedAt, setHealthUpdatedAt] = useState<number | null>(null);
  const [tunnelSettings, setTunnelSettings] = useState<TunnelSettings | null>(null);
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
  const providerIdRef = useRef(providerId);
  const selectedNodeIdRef = useRef(selectedNodeId);
  const selectedTunnelIdRef = useRef(selectedTunnelId);
  const lastTrafficSampleRef = useRef<{
    timestamp: number;
    nodeId: string;
    tunnelId: string;
    rxBytes: number;
    txBytes: number;
    metricsSource: TunnelMetricsSource | null;
  } | null>(null);

  // Whether a Docker development key exists. Derived from the stored credential rather than a
  // localStorage marker: the marker was a second, drift-prone record of the same fact, and it went
  // stale the moment a key was removed from anywhere but its own save path.
  const dockerKeyStored = providers.some((provider) => provider.slug === "docker" && !!provider.token_ref);

  const selectedNode = nodes.find((node) => node.id === selectedNodeId) ?? null;
  // Precedence: the node carrying a tunnel, then the user's persisted choice, then a lone node,
  // and only then whatever row is selected in the UI. The persisted choice is the addition — it
  // used to jump straight from "any lifecycle-connected node" to the current selection, so a
  // deliberate default could not outrank an incidental one.
  const activeNode = nodes.find((node) => node.lifecycle_status.toLowerCase() === "connected")
    ?? nodes.find((node) => node.id === currentNodeId)
    ?? (nodes.length === 1 ? nodes[0] : null)
    ?? selectedNode;
  const connectedTunnel = tunnels.find((tunnel) => tunnel.status.toLowerCase() === "connected") ?? tunnels[0] ?? null;
  const noServerConfigured = nodes.length === 0;

  // The live Windows service decides whether a tunnel is up; the stored row is only a fallback
  // for the moment before the first health snapshot arrives.
  //
  // These disagreed. The row went stale whenever a tunnel changed state outside a completed
  // connect flow, and reading it made the dashboard show "Idle" with a Connect button while the
  // handshake card on the same screen — fed by the runtime — correctly read "connected".
  const runtimeStatus = health?.tunnel_status?.toLowerCase() ?? null;
  const tunnelConnected = runtimeStatus !== null
    ? runtimeStatus === "connected"
    : connectedTunnel?.status.toLowerCase() === "connected";

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
    // Every backend failure arrives structured, so message text is only ever displayed —
    // never parsed. The " Next: " suffix this used to split on no longer exists.
    const commandError = toCommandError(error);
    const classification = categorizeError(commandError.kind);

    setRecovery({
      ...classification,
      message: commandError.message,
      nextAction: commandError.nextAction,
    });
    pushToast(
      classification.kind === "state" ? "warning" : "error",
      classification.title,
      `${commandError.message} ${commandError.nextAction}`,
    );

    return `${commandError.message} ${commandError.nextAction}`;
  }, [pushToast]);

  const clearThroughputSamples = useCallback(() => {
    lastTrafficSampleRef.current = null;
    setThroughputSamples([]);
  }, []);

  const recordThroughputSample = useCallback((snapshot: HealthPanelData) => {
    // Counters absent means they could not be read, which is not the same claim as "no traffic".
    // Coercing to zero drew a flat line that was indistinguishable from a genuinely idle tunnel.
    if (snapshot.rx_bytes === null || snapshot.tx_bytes === null) {
      lastTrafficSampleRef.current = null;
      return;
    }

    const now = Date.now();
    const nodeId = snapshot.node_id;
    const tunnelId = snapshot.tunnel_id ?? "";
    const rxBytes = snapshot.rx_bytes;
    const txBytes = snapshot.tx_bytes;
    const metricsSource = snapshot.metrics_source;
    const previous = lastTrafficSampleRef.current;

    let downloadBytesPerSecond = 0;
    let uploadBytesPerSecond = 0;

    // A delta is only meaningful between two readings of the same counter. `wg show` and the
    // adapter count different things, so a tier switch (gaining or losing elevation) changes the
    // basis mid-series — differencing across it invents a spike the size of the whole offset.
    if (
      previous
      && previous.nodeId === nodeId
      && previous.tunnelId === tunnelId
      && previous.metricsSource === metricsSource
      && now > previous.timestamp
    ) {
      const elapsedSeconds = (now - previous.timestamp) / 1000;
      if (elapsedSeconds > 0) {
        downloadBytesPerSecond = Math.max(0, (rxBytes - previous.rxBytes) / elapsedSeconds);
        uploadBytesPerSecond = Math.max(0, (txBytes - previous.txBytes) / elapsedSeconds);
      }
    }

    const isBaseline = previous === null
      || previous.nodeId !== nodeId
      || previous.tunnelId !== tunnelId
      || previous.metricsSource !== metricsSource;

    lastTrafficSampleRef.current = {
      timestamp: now,
      nodeId,
      tunnelId,
      rxBytes,
      txBytes,
      metricsSource,
    };

    // The first reading establishes the baseline; there is no earlier counter to subtract, so it
    // carries no rate. Plotting its zero would render as a drop to idle.
    if (isBaseline) {
      return;
    }

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

  const dismissRecovery = useCallback(() => {
    setRecovery(null);
  }, []);

  const saveTunnelSettings = useCallback(async (settings: TunnelSettings) => {
    setBusyAction("save-tunnel-settings");
    try {
      await setTunnelSettingsCommand(settings);
      setTunnelSettings(settings);
      setRecovery(null);
      pushToast(
        "success",
        "WireGuard settings saved",
        "Applied the next time a tunnel starts. Reconnect for them to take effect now.",
      );
      return true;
    } catch (error) {
      handleError(error);
      return false;
    } finally {
      setBusyAction(null);
    }
  }, [handleError, pushToast]);

  const refreshWireGuardReadiness = useCallback(async () => {
    try {
      setWireguardReadiness(await checkWireGuardReadiness());
    } catch {
      // A probe that cannot run is reported as "not checked" rather than as an install failure.
      setWireguardReadiness(null);
    }
  }, []);

  /// Silent on purpose: the log panel refreshes on a timer, and setting `busyAction` would disable
  /// every button on the page for the duration of each poll.
  const refreshDiagnostics = useCallback(async () => {
    try {
      setDiagnostics(await getDiagnosticsBundle());
    } catch {
      // Leave the previous bundle in place; a failed refresh is not worth clearing real logs for.
    }
  }, []);

  const setCurrentNode = useCallback(async (nodeId: string) => {
    try {
      await setCurrentNodeId(nodeId);
      setCurrentNodeIdState(nodeId);
      pushToast("success", "Current server updated", "New connections will use this server by default.");
    } catch (error) {
      handleError(error);
    }
  }, [handleError, pushToast]);

  const loadDashboard = useCallback(async (preferredSelectedNodeId?: string) => {
    setBusyAction("refresh");
    try {
      const [loadedNodes, loadedDiagnostics, loadedProviders, loadedCurrentNodeId, loadedTunnelSettings] = await Promise.all([
        listVpnNodes(),
        getDiagnosticsBundle(),
        listProviders(),
        getCurrentNodeId(),
        getTunnelSettings(),
      ]);

      setNodes(loadedNodes);
      setDiagnostics(loadedDiagnostics);
      setProviders(loadedProviders);
      setCurrentNodeIdState(loadedCurrentNodeId);
      setTunnelSettings(loadedTunnelSettings);

      const configuredProvider = loadedProviders.find((provider) => provider.id === providerIdRef.current)
        ?? loadedProviders[0]
        ?? null;
      if (configuredProvider) {
        setProviderId(configuredProvider.id);
        setProviderName(configuredProvider.name);
      }

      await refreshProviderAccessStatus(configuredProvider);
      // Rides along with the dashboard refresh rather than running its own timer; the install
      // state only changes when the user installs or repairs WireGuard.
      void refreshWireGuardReadiness();
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
  }, [handleError, refreshProviderAccessStatus, refreshWireGuardReadiness]);

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
      setHealthUpdatedAt(Date.now());
      if (nextHealth.tunnel_id && snapshotIsConnected(nextHealth)) {
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
      // A user-initiated refresh bypasses the backend's egress-IP cache; the ambient poll does
      // not, so polling never hammers the third-party IP service.
      const snapshot = await getHealthPanelData(
        selectedNodeIdRef.current,
        selectedTunnelIdRef.current || null,
        !silent,
      );
      setHealth(snapshot);
      setHealthUpdatedAt(Date.now());
      if (snapshot.tunnel_id && snapshotIsConnected(snapshot)) {
        recordThroughputSample(snapshot);
      } else {
        clearThroughputSamples();
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
  }, [clearThroughputSamples, handleError, recordThroughputSample]);

  const refreshHealth = useCallback(async () => {
    await refreshHealthSnapshot(false);
  }, [refreshHealthSnapshot]);

  /**
   * Stores a key and immediately checks it against the provider.
   *
   * One action, because the two used to be separate buttons and the validating one took no token —
   * it read whatever was already stored. Pasting a new key and clicking Validate therefore checked
   * the *old* key and could report success for a key that had never been saved.
   *
   * Returns whether the key was stored, so the caller knows whether it is safe to clear the field.
   */
  const saveToken = useCallback(async (apiToken: string, targetProviderId?: string) => {
    const provider = targetProviderId ?? providerId;
    const name = providers.find((entry) => entry.id === provider)?.name ?? providerName;

    setBusyAction("save-token");
    try {
      await registerProviderToken(provider, name, apiToken);
      setRecovery(null);
    } catch (error) {
      setProviderFeedback(handleError(error));
      setBusyAction(null);
      return false;
    }

    // Saved. A validation failure from here is a bad key, not a lost one, so the stored value
    // stays and the field can be cleared.
    try {
      setBusyAction("validate-token");
      const result = await validateProviderToken(provider);
      setProviderAccessState("ready");
      setProviderStatusMessage(result);
      setProviderFeedback(result);
      pushToast("success", "Key saved and verified", result);
    } catch (error) {
      setProviderAccessState("invalid");
      const message = handleError(error);
      setProviderStatusMessage(message);
      setProviderFeedback(message);
    } finally {
      setBusyAction(null);
    }

    await loadDashboard();
    return true;
  }, [handleError, loadDashboard, providerId, providerName, providers, pushToast]);

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

  const validateDockerReadiness = useCallback(async (options?: { silent?: boolean }) => {
    const silent = options?.silent ?? false;

    if (!dockerDevEnabled) {
      return;
    }

    setBusyAction("validate-docker");
    setDockerReadinessState("checking");
    try {
      if (!dockerKeyStored) {
        setDockerReadinessState("missing");
        setDockerReadinessMessage("Save a Docker development key in Settings before validating readiness.");
        if (!silent) {
          pushToast("warning", "Docker key missing", "Add a Docker development key in Settings first.");
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
  }, [dockerDevEnabled, dockerKeyStored, handleError, pushToast]);

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

    const canonicalTunnelName = `${node.name
      .trim()
      .replace(/[^A-Za-z0-9_-]+/g, "-")
      .replace(/-{2,}/g, "-")
      .replace(/^-|-$/g, "")
      .slice(0, 56) || "primary"}-wg`;

    setBusyAction("connect-tunnel");
    try {
      let tunnelForNode = tunnels.find((tunnel) => tunnel.node_id === node.id) ?? null;
      const canonicalTunnelId = `tunnel-${node.id}`;

      if (!tunnelForNode) {
        setBusyAction("create-tunnel");
        await createAndActivateClientTunnel({
          tunnelId: canonicalTunnelId,
          nodeId: node.id,
          tunnelName: canonicalTunnelName,
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
      // A stale or missing tunnel service is recoverable by re-importing the tunnel. The backend
      // reports it as a WireGuard failure; this used to be inferred by matching five different
      // message fragments, including raw Win32 text like "OpenService FAILED 1060".
      const commandError = toCommandError(error);
      const staleOrMissingService = commandError.kind === "wire-guard";

      if (staleOrMissingService) {
        const recoverableTunnel = tunnels.find((tunnel) => tunnel.node_id === node.id) ?? null;
        if (recoverableTunnel) {
          try {
            setBusyAction("create-tunnel");
            await createAndActivateClientTunnel({
              tunnelId: recoverableTunnel.id,
              nodeId: recoverableTunnel.node_id,
              tunnelName: canonicalTunnelName,
              clientAddressCidr: "10.66.0.2/32",
              dnsServer: "1.1.1.1",
              allowedIps: ["0.0.0.0/0", "::/0"],
            });

            setBusyAction("connect-tunnel");
            await connectTunnelCommand(node.id, recoverableTunnel.id);

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
      const missingService = toCommandError(error).kind === "wire-guard";

      if (missingService) {
        try {
          setBusyAction("create-tunnel");
          await createAndActivateClientTunnel({
            tunnelId: tunnel.id,
            nodeId: tunnel.node_id,
            tunnelName: `${(nodes.find((node) => node.id === tunnel.node_id)?.name ?? tunnel.name)
              .trim()
              .replace(/[^A-Za-z0-9_-]+/g, "-")
              .replace(/-{2,}/g, "-")
              .replace(/^-|-$/g, "")
              .slice(0, 56) || "primary"}-wg`,
            clientAddressCidr: "10.66.0.2/32",
            dnsServer: "1.1.1.1",
            allowedIps: ["0.0.0.0/0", "::/0"],
          });

          setBusyAction("connect-tunnel");
          await connectTunnelCommand(tunnel.node_id, tunnel.id);

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
  }, [handleError, loadDashboard, nodes, pushToast, refreshHealth]);

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

  const forceDisconnectNodeService = useCallback(async (node: VpnNodeRecord) => {
    setBusyAction("disconnect-tunnel");
    try {
      await forceDisconnectNodeServiceCommand(node.id);
      setRecovery(null);
      pushToast("success", "Tunnel disconnect signal sent", `Best-effort service stop sent for ${node.name}.`);
      await loadDashboard(node.id);
      await refreshHealthSnapshot(true);
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, pushToast, refreshHealthSnapshot]);

  const destroyNode = useCallback(async (node: VpnNodeRecord) => {
    // Names the location as well as the node: with the fleet keyed by city, "which one is this?"
    // is the question a user needs answered before deleting a server they pay for.
    const where = node.region_geo?.city ?? node.region;
    const confirmed = window.confirm(
      `Destroy ${node.name} in ${where}?\n\nThis deletes the server at your provider and stops its billing. It cannot be undone.`,
    );
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
      if (provider.slug === "docker") {
        // `dockerKeyStored` follows the reloaded provider list, so only the readiness result needs
        // clearing here.
        setDockerReadiness(null);
        setDockerReadinessState("missing");
        setDockerReadinessMessage("Save a Docker development key in Settings before validating readiness.");
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
    configuredProvider,
    providerHasStoredToken,
    selectedNode,
    activeNode,
    connectedTunnel,
    noServerConfigured,
    tunnelConnected,
    currentNodeId,
    setCurrentNode,
    dismissRecovery,
    wireguardReadiness,
    refreshWireGuardReadiness,
    healthUpdatedAt,
    refreshDiagnostics,
    tunnelSettings,
    saveTunnelSettings,
    throughputSamples,
    loadDashboard,
    refreshHealth,
    saveToken,
    validateToken,
    suspendProvider,
    resumeProvider,
    deleteProvider,
    validateDockerReadiness,
    createNode,
    installHostForNode,
    connectNode,
    connectPrimary,
    connectTunnel,
    disconnectTunnel,
    forceDisconnectNodeService,
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