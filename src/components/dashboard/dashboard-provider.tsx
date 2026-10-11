"use client";

import {
  checkDockerReadiness,
  checkWireGuardReadiness,
  connectTunnel as connectTunnelCommand,
  createAndActivateClientTunnel,
  deleteProvider as deleteProviderCommand,
  createVpnNode,
  destroyVpnNode,
  forgetSavedLocation,
  listProviderPricing,
  listSavedLocations,
  turnOffSavedLocation,
  turnOnSavedLocation,
  type ProviderPlanPricing,
  type SavedLocationRecord,
  disconnectTunnel as disconnectTunnelCommand,
  forceDisconnectNodeService as forceDisconnectNodeServiceCommand,
  exportDiagnosticsBundle,
  forceCleanupVpnNode,
  getCurrentNodeId,
  setCurrentNodeId,
  getTunnelSettings,
  setTunnelSettings as setTunnelSettingsCommand,
  getAppPreferences,
  setAppPreferences as setAppPreferencesCommand,
  getDiagnosticsBundle,
  getHealthPanelData,
  getTunnelPhase,
  listAppLocations,
  listProviders,
  listNodePeers,
  listTunnelsForNode,
  claimNodePeer,
  getPeerConfig,
  exportPeerConfig,
  type NodePeerRecord,
  type PeerConfigPayload,
  listVpnNodes,
  registerProviderCredential,
  resumeProvider as resumeProviderCommand,
  revealAppLocation,
  suspendProvider as suspendProviderCommand,
  validateProviderToken,
  type AppLocation,
  type AppPreferences,
  type NodeLifecycleStatus,
  type DockerReadiness,
  type ProviderRecord,
  type TunnelMetricsSource,
  type TunnelPhase,
  type TunnelPhaseSnapshot,
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
import { describeTurnOn } from "@/lib/saved-location-display";
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
  /** What the build screen calls it, "BUILDING FRANKFURT". Defaults to the node name. */
  place?: string;
};

/** The "one Windows prompt is coming" explainer, waiting on the user's answer. */
export type UacPrompt = { place: string };

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

/**
 * A connect the user started that has not finished.
 *
 * Separate from `busyAction` because that slot is global and exclusive — every control in the app
 * is disabled while it is set. A connect takes tens of seconds and leaves the user without
 * internet, which is the worst possible moment to also freeze the window they would use to
 * understand or undo it.
 */
export type ConnectAttempt = {
  nodeId: string;
  tunnelId: string | null;
  tunnelName: string;
  /** `Date.now()` at the click, so the clock runs before the first poll lands. */
  startedAt: number;
  /** True while the tunnel record is being created, before there is a service to poll. */
  creatingTunnel: boolean;
  /** Latest backend verdict; null until the first phase poll returns. */
  snapshot: TunnelPhaseSnapshot | null;
};

/**
 * A provision, destroy or force cleanup in flight, for the progress card.
 *
 * These already hold `busyAction`, which is exclusive, so at most one exists at a time. Tracked
 * here rather than in the page that started it because the card outlives navigation — a finished
 * provision sends the user back to the server list.
 */
export type OperationAttempt =
  | {
      kind: "provision";
      nodeId: string;
      nodeName: string;
      /** The city, or the node name when the region has no geography. */
      place: string;
      providerName: string;
      /** `Date.now()` at the click. */
      startedAt: number;
      /** Latest polled lifecycle status; null until the first poll lands. */
      status: NodeLifecycleStatus | null;
      /** `Date.now()` when `bootstrapping` was first seen, so that stage's clock starts there. */
      bootstrappingSince: number | null;
      /**
       * The user asked to cancel. A create is one blocking provider call with nothing to cancel
       * part-way, so the server is deleted as soon as it exists instead of being connected to.
       */
      cancelRequested: boolean;
    }
  | {
      kind: "destroy" | "force-cleanup";
      nodeId: string;
      nodeName: string;
      providerName: string;
      startedAt: number;
    };

type DashboardContextValue = {
  busyAction: DashboardBusyAction;
  /** The in-flight provision, destroy or cleanup, or null. Drives the operation progress card. */
  operationAttempt: OperationAttempt | null;
  /** The in-flight connect, or null. Drives the progress card and gates connect controls. */
  connectAttempt: ConnectAttempt | null;
  /** Tears down a connect that is taking too long, restoring the user's normal connection. */
  cancelConnect: () => Promise<void>;
  toasts: Toast[];
  recovery: RecoveryState | null;
  nodes: VpnNodeRecord[];
  /** Servers the user can turn back on, with whether each is on now. */
  savedLocations: SavedLocationRecord[];
  /** Quotes today's price, asks, then provisions the location. */
  turnOnLocation: (location: SavedLocationRecord) => Promise<void>;
  /** Deletes the location's server to stop billing; the location stays. */
  turnOffLocation: (location: SavedLocationRecord) => Promise<void>;
  forgetLocation: (location: SavedLocationRecord) => Promise<void>;
  providers: ProviderRecord[];
  tunnels: TunnelRecord[];
  /** Baked device slots for the selected node. Empty for a node provisioned before them. */
  nodePeers: NodePeerRecord[];
  claimDevice: (nodeId: string, peerId: string, label: string) => Promise<boolean>;
  loadDeviceConfig: (nodeId: string, peerId: string) => Promise<PeerConfigPayload | null>;
  exportDeviceConfig: (nodeId: string, peerId: string) => Promise<void>;
  selectedNodeId: string;
  setSelectedNodeId: (value: string) => void;
  selectedTunnelId: string;
  setSelectedTunnelId: (value: string) => void;
  health: HealthPanelData | null;
  diagnostics: DiagnosticsBundle | null;
  diagnosticsExportPath: string | null;
  appLocations: AppLocation[];
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
  /**
   * Whether the tunnel provably carries traffic. The one value every connection widget keys off.
   *
   * Not "is the service running" — that is true roughly fifteen seconds before the tunnel works,
   * and turning the indicator green there is the bug this replaced.
   */
  tunnelConnected: boolean;
  /** The backend's phase verdict, for widgets that need to distinguish connecting from idle. */
  tunnelPhase: TunnelPhase;
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
  /** Desktop-shell preferences; null until first loaded. */
  appPreferences: AppPreferences | null;
  /** Resolves true when the preferences were stored. */
  saveAppPreferences: (preferences: AppPreferences) => Promise<boolean>;
  /**
   * Merges a change into the stored preferences without the global busy state — for small pieces
   * of interface state (favourites, the game's best score) that must not freeze the window.
   */
  updateAppPreferences: (patch: Partial<AppPreferences>) => Promise<void>;
  /** Marks the running build to be deleted as soon as the provider finishes creating it. */
  requestCancelBuild: () => void;
  /** The pre-connect explainer, while it waits for an answer. */
  uacPrompt: UacPrompt | null;
  /** Answers the explainer. `dontExplainAgain` turns it off for later connects. */
  answerUacPrompt: (proceed: boolean, dontExplainAgain: boolean) => void;
  throughputSamples: ThroughputSample[];
  loadDashboard: (preferredSelectedNodeId?: string) => Promise<void>;
  refreshHealth: () => Promise<void>;
  /** Saves and validates in one step. Resolves true only when the key was stored. */
  saveToken: (credentials: Record<string, string>, targetProviderId?: string) => Promise<boolean>;
  /** Checks one provider's key and records the result against that provider alone. */
  validateProviderKey: (providerId: string) => Promise<string | null>;
  /** Which provider is being checked right now, so only that row shows a busy state. */
  validatingProviderId: string | null;
  suspendProvider: (provider: ProviderRecord) => Promise<void>;
  resumeProvider: (provider: ProviderRecord) => Promise<void>;
  deleteProvider: (provider: ProviderRecord) => Promise<void>;
  validateDockerReadiness: (options?: { silent?: boolean }) => Promise<void>;
  /** Resolves true only when the server reached ready. */
  createNode: (input: CreateNodeInput) => Promise<boolean>;
  installHostForNode: (node: VpnNodeRecord) => Promise<void>;
  connectPrimary: () => Promise<void>;
  connectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  disconnectTunnel: (tunnel: TunnelRecord) => Promise<void>;
  forceDisconnectNodeService: (node: VpnNodeRecord) => Promise<void>;
  destroyNode: (node: VpnNodeRecord) => Promise<void>;
  connectNode: (node: VpnNodeRecord) => Promise<void>;
  forceCleanupSelectedNode: () => Promise<void>;
  forceCleanupNode: (nodeId: string) => Promise<void>;
  copyDiagnostics: () => Promise<void>;
  exportDiagnostics: () => Promise<void>;
  copyText: (value: string, label: string) => Promise<void>;
  revealLocation: (key: string) => Promise<void>;
};

const DashboardContext = createContext<DashboardContextValue | null>(null);

export function DashboardProvider({ children }: { children: ReactNode }) {
  const dockerDevEnabled = process.env.NEXT_PUBLIC_DEV_DOCKER === "1";
  const [busyAction, setBusyAction] = useState<DashboardBusyAction>(null);
  const [connectAttempt, setConnectAttempt] = useState<ConnectAttempt | null>(null);
  const [operationAttempt, setOperationAttempt] = useState<OperationAttempt | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [recovery, setRecovery] = useState<RecoveryState | null>(null);
  const [nodes, setNodes] = useState<VpnNodeRecord[]>([]);
  const [savedLocations, setSavedLocations] = useState<SavedLocationRecord[]>([]);
  const [providers, setProviders] = useState<ProviderRecord[]>([]);
  const [tunnels, setTunnels] = useState<TunnelRecord[]>([]);
  const [nodePeers, setNodePeers] = useState<NodePeerRecord[]>([]);
  const [selectedNodeId, setSelectedNodeId] = useState("");
  const [selectedTunnelId, setSelectedTunnelId] = useState("");
  const [health, setHealth] = useState<HealthPanelData | null>(null);
  // Named to keep the React setter distinct from the `setCurrentNodeId` Tauri command.
  const [currentNodeId, setCurrentNodeIdState] = useState<string | null>(null);
  const [wireguardReadiness, setWireguardReadiness] = useState<WireGuardReadiness | null>(null);
  const [healthUpdatedAt, setHealthUpdatedAt] = useState<number | null>(null);
  const [tunnelSettings, setTunnelSettings] = useState<TunnelSettings | null>(null);
  const [appPreferences, setAppPreferencesState] = useState<AppPreferences | null>(null);
  const [uacPrompt, setUacPrompt] = useState<UacPrompt | null>(null);
  const [throughputSamples, setThroughputSamples] = useState<ThroughputSample[]>([]);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsBundle | null>(null);
  // Null rather than a "No export yet." sentinel: the UI hides the row instead of printing prose
  // where a path belongs.
  const [diagnosticsExportPath, setDiagnosticsExportPath] = useState<string | null>(null);
  const [appLocations, setAppLocations] = useState<AppLocation[]>([]);
  const [providerId, setProviderId] = useState(DEFAULT_PROVIDER_ID);
  const [providerName, setProviderName] = useState(DEFAULT_PROVIDER_NAME);
  const [providerAccessState, setProviderAccessState] = useState<ProviderAccessState>("unknown");
  const [providerStatusMessage, setProviderStatusMessage] = useState("Provider status has not been checked yet.");
  const [providerFeedback, setProviderFeedback] = useState("No provider action yet.");
  const [validatingProviderId, setValidatingProviderId] = useState<string | null>(null);
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
  // Guards against a slow health read queueing more of itself behind the tick.
  const healthPollInFlightRef = useRef(false);
  // The latest stored preferences, for callbacks that must not re-create on every change.
  const appPreferencesRef = useRef<AppPreferences | null>(null);
  // Preference writes run one after another, each merging into the previous result, so two quick
  // changes (a favourite and a game score) cannot overwrite each other.
  const preferencesWriteRef = useRef<Promise<void>>(Promise.resolve());
  // Resolves the pre-connect explainer's promise when the user answers it.
  const uacResolverRef = useRef<((proceed: boolean) => void) | null>(null);
  // The node id of a build the user cancelled, so it is deleted instead of connected when it lands.
  const cancelBuildRef = useRef<string | null>(null);
  // Declared further down; the build flow reaches them through these so it can be declared first.
  const connectNodeRef = useRef<(node: VpnNodeRecord) => Promise<void>>(async () => {});
  const destroyNodeNowRef = useRef<(node: VpnNodeRecord, reason?: "cancelled") => Promise<void>>(async () => {});
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

  // Whether the tunnel provably carries traffic, decided in Rust by `classify_tunnel_phase`.
  //
  // This used to read `health.tunnel_status`, which reports the Windows *service*. The service
  // reaches "connected" the instant it starts — and because the adapter captures the default route
  // at that same instant, that is exactly when the user's internet goes down. In the reported
  // failure the handshake landed fifteen seconds later, so the indicator was green for the entire
  // outage. The phase verdict waits for evidence that traffic actually crossed the tunnel.
  //
  // While an attempt is live, only its own snapshot may turn anything green: the 5s health poll is
  // suspended during a connect, so its last value is stale by definition.
  const tunnelPhase: TunnelPhase = connectAttempt !== null
    ? connectAttempt.snapshot?.phase ?? "connecting"
    : health?.phase
      // Before the first snapshot of the session there is nothing measured yet, so the stored row
      // stands in. It is the only fallback left, and only for that moment.
      ?? (connectedTunnel?.status.toLowerCase() === "connected" ? "connected" : "disconnected");

  const tunnelConnected = tunnelPhase === "connected" || tunnelPhase === "degraded";

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

  /** Display name of a provider by id, for progress copy. Falls back to the selected provider. */
  const providerNameFor = useCallback((id: string | null) => {
    return providers.find((provider) => provider.id === id)?.name ?? providerName;
  }, [providerName, providers]);

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
      setProviderStatusMessage(`No ${providerLabel} token is stored yet. Save and validate a token in Setup to provision nodes.`);
      return;
    }

    if (provider.status === "suspended") {
      setProviderAccessState("invalid");
      setProviderStatusMessage(`Provider ${providerLabel} is suspended. Resume it in Setup before running this operation.`);
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

  useEffect(() => {
    appPreferencesRef.current = appPreferences;
  }, [appPreferences]);

  const saveAppPreferences = useCallback(async (preferences: AppPreferences) => {
    setBusyAction("save-app-preferences");
    try {
      const stored = await setAppPreferencesCommand(preferences);
      appPreferencesRef.current = stored;
      setAppPreferencesState(stored);
      return true;
    } catch (error) {
      handleError(error);
      return false;
    } finally {
      setBusyAction(null);
    }
  }, [handleError]);

  const updateAppPreferences = useCallback((patch: Partial<AppPreferences>) => {
    const write = preferencesWriteRef.current.then(async () => {
      try {
        const base = appPreferencesRef.current ?? await getAppPreferences();
        const next = { ...base, ...patch };
        // Shown at once; the stored result replaces it when the write lands.
        appPreferencesRef.current = next;
        setAppPreferencesState(next);
        const stored = await setAppPreferencesCommand(next);
        appPreferencesRef.current = stored;
        setAppPreferencesState(stored);
      } catch (error) {
        handleError(error);
      }
    });
    preferencesWriteRef.current = write;
    return write;
  }, [handleError]);

  /**
   * Explains, once, that connecting raises one Windows prompt and pauses the internet briefly.
   *
   * Resolves true to go ahead. Shown before the first ever connect and, after that, only if the
   * user un-ticked "don't explain this again".
   */
  const ensureUacExplained = useCallback((place: string) => {
    if (appPreferencesRef.current && !appPreferencesRef.current.explain_uac) {
      return Promise.resolve(true);
    }
    return new Promise<boolean>((resolve) => {
      // A second connect while the first explainer is open supersedes it.
      uacResolverRef.current?.(false);
      uacResolverRef.current = resolve;
      setUacPrompt({ place });
    });
  }, []);

  const answerUacPrompt = useCallback((proceed: boolean, dontExplainAgain: boolean) => {
    const resolve = uacResolverRef.current;
    uacResolverRef.current = null;
    setUacPrompt(null);
    if (proceed && dontExplainAgain) void updateAppPreferences({ explain_uac: false });
    resolve?.(proceed);
  }, [updateAppPreferences]);

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

  const loadNodeDetails = useCallback(async (nodeIdValue: string) => {
    if (!nodeIdValue) {
      setTunnels([]);
      setSelectedTunnelId("");
      setHealth(null);
      setNodePeers([]);
      clearThroughputSamples();
      return;
    }

    try {
      const loadedTunnels = await listTunnelsForNode(nodeIdValue);
      setTunnels(loadedTunnels);

      // Device slots are informational, so a node that predates them must not break the panel
      // the tunnel and health data also live in.
      try {
        setNodePeers(await listNodePeers(nodeIdValue));
      } catch {
        setNodePeers([]);
      }

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

  const loadDashboard = useCallback(async (preferredSelectedNodeId?: string) => {
    setBusyAction("refresh");
    try {
      const [
        loadedNodes,
        loadedDiagnostics,
        loadedProviders,
        loadedCurrentNodeId,
        loadedTunnelSettings,
        loadedAppPreferences,
        loadedLocations,
        loadedSavedLocations,
      ] = await Promise.all([
        listVpnNodes(),
        getDiagnosticsBundle(),
        listProviders(),
        getCurrentNodeId(),
        getTunnelSettings(),
        getAppPreferences(),
        listAppLocations(),
        listSavedLocations(),
      ]);

      setNodes(loadedNodes);
      setSavedLocations(loadedSavedLocations);
      setDiagnostics(loadedDiagnostics);
      setProviders(loadedProviders);
      setCurrentNodeIdState(loadedCurrentNodeId);
      setTunnelSettings(loadedTunnelSettings);
      setAppPreferencesState(loadedAppPreferences);
      setAppLocations(loadedLocations);

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

      const selectionUnchanged = nextSelectedNodeId === selectedNodeIdRef.current;
      setSelectedNodeId(nextSelectedNodeId);

      if (!nextSelectedNodeId) {
        setTunnels([]);
        setSelectedTunnelId("");
        setHealth(null);
      } else if (selectionUnchanged) {
        // The selection effect only fires when the id changes, so a reload for the same node has
        // to fetch its tunnels itself. Without this, a connect that created the node's first
        // tunnel left `tunnels` empty: the button read "Disconnect" off the health snapshot while
        // its click had no tunnel to stop, and fell through to connecting again.
        await loadNodeDetails(nextSelectedNodeId);
      }
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, loadNodeDetails, refreshProviderAccessStatus, refreshWireGuardReadiness]);

  /// Name an unclaimed slot so it becomes a device.
  const claimDevice = useCallback(
    async (nodeIdValue: string, peerId: string, label: string): Promise<boolean> => {
      setBusyAction("claim-device");
      try {
        const claimed = await claimNodePeer({ nodeId: nodeIdValue, peerId, label });
        setNodePeers((current) =>
          current.map((peer) => (peer.id === claimed.id ? claimed : peer)),
        );
        pushToast("success", "Device added", `${claimed.label} can now use this server.`);
        return true;
      } catch (error) {
        handleError(error);
        return false;
      } finally {
        setBusyAction(null);
      }
    },
    [handleError, pushToast],
  );

  /// The config and QR for one slot.
  ///
  /// Deliberately not held in context state: it carries the slot’s key, and there is no reason for
  /// it to outlive the dialog showing it.
  const loadDeviceConfig = useCallback(
    async (nodeIdValue: string, peerId: string): Promise<PeerConfigPayload | null> => {
      try {
        return await getPeerConfig({ nodeId: nodeIdValue, peerId });
      } catch (error) {
        handleError(error);
        return null;
      }
    },
    [handleError],
  );

  const exportDeviceConfig = useCallback(
    async (nodeIdValue: string, peerId: string) => {
      setBusyAction("export-device-config");
      try {
        const path = await exportPeerConfig({ nodeId: nodeIdValue, peerId });
        pushToast("success", "Configuration saved", path);
      } catch (error) {
        handleError(error);
      } finally {
        setBusyAction(null);
      }
    },
    [handleError, pushToast],
  );

  const refreshHealthSnapshot = useCallback(async (silent = false) => {
    if (!selectedNodeIdRef.current) {
      return;
    }

    // A silent poll that is already running must not be stacked on. This read can take seconds —
    // its egress probe alone has a three-second timeout — and the 5s interval would otherwise queue
    // several, which then all resolve at once and fight over `health`.
    //
    // A user-initiated refresh is never dropped: the user pressed a button and is owed an answer.
    if (silent && healthPollInFlightRef.current) {
      return;
    }

    if (silent) {
      healthPollInFlightRef.current = true;
    } else {
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
      if (silent) {
        healthPollInFlightRef.current = false;
      } else {
        setBusyAction(null);
      }
    }
  }, [clearThroughputSamples, handleError, recordThroughputSample]);

  const refreshHealth = useCallback(async () => {
    await refreshHealthSnapshot(false);
  }, [refreshHealthSnapshot]);

  /**
   * Checks one provider's stored key and records the result against **that** provider.
   *
   * Per-provider on purpose. A single shared "access state" described whichever provider happened
   * to be selected, so checking Docker moved DigitalOcean's Verified badge onto Docker, and the
   * next check moved it back — the badge looked like a toggle between rows rather than a fact
   * about either.
   *
   * It also deliberately does not change the app-wide selected provider: re-checking a row should
   * not silently repoint node creation at a different account.
   */
  const validateProviderKey = useCallback(async (
    targetProviderId: string,
    options?: { savedJustNow?: boolean },
  ) => {
    setValidatingProviderId(targetProviderId);
    setBusyAction("validate-token");
    try {
      const result = await validateProviderToken(targetProviderId);
      // The global access state describes the selected provider and gates provisioning, so it
      // tracks this result only when this *is* the selected provider.
      if (targetProviderId === providerIdRef.current) {
        setProviderAccessState("ready");
        setProviderStatusMessage(result);
        setProviderFeedback(result);
      }
      setRecovery(null);
      pushToast(
        "success",
        options?.savedJustNow ? "Key saved and verified" : "Key verified",
        result,
      );
      return result;
    } catch (error) {
      const message = handleError(error);
      if (targetProviderId === providerIdRef.current) {
        setProviderAccessState("invalid");
        setProviderStatusMessage(message);
        setProviderFeedback(message);
      }
      return null;
    } finally {
      setValidatingProviderId(null);
      setBusyAction(null);
      // The backend records the verdict on the provider row, so the badge comes from a reload
      // rather than a second copy held here.
      try {
        setProviders(await listProviders());
      } catch {
        // A failed refresh leaves the previous rows in place; the toast already reported the check.
      }
    }
  }, [handleError, pushToast]);

  /**
   * Stores a key and immediately checks it against the provider.
   *
   * One action, because the two used to be separate buttons and the validating one took no token —
   * it read whatever was already stored. Pasting a new key and clicking Validate therefore checked
   * the *old* key and could report success for a key that had never been saved.
   *
   * Returns whether the key was stored, so the caller knows whether it is safe to clear the field.
   */
  const saveToken = useCallback(async (credentials: Record<string, string>, targetProviderId?: string) => {
    const provider = targetProviderId ?? providerId;
    const name = providers.find((entry) => entry.id === provider)?.name ?? providerName;

    setBusyAction("save-token");
    try {
      await registerProviderCredential(provider, name, credentials);
      setRecovery(null);
    } catch (error) {
      setProviderFeedback(handleError(error));
      setBusyAction(null);
      return false;
    }

    // Saved. A validation failure from here is a bad key, not a lost one, so the stored value
    // stays and the field can be cleared.
    await validateProviderKey(provider, { savedJustNow: true });

    await loadDashboard();
    return true;
  }, [handleError, loadDashboard, providerId, providerName, providers, validateProviderKey]);

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
        setDockerReadinessMessage("Save a Docker development key in Setup before validating readiness.");
        if (!silent) {
          pushToast("warning", "Docker key missing", "Add a Docker development key in Setup first.");
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

  /**
   * What happens when a build the user started reaches ready: delete it if they cancelled, else
   * connect to it (one UAC prompt, the same connect a click would start) unless auto-connect is off.
   */
  const finishBuild = useCallback(async (nodeId: string, place: string) => {
    const node = (await listVpnNodes()).find((candidate) => candidate.id === nodeId) ?? null;

    if (cancelBuildRef.current === nodeId) {
      cancelBuildRef.current = null;
      if (node) await destroyNodeNowRef.current(node, "cancelled");
      return;
    }

    if (appPreferencesRef.current?.auto_connect_after_provision === false || !node) {
      pushToast("success", `${place} is ready`, "Connect when you want to use it.");
      return;
    }

    pushToast("success", `${place} is ready — connecting now`, "Windows will ask for permission in a second.");
    void connectNodeRef.current(node);
  }, [pushToast]);

  const requestCancelBuild = useCallback(() => {
    setOperationAttempt((current) => {
      if (current?.kind !== "provision") return current;
      cancelBuildRef.current = current.nodeId;
      return { ...current, cancelRequested: true };
    });
  }, []);

  const createNode = useCallback(async (input: CreateNodeInput) => {
    const place = input.place ?? input.nodeName;
    cancelBuildRef.current = null;
    setBusyAction("create-node");
    setOperationAttempt({
      kind: "provision",
      nodeId: input.nodeId,
      nodeName: input.nodeName,
      place,
      providerName: providerNameFor(providerId),
      startedAt: Date.now(),
      status: null,
      bootstrappingSince: null,
      cancelRequested: false,
    });
    let ready = false;
    try {
      await createVpnNode({
        nodeId: input.nodeId,
        providerId,
        nodeName: input.nodeName,
        region: input.region,
        planKey: input.planKey,
      });
      setRecovery(null);
      await loadDashboard(input.nodeId);
      ready = true;
    } catch (error) {
      handleError(error);
    } finally {
      setOperationAttempt(null);
      setBusyAction(null);
    }
    if (ready) await finishBuild(input.nodeId, place);
    return ready;
  }, [finishBuild, handleError, loadDashboard, providerId, providerNameFor]);

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
    setOperationAttempt({
      kind: "provision",
      nodeId: node.id,
      nodeName: node.name,
      place: node.region_geo?.city ?? node.name,
      providerName: providerNameFor(node.provider_id),
      startedAt: Date.now(),
      status: node.lifecycle_status,
      bootstrappingSince: null,
      cancelRequested: false,
    });
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
      setOperationAttempt(null);
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, providerNameFor, pushToast]);

  const connectNode = useCallback(async (node: VpnNodeRecord) => {
    if (!node) {
      pushToast("warning", "No node selected", "Select a node before connecting.");
      return;
    }
    if (!(await ensureUacExplained(node.region_geo?.city ?? node.name))) return;

    const canonicalTunnelName = `${node.name
      .trim()
      .replace(/[^A-Za-z0-9_-]+/g, "-")
      .replace(/-{2,}/g, "-")
      .replace(/^-|-$/g, "")
      .slice(0, 56) || "primary"}-wg`;

    // Not `busyAction`: that slot disables every control in the app, and a connect runs for tens of
    // seconds while the user has no internet — the worst moment to also freeze the window.
    setConnectAttempt({
      nodeId: node.id,
      tunnelId: null,
      tunnelName: canonicalTunnelName,
      startedAt: Date.now(),
      creatingTunnel: false,
      snapshot: null,
    });

    try {
      let tunnelForNode = tunnels.find((tunnel) => tunnel.node_id === node.id) ?? null;
      const canonicalTunnelId = `tunnel-${node.id}`;

      if (!tunnelForNode) {
        setConnectAttempt((current) =>
          current === null ? current : { ...current, creatingTunnel: true },
        );
        await createAndActivateClientTunnel({
          tunnelId: canonicalTunnelId,
          nodeId: node.id,
          tunnelName: canonicalTunnelName,
          dnsServer: "1.1.1.1",
          allowedIps: ["0.0.0.0/0", "::/0"],
        });

        const refreshedTunnels = await listTunnelsForNode(node.id);
        tunnelForNode = refreshedTunnels[0] ?? null;
      }

      if (!tunnelForNode) {
        throw new Error("Primary tunnel could not be resolved after creation.");
      }

      // Recorded before the command runs so the phase poll has something to ask about while the
      // service is being registered — that window includes the UAC prompt and is where the app
      // previously looked frozen with nothing on screen to explain it.
      const resolvedTunnel = tunnelForNode;
      setConnectAttempt((current) =>
        current === null
          ? current
          : { ...current, tunnelId: resolvedTunnel.id, creatingTunnel: false },
      );

      if (tunnelForNode.status.toLowerCase() !== "connected") {
        await connectTunnelCommand(node.id, tunnelForNode.id);
      }

      // No success toast here. The command resolving means the *service* started, which is roughly
      // fifteen seconds before the tunnel carries traffic. The toast now fires from the phase poll
      // when the verdict actually reaches `connected`.
      setRecovery(null);
      await loadDashboard(node.id);
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
            setConnectAttempt((current) =>
              current === null
                ? current
                : { ...current, tunnelId: recoverableTunnel.id, creatingTunnel: true },
            );
            await createAndActivateClientTunnel({
              tunnelId: recoverableTunnel.id,
              nodeId: recoverableTunnel.node_id,
              tunnelName: canonicalTunnelName,
              dnsServer: "1.1.1.1",
              allowedIps: ["0.0.0.0/0", "::/0"],
            });

            setConnectAttempt((current) =>
              current === null ? current : { ...current, creatingTunnel: false },
            );
            await connectTunnelCommand(node.id, recoverableTunnel.id);

            // Same reasoning as the happy path: the service starting is not the tunnel working, so
            // the phase poll announces success and this only reports the repair.
            setRecovery(null);
            await loadDashboard(node.id);
            return;
          } catch (recoveryError) {
            setConnectAttempt(null);
            handleError(recoveryError);
            return;
          }
        }
      }

      setConnectAttempt(null);
      handleError(error);
    }
    // No `finally` clearing the attempt: on success it stays live so the phase poll can watch the
    // handshake land. The poll owns clearing it.
  }, [ensureUacExplained, handleError, loadDashboard, pushToast, tunnels]);

  useEffect(() => {
    connectNodeRef.current = connectNode;
  }, [connectNode]);

  const connectPrimary = useCallback(async () => {
    if (!activeNode) {
      pushToast("warning", "No node selected", "Select a node before connecting.");
      return;
    }

    void connectNode(activeNode);
  }, [activeNode, connectNode, pushToast]);

  const connectTunnel = useCallback(async (tunnel: TunnelRecord) => {
    const tunnelNode = nodes.find((node) => node.id === tunnel.node_id);
    if (!(await ensureUacExplained(tunnelNode?.region_geo?.city ?? tunnelNode?.name ?? tunnel.name))) return;
    setConnectAttempt({
      nodeId: tunnel.node_id,
      tunnelId: tunnel.id,
      tunnelName: tunnel.name,
      startedAt: Date.now(),
      creatingTunnel: false,
      snapshot: null,
    });

    try {
      await connectTunnelCommand(tunnel.node_id, tunnel.id);
      // Success is announced by the phase poll, once traffic provably crosses the tunnel.
      setRecovery(null);
      await loadDashboard(tunnel.node_id);
    } catch (error) {
      const missingService = toCommandError(error).kind === "wire-guard";

      if (missingService) {
        try {
          setConnectAttempt((current) =>
            current === null ? current : { ...current, creatingTunnel: true },
          );
          await createAndActivateClientTunnel({
            tunnelId: tunnel.id,
            nodeId: tunnel.node_id,
            tunnelName: `${(nodes.find((node) => node.id === tunnel.node_id)?.name ?? tunnel.name)
              .trim()
              .replace(/[^A-Za-z0-9_-]+/g, "-")
              .replace(/-{2,}/g, "-")
              .replace(/^-|-$/g, "")
              .slice(0, 56) || "primary"}-wg`,
            dnsServer: "1.1.1.1",
            allowedIps: ["0.0.0.0/0", "::/0"],
          });

          setConnectAttempt((current) =>
            current === null ? current : { ...current, creatingTunnel: false },
          );
          await connectTunnelCommand(tunnel.node_id, tunnel.id);

          setRecovery(null);
          await loadDashboard(tunnel.node_id);
          return;
        } catch (recoveryError) {
          setConnectAttempt(null);
          handleError(recoveryError);
          return;
        }
      }

      setConnectAttempt(null);
      handleError(error);
    }
  }, [ensureUacExplained, handleError, loadDashboard, nodes]);

  const disconnectTunnel = useCallback(async (tunnel: TunnelRecord) => {
    setBusyAction("disconnect-tunnel");
    // Ends any attempt for this tunnel first, so the progress card cannot outlive the tunnel it is
    // reporting on — and so its poll stops competing with the teardown.
    setConnectAttempt((current) => (current?.tunnelId === tunnel.id ? null : current));
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

  /**
   * Abandons a connect that is taking too long and restores the user's normal connection.
   *
   * Tearing the tunnel down is the only real escape: the adapter holds the default route for as
   * long as it exists, so simply hiding the progress card would leave the user offline with nothing
   * on screen explaining why.
   */
  const cancelConnect = useCallback(async () => {
    const attempt = connectAttempt;
    if (!attempt) return;

    setBusyAction("disconnect-tunnel");
    try {
      if (attempt.tunnelId) {
        await disconnectTunnelCommand(attempt.nodeId, attempt.tunnelId);
      } else {
        // No tunnel record resolved yet, so there is no id to disconnect by. The node-wide stop is
        // the only handle available.
        await forceDisconnectNodeServiceCommand(attempt.nodeId);
      }

      setConnectAttempt(null);
      pushToast(
        "info",
        "Connection cancelled",
        "The tunnel was shut down and your normal connection is back.",
      );
      await loadDashboard(attempt.nodeId);
      await refreshHealthSnapshot(true);
    } catch (error) {
      // The attempt is cleared regardless: leaving the card up would imply a connect is still
      // progressing when the user has asked for it to stop.
      setConnectAttempt(null);
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [connectAttempt, handleError, loadDashboard, pushToast, refreshHealthSnapshot]);

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

  /** Deletes a server at its provider, no questions asked. Callers decide whether to ask first. */
  const destroyNodeNow = useCallback(async (node: VpnNodeRecord, reason?: "cancelled") => {
    setBusyAction("destroy-node");
    setOperationAttempt({
      kind: "destroy",
      nodeId: node.id,
      nodeName: node.name,
      providerName: providerNameFor(node.provider_id),
      startedAt: Date.now(),
    });
    try {
      await destroyVpnNode(node.id);
      setRecovery(null);
      pushToast(
        "success",
        reason === "cancelled" ? "Build cancelled" : "Server turned off",
        `${node.name} was deleted at the provider and is no longer billing.`,
      );
      await loadDashboard();
    } catch (error) {
      handleError(error);
    } finally {
      setOperationAttempt(null);
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, providerNameFor, pushToast]);

  useEffect(() => {
    destroyNodeNowRef.current = destroyNodeNow;
  }, [destroyNodeNow]);

  const destroyNode = useCallback(async (node: VpnNodeRecord) => {
    // Names the location as well as the node: with the fleet keyed by city, "which one is this?"
    // is the question a user needs answered before deleting a server they pay for.
    const where = node.region_geo?.city ?? node.region;
    const confirmed = window.confirm(
      `Turn off ${node.name} in ${where}?\n\nThis deletes the server at your provider and stops its billing. The location stays saved, so you can turn it back on later.`,
    );
    if (confirmed) await destroyNodeNow(node);
  }, [destroyNodeNow]);


  const turnOnLocation = useCallback(async (location: SavedLocationRecord) => {
    // Billing starts the moment the server exists, so the quote is today's price. A catalog that
    // cannot be read falls back to the last price seen, said as such; a plan the provider no longer
    // offers is refused here rather than failing at the provider.
    let livePlan: ProviderPlanPricing | null | undefined;
    try {
      const pricing = await listProviderPricing(location.provider_id, location.region);
      livePlan = pricing.plans.find((plan) => plan.plan_key === location.plan_key) ?? null;
    } catch {
      livePlan = undefined;
    }

    const question = describeTurnOn(location, livePlan);
    if (question === null) {
      pushToast(
        "warning",
        "Plan no longer offered",
        `${location.provider_name ?? "The provider"} no longer offers this plan in ${location.name}. Create a new server there instead, then forget this one.`,
      );
      return;
    }
    // The button that started this already showed the saved price. Ask again only when today's
    // price differs or could not be read, so billing never starts at a price the user has not seen.
    const priceAsShown = livePlan != null
      && livePlan.monthly_usd === location.price_monthly_usd
      && livePlan.hourly_usd === location.price_hourly_usd;
    if (!priceAsShown && !window.confirm(question)) {
      return;
    }

    const nodeId = `node-${crypto.randomUUID().slice(0, 8)}`;
    const place = location.region_geo?.city ?? location.name;
    cancelBuildRef.current = null;
    setBusyAction("create-node");
    setOperationAttempt({
      kind: "provision",
      nodeId,
      nodeName: location.name,
      place,
      providerName: location.provider_name ?? providerNameFor(location.provider_id),
      startedAt: Date.now(),
      status: null,
      bootstrappingSince: null,
      cancelRequested: false,
    });
    let ready = false;
    try {
      await turnOnSavedLocation(location.id, nodeId);
      setRecovery(null);
      await loadDashboard(nodeId);
      ready = true;
    } catch (error) {
      handleError(error);
      // A failed create leaves its node in error; the list shows that rather than "starting".
      await loadDashboard();
    } finally {
      setOperationAttempt(null);
      setBusyAction(null);
    }
    if (ready) await finishBuild(nodeId, place);
  }, [finishBuild, handleError, loadDashboard, providerNameFor, pushToast]);

  const turnOffLocation = useCallback(async (location: SavedLocationRecord) => {
    if (!location.node_id) return;

    const confirmed = window.confirm(
      `Turn off ${location.name}?\n\nThis deletes the server at your provider and stops its billing. The location stays saved, so you can turn it back on later.`,
    );
    if (!confirmed) return;

    setBusyAction("destroy-node");
    setOperationAttempt({
      kind: "destroy",
      nodeId: location.node_id,
      nodeName: location.name,
      providerName: location.provider_name ?? providerNameFor(location.provider_id),
      startedAt: Date.now(),
    });
    try {
      await turnOffSavedLocation(location.id);
      setRecovery(null);
      pushToast("success", "Server turned off", `${location.name} was deleted at the provider and is no longer billing.`);
      await loadDashboard();
    } catch (error) {
      handleError(error);
    } finally {
      setOperationAttempt(null);
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, providerNameFor, pushToast]);

  const forgetLocation = useCallback(async (location: SavedLocationRecord) => {
    const confirmed = window.confirm(
      `Forget ${location.name}?\n\nIt is removed from your saved locations. Nothing at the provider changes.`,
    );
    if (!confirmed) return;

    setBusyAction("forget-location");
    try {
      await forgetSavedLocation(location.id);
      setSavedLocations((current) => current.filter((candidate) => candidate.id !== location.id));
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError]);

  /**
   * Force cleanup for a node named by id.
   *
   * Shield's log recovers the node a failed run names, which is not necessarily the selected
   * one — so the id is the parameter and selection is a caller's concern.
   */
  const forceCleanupNode = useCallback(async (nodeId: string) => {
    const node = nodes.find((candidate) => candidate.id === nodeId) ?? null;

    setBusyAction("force-cleanup");
    setOperationAttempt({
      kind: "force-cleanup",
      nodeId,
      nodeName: node?.name ?? nodeId,
      providerName: providerNameFor(node?.provider_id ?? null),
      startedAt: Date.now(),
    });
    try {
      const result = await forceCleanupVpnNode(nodeId);
      setRecovery(null);

      const label = node?.name ?? nodeId;
      // A remote delete that was tried and failed leaves a server running that nothing in the app
      // points at any more — it keeps billing until the user removes it themselves. The backend
      // records that case as an error; reporting "cleanup complete" here would bury it.
      if (result.remote_node_delete_attempted && !result.remote_node_delete_succeeded) {
        pushToast(
          "warning",
          "Server may still be running",
          `Local records for ${label} were removed, but the server could not be deleted and may still be billing. Remove it in your provider's console, then check the Activity page for details.`,
        );
      } else {
        pushToast(
          "success",
          "Force cleanup complete",
          `Removed ${result.local_tunnels_removed} tunnel(s) for ${label}.`,
        );
      }

      await loadDashboard(nodeId);
    } catch (error) {
      handleError(error);
    } finally {
      setOperationAttempt(null);
      setBusyAction(null);
    }
  }, [handleError, loadDashboard, nodes, providerNameFor, pushToast]);

  const forceCleanupSelectedNode = useCallback(async () => {
    if (!selectedNode) {
      pushToast("warning", "No node selected", "Select a node before forcing cleanup.");
      return;
    }

    await forceCleanupNode(selectedNode.id);
  }, [forceCleanupNode, pushToast, selectedNode]);

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
      // The export just created the folder, so its Open button must stop being disabled.
      setAppLocations(await listAppLocations());
    } catch (error) {
      handleError(error);
    } finally {
      setBusyAction(null);
    }
  }, [handleError, pushToast]);

  /** Copies any single value — a path, a config — with a toast naming what was copied. */
  const copyText = useCallback(async (value: string, label: string) => {
    try {
      await navigator.clipboard.writeText(value);
      pushToast("success", "Copied", `${label} copied to clipboard.`);
    } catch (error) {
      handleError(error);
    }
  }, [handleError, pushToast]);

  const revealLocation = useCallback(async (key: string) => {
    try {
      await revealAppLocation(key);
    } catch (error) {
      handleError(error);
    }
  }, [handleError]);

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
      // Deliberately keeps running during a connect. Its egress probe is the traffic that proves
      // the tunnel carries anything — the phase verdict waits for decrypted inbound bytes, and on
      // an otherwise idle machine something has to generate them. Suspending this poll produced a
      // connect that could sit at "waiting for the server" while the tunnel was already working.
      //
      // Safe to leave running now that commands execute off the UI thread: the three-second egress
      // timeout it can hit during the handshake wait no longer blocks the window.
      void refreshHealthSnapshot(true);
    }, 5000);

    return () => window.clearInterval(intervalId);
  }, [refreshHealthSnapshot, selectedNodeId]);

  /**
   * Watches a connect through to the point where traffic actually crosses the tunnel.
   *
   * This is what replaced announcing success when `connect_tunnel` returned. That command resolves
   * when the Windows service is running, which in the reported failure was fifteen seconds before
   * the first handshake — the entire time the user had no internet and the app showed green.
   *
   * Runs at 1 Hz because the user is staring at a dead connection and the elapsed counter has to
   * look alive. `get_tunnel_phase` is built for that rate: one `sc query` and an in-process counter
   * read, no credential access and no network call.
   */
  useEffect(() => {
    const nodeId = connectAttempt?.nodeId;
    const tunnelId = connectAttempt?.tunnelId;

    // Nothing to poll until a tunnel record exists — during creation there is no service yet.
    if (!nodeId || !tunnelId) {
      return;
    }

    let cancelled = false;
    let inFlight = false;
    // A tunnel whose service has not been registered yet reports `disconnected`, and that is the
    // normal opening state of a connect — the service is created partway through, after the UAC
    // prompt. Only a service that was up and then went away is a failure, so the two are told apart
    // by whether one was ever observed.
    let serviceWasSeenUp = false;

    const poll = async () => {
      // The command runs off the UI thread now, so a slow read no longer blocks the window — but
      // it can still outlive its tick, and overlapping reads would queue up behind each other.
      if (inFlight) return;
      inFlight = true;

      try {
        const snapshot = await getTunnelPhase(nodeId, tunnelId);
        if (cancelled) return;

        setConnectAttempt((current) => {
          // A different attempt started, or the user cancelled, while this read was in flight.
          if (current === null || current.tunnelId !== tunnelId) return current;
          return { ...current, snapshot };
        });

        if (snapshot.phase !== "disconnected") {
          serviceWasSeenUp = true;
        }

        if (snapshot.phase === "connected" || snapshot.phase === "degraded") {
          setConnectAttempt(null);
          pushToast(
            "success",
            "Tunnel connected",
            `${snapshot.tunnel_name} is carrying traffic.`,
          );
          // From now on HOLD ON is the compact version: the user has seen it work once.
          if (appPreferencesRef.current && !appPreferencesRef.current.first_connect_done) {
            void updateAppPreferences({ first_connect_done: true });
          }
          // Forced refresh: this both updates the egress card immediately and generates the
          // traffic that keeps proving the tunnel works.
          void refreshHealthSnapshot(true);
        } else if (snapshot.phase === "disconnected" && serviceWasSeenUp) {
          // The service was up and has gone away — the connect failed rather than completed, and
          // leaving the card up would show progress that is no longer happening.
          setConnectAttempt(null);
          pushToast(
            "warning",
            "Connection did not start",
            `${snapshot.tunnel_name} stopped before it carried any traffic. Try connecting again.`,
          );
          void refreshHealthSnapshot(true);
        }
      } catch (error) {
        if (cancelled) return;
        // A failed status read is not a failed connect — the tunnel may still be coming up — so the
        // attempt continues and the next tick tries again. Surfacing a toast per second here would
        // bury the progress card the user is reading.
        console.warn("connect phase poll failed", error);
      } finally {
        inFlight = false;
      }
    };

    void poll();
    const intervalId = window.setInterval(() => void poll(), 1000);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [connectAttempt?.nodeId, connectAttempt?.tunnelId, pushToast, refreshHealthSnapshot, updateAppPreferences]);

  /**
   * Follows a provision's lifecycle status so the progress bar can jump to the stage the backend
   * has reached. `create_vpn_node_to_ready` blocks until the server is ready and reports nothing
   * on the way, but it writes each status transition to SQLite, and `list_vpn_nodes` reads that.
   */
  const provisionNodeId = operationAttempt?.kind === "provision" ? operationAttempt.nodeId : null;
  useEffect(() => {
    if (!provisionNodeId) {
      return;
    }

    let cancelled = false;
    let inFlight = false;

    const poll = async () => {
      if (inFlight) return;
      inFlight = true;

      try {
        const node = (await listVpnNodes()).find((candidate) => candidate.id === provisionNodeId);
        if (cancelled || !node) return;

        setOperationAttempt((current) => {
          if (current?.kind !== "provision" || current.nodeId !== provisionNodeId) return current;
          const bootstrappingSince =
            current.bootstrappingSince ?? (node.lifecycle_status === "bootstrapping" ? Date.now() : null);
          if (current.status === node.lifecycle_status && current.bootstrappingSince === bootstrappingSince) {
            return current;
          }
          return { ...current, status: node.lifecycle_status, bootstrappingSince };
        });
      } catch (error) {
        // The bar keeps estimating on the clock; the provision itself reports its own failure.
        if (!cancelled) console.warn("provision status poll failed", error);
      } finally {
        inFlight = false;
      }
    };

    void poll();
    const intervalId = window.setInterval(() => void poll(), 1500);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [provisionNodeId]);

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
      // The backend clears the stored verdict along with the key, so the reload below drops it.
      await deleteProviderCommand(provider.id);
      if (provider.slug === "docker") {
        // `dockerKeyStored` follows the reloaded provider list, so only the readiness result needs
        // clearing here.
        setDockerReadiness(null);
        setDockerReadinessState("missing");
        setDockerReadinessMessage("Save a Docker development key in Setup before validating readiness.");
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
    operationAttempt,
    connectAttempt,
    cancelConnect,
    toasts,
    recovery,
    nodes,
    savedLocations,
    turnOnLocation,
    turnOffLocation,
    forgetLocation,
    providers,
    tunnels,
    nodePeers,
    claimDevice,
    loadDeviceConfig,
    exportDeviceConfig,
    selectedNodeId,
    setSelectedNodeId,
    selectedTunnelId,
    setSelectedTunnelId,
    health,
    diagnostics,
    diagnosticsExportPath,
    appLocations,
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
    tunnelPhase,
    currentNodeId,
    setCurrentNode,
    dismissRecovery,
    wireguardReadiness,
    refreshWireGuardReadiness,
    healthUpdatedAt,
    refreshDiagnostics,
    tunnelSettings,
    saveTunnelSettings,
    appPreferences,
    saveAppPreferences,
    updateAppPreferences,
    requestCancelBuild,
    uacPrompt,
    answerUacPrompt,
    throughputSamples,
    loadDashboard,
    refreshHealth,
    saveToken,
    validateProviderKey,
    validatingProviderId,
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
    forceCleanupNode,
    copyDiagnostics,
    exportDiagnostics,
    copyText,
    revealLocation,
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