import { invoke } from "@tauri-apps/api/core";

type TauriWindow = Window & {
  __TAURI_INTERNALS__?: unknown;
};

export type NodeLifecycleStatus =
  | "new"
  | "provisioning"
  | "bootstrapping"
  | "ready"
  | "connected"
  | "error"
  | "deleting"
  // The backend emits this for nodes whose remote resource is gone. It was missing from this
  // union, so those rows rendered unstyled and broke the status filter.
  | "deleted";

/**
 * Machine-readable failure category from the backend.
 *
 * The UI branches on this. It must never go back to matching on message text: errors used to
 * cross the boundary as bare strings, so classification meant substring checks against phrases
 * like "OpenService FAILED 1060", and every backend copy edit silently broke UI recovery.
 */
export type CommandErrorKind =
  | "invalid-state"
  | "missing-resource"
  | "provider"
  | "wire-guard"
  | "persistence"
  | "secret-storage"
  | "invalid-input"
  | "dependency"
  | "unknown";

export interface CommandErrorPayload {
  kind: CommandErrorKind;
  message: string;
  next_action: string;
}

/** A backend failure with its structure intact. */
export class CommandError extends Error {
  readonly kind: CommandErrorKind;
  readonly nextAction: string;

  constructor(payload: CommandErrorPayload) {
    super(payload.message);
    this.name = "CommandError";
    this.kind = payload.kind;
    this.nextAction = payload.next_action;
  }
}

function isCommandErrorPayload(value: unknown): value is CommandErrorPayload {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<CommandErrorPayload>;
  return typeof candidate.kind === "string"
    && typeof candidate.message === "string"
    && typeof candidate.next_action === "string";
}

/**
 * Normalize whatever the bridge threw into a `CommandError`.
 *
 * Anything unrecognized becomes `unknown` rather than being guessed at from its text.
 */
export function toCommandError(error: unknown): CommandError {
  if (error instanceof CommandError) return error;

  if (isCommandErrorPayload(error)) return new CommandError(error);

  const message = error instanceof Error ? error.message : String(error);
  return new CommandError({
    kind: "unknown",
    message,
    next_action: "Retry the action. If it keeps failing, export a diagnostics bundle from the Activity page.",
  });
}

export type TunnelStatus =
  | "new"
  | "ready"
  | "connected"
  | "disconnected"
  | "error"
  | "deleting";


export type ProviderStatus = "active" | "suspended";

/** Outcome of the last key check, persisted so it survives a restart. */
export type ProviderValidationStatus = "ok" | "failed";

export interface ProviderRecord {
  id: string;
  name: string;
  slug: string;
  status: ProviderStatus;
  token_ref: string | null;
  /** Null when the key has never been checked, or when it was cleared because the key changed. */
  last_validation_status: ProviderValidationStatus | null;
  /** When that check ran, so a verdict can show its age rather than implying it is current. */
  last_validated_at: string | null;
  created_at: string;
  updated_at: string;
}

/**
 * One credential input a provider needs, as declared by its adapter.
 *
 * Mirrors `CredentialField` in `src-tauri/src/backend/provider/mod.rs`. Bindings are hand-written,
 * so a change to the Rust struct has to be made here in the same commit — nothing enforces it.
 */
export interface CredentialField {
  /** Stable machine key, and the key to use in the `credentials` record when saving. */
  key: string;
  label: string;
  placeholder: string;
  /** Whether the input should be masked. An AWS access key id is not a secret; its pair is. */
  secret: boolean;
  help: string | null;
}

/**
 * Where a region physically is.
 *
 * Only `slug` is guaranteed — providers add regions faster than a curated table follows, and an
 * unrecognised slug carries no geography rather than an invented city. Render the slug when `city`
 * is null; never guess.
 */
export interface ProviderRegion {
  slug: string;
  city: string | null;
  /** State or province, where it disambiguates. */
  area: string | null;
  country: string | null;
  /** ISO-3166 alpha-2, for rendering a flag. */
  country_code: string | null;
  continent: string | null;
}

export interface VpnNodeRecord {
  id: string;
  provider_id: string;
  name: string;
  region: string;
  plan_key: string | null;
  size_slug: string | null;
  price_monthly_usd: number | null;
  price_hourly_usd: number | null;
  price_source_timestamp: string | null;
  lifecycle_status: NodeLifecycleStatus;
  provider_status?: string;
  /** Geography for `region`, attached at list time by the owning adapter. */
  region_geo?: ProviderRegion;
  external_ref: string | null;
  public_ipv4: string | null;
  server_public_key: string | null;
  error_reason: string | null;
  created_at: string;
  updated_at: string;
  /** UDP port the client dials, as reported by the provider adapter at provision time. */
  endpoint_port: number | null;
}

export interface TunnelRecord {
  id: string;
  node_id: string;
  name: string;
  status: TunnelStatus;
  /** The peer slot supplying this tunnel’s key and address. */
  peer_id: string | null;
  interface_name: string | null;
  assigned_ipv4: string | null;
  last_handshake_age_seconds: number | null;
  rx_bytes: number | null;
  tx_bytes: number | null;
  stats_updated_at: string | null;
  last_error: string | null;
  /** When the current session began. `null` means not connected, or start time unknown. */
  connected_at: string | null;
  created_at: string;
  updated_at: string;
}

/**
 * Which measurement produced the byte counters.
 *
 * `peer` is `wg show` (per-peer WireGuard counters, needs admin); `adapter` is the interface's
 * own lifetime octets (no elevation). They count different things, so anything differencing
 * samples over time must reset its baseline when this changes.
 */
export type TunnelMetricsSource = "peer" | "adapter";

/**
 * How far along a connect is — as opposed to whether the Windows service is running.
 *
 * The distinction is the whole point. The tunnel adapter captures the default route the moment its
 * service starts, so from that instant the user's traffic is inside a tunnel whose peer has not
 * answered yet and their internet is down. In the reported failure the service was up at 10:22:07
 * and the first handshake landed at 10:22:23 — fifteen seconds during which the old UI showed
 * green.
 *
 * Decided in Rust by `classify_tunnel_phase`, from evidence readable without administrator:
 * decrypted inbound bytes on the adapter, which cannot be non-zero without a completed handshake.
 * `wg show` handshake data needs elevation and is unavailable in a normal run, so it is an
 * enrichment here, never the gate.
 */
export type TunnelPhase =
  /** No service, or it is stopped. Traffic uses the normal connection. */
  | "disconnected"
  /** Service up, nothing has come back through it — the user has no working internet right now. */
  | "connecting"
  /** Still connecting past the point a working server would have answered. */
  | "stalled"
  /** Traffic has provably crossed the tunnel. The only phase that may render as green. */
  | "connected"
  /** A handshake happened, but long enough ago that the link may be dead. */
  | "degraded";

/** The raw `sc query` verdict behind a {@link TunnelPhase}. */
export type TunnelServiceState = "running" | "starting" | "stopped" | "unknown";

/**
 * One reading from `get_tunnel_phase`, the cheap command polled while a connect is in flight.
 *
 * Carries the evidence next to the verdict so the UI can say why it is still waiting rather than
 * showing an unexplained spinner.
 */
export interface TunnelPhaseSnapshot {
  tunnel_id: string;
  tunnel_name: string;
  phase: TunnelPhase;
  service_state: TunnelServiceState;
  /** Seconds since the app asked the service to start; `null` when it was already running. */
  elapsed_seconds: number | null;
  /** Decrypted inbound octets. `> 0` is the unelevated proof a handshake completed. */
  rx_bytes: number | null;
  tx_bytes: number | null;
}

export interface HealthPanelData {
  node_id: string;
  tunnel_id: string | null;
  /** The provisioned server's address. Not the machine's egress address. */
  endpoint_ip: string | null;
  handshake_age_seconds: number | null;
  rx_bytes: number | null;
  tx_bytes: number | null;
  /** The address the internet currently sees, measured in every state. `null` if the probe failed. */
  external_ipv4: string | null;
  /**
   * Present only when this machine has IPv6 egress.
   *
   * The tunnel is IPv4-only, so a value here while connected means IPv6 traffic is bypassing it.
   */
  external_ipv6: string | null;
  /**
   * The stored row's status, which tracks the Windows *service*.
   *
   * Reaches `"connected"` as soon as the service starts. Use {@link HealthPanelData.phase} to
   * decide what the user is told — this reads connected during the window where their internet is
   * down.
   */
  tunnel_status: string | null;
  /** Whether the tunnel actually carries traffic. This is what may turn the UI green. */
  phase: TunnelPhase;
  metrics_source: TunnelMetricsSource | null;
  connected_at: string | null;
  kill_switch: KillSwitchState;
  unexpected_drop: UnexpectedDrop | null;
}

export type KillSwitchReason = "active" | "tunnel_not_running" | "routes_scoped";

/**
 * Whether untunneled traffic is currently blocked.
 *
 * The app installs no firewall rules. WireGuard for Windows enables WFP filters that block traffic
 * outside the tunnel when a config has one peer, an `AllowedIPs` carrying a `/0` route, and no
 * `Table = off`. This reports whether that actually holds for the running tunnel — including the
 * case where routes were scoped to a local endpoint and nothing is blocked.
 */
export interface KillSwitchState {
  active: boolean;
  blocks_ipv4: boolean;
  blocks_ipv6: boolean;
  reason: KillSwitchReason;
  /** The routes the running config carries, so the UI can show its working. */
  routes: string[];
}

/** A tunnel that stopped without the app asking it to, leaving traffic unprotected. */
export interface UnexpectedDrop {
  tunnel_id: string;
  node_id: string;
  observed_at_epoch_ms: number;
}

export interface DiagnosticsBundle {
  generated_at: string;
  providers: ProviderRecord[];
  nodes: VpnNodeRecord[];
  tunnels: TunnelRecord[];
  activity?: ActivityEvent[];
  redacted_notes: string[];
  wireguard_debug?: {
    commands: string[];
    last_import_tunnel_name: string | null;
    last_import_config: string | null;
    last_import_config_temp_path: string | null;
  };
}

/** What the app did, one recorded step at a time. Stages of one run share a `correlation_id`. */
export interface ActivityEvent {
  id: number;
  /** Null for events belonging to the app rather than to one node. */
  node_id: string | null;
  provider_slug: string | null;
  /** `provision` | `connect` | `disconnect` | `destroy` | `cleanup` | `app` */
  category: string;
  stage: string;
  /** `info` | `success` | `error` */
  outcome: string;
  correlation_id: string | null;
  /** Provider credentials are redacted at the write boundary; WireGuard key material is not. */
  detail: string;
  created_at: string;
}

/**
 * A folder or file the user may need to open when reporting a problem.
 *
 * `reveal_app_location` takes the `key` back, never `path` — the backend resolves it, so the
 * webview cannot hand an arbitrary target to the Windows shell.
 */
export interface AppLocation {
  key: string;
  label: string;
  description: string;
  path: string;
  /** False until the app has written something there. */
  exists: boolean;
}

export interface WireGuardReadiness {
  installed: boolean;
  executable_found: boolean;
  service_available: boolean;
  /** Installed WireGuard version from `wg --version`, or null when it could not be read. */
  version: string | null;
  /** This app's own version. */
  app_version: string;
  message: string;
  action_guidance: string;
}

export interface WireGuardInstallResult {
  installed: boolean;
  prerequisites_configured: boolean;
  installer_path: string;
  message: string;
  next_action: string;
}

export interface DockerReadiness {
  installed: boolean;
  daemon_reachable: boolean;
  cli_version: string | null;
  daemon_version: string | null;
  message: string;
  action_guidance: string;
}

export interface ProviderPlanPricing {
  plan_key: string;
  plan_label: string;
  size_slug: string;
  region: string;
  memory_mb: number;
  vcpus: number;
  disk_gb: number;
  transfer_tb: number;
  transfer_note: string;
  // `null` where the provider publishes no price. Not zero: rendered, zero says the plan is free,
  // and an OpenStack cloud whose operator sets its own prices is emphatically not.
  monthly_usd: number | null;
  hourly_usd: number | null;
  estimated: boolean;
  source_timestamp: string;
}

export interface ProviderPricingResponse {
  provider_id: string;
  region: string;
  plans: ProviderPlanPricing[];
}

async function invokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (typeof window === "undefined") {
    throw new CommandError({
      kind: "dependency",
      message: "UI is rendering on the server.",
      next_action: "This view must run in the desktop app window.",
    });
  }

  const tauriWindow = window as TauriWindow;
  if (!tauriWindow.__TAURI_INTERNALS__) {
    throw new CommandError({
      kind: "dependency",
      message: "Running in browser mode, so the Rust backend is unreachable.",
      next_action: "Start the desktop app with npm run tauri:dev.",
    });
  }

  // Every rejection leaves this function as a CommandError, so callers never parse message text.
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw toCommandError(error);
  }
}

export async function checkWireGuardReadiness(): Promise<WireGuardReadiness> {
  return invokeCommand<WireGuardReadiness>("check_wireguard_readiness");
}

export async function checkDockerReadiness(): Promise<DockerReadiness> {
  return invokeCommand<DockerReadiness>("check_docker_readiness");
}

export async function openWireGuardInstallPage(): Promise<void> {
  await invokeCommand<void>("open_wireguard_install_page");
}

export async function installWireGuardWithPrerequisites(): Promise<WireGuardInstallResult> {
  return invokeCommand<WireGuardInstallResult>("install_wireguard_with_prerequisites");
}

/**
 * Register a credential for any provider.
 *
 * The backend resolves the adapter from the provider's own record, so this no longer picks a
 * command name by slug — adding a provider does not mean adding another binding here.
 *
 * `credentials` is keyed by the `key` of each field from {@link listProviderCredentialFields}.
 * DigitalOcean supplies one entry; AWS supplies an access key id and a secret access key.
 */
export async function registerProviderCredential(
  providerId: string,
  providerName: string,
  credentials: Record<string, string>,
): Promise<string> {
  return invokeCommand<string>("register_provider_credential", {
    providerId,
    providerName,
    credentials,
  });
}

/**
 * The credential inputs a provider needs, declared by its adapter.
 *
 * The settings modal renders whatever this returns, so a provider that needs two fields does not
 * require a UI change — and the UI never branches on a provider slug to decide what to show.
 */
export async function listProviderCredentialFields(providerId: string): Promise<CredentialField[]> {
  return invokeCommand<CredentialField[]>("list_provider_credential_fields", { providerId });
}

export async function validateProviderToken(providerId: string): Promise<string> {
  return invokeCommand<string>("validate_provider_token", { providerId });
}

export async function listProviderPricing(providerId: string, region: string): Promise<ProviderPricingResponse> {
  return invokeCommand<ProviderPricingResponse>("list_provider_pricing", { providerId, region });
}

export async function listProviderRegions(providerId: string): Promise<ProviderRegion[]> {
  return invokeCommand<ProviderRegion[]>("list_provider_regions", { providerId });
}

/** The node the app connects to by default, or null when the user has not chosen between several. */
export async function getCurrentNodeId(): Promise<string | null> {
  return invokeCommand<string | null>("get_current_node_id");
}

export async function setCurrentNodeId(nodeId: string | null): Promise<void> {
  return invokeCommand<void>("set_current_node_id", { nodeId });
}

/**
 * Client tunnel parameters.
 *
 * Rendered into the `.conf` when a tunnel next starts — a running tunnel keeps the config it was
 * started with, so a change needs a reconnect to take effect.
 */
export interface TunnelSettings {
  /** `null` means no `DNS` line at all, leaving the system resolver in place. */
  dns_server: string | null;
  /** `0` disables keepalive. */
  keepalive_seconds: number;
}

export async function getTunnelSettings(): Promise<TunnelSettings> {
  return invokeCommand<TunnelSettings>("get_tunnel_settings");
}

export async function setTunnelSettings(settings: TunnelSettings): Promise<void> {
  return invokeCommand<void>("set_tunnel_settings", {
    dnsServer: settings.dns_server,
    keepaliveSeconds: settings.keepalive_seconds,
  });
}

export async function createVpnNode(params: {
  nodeId: string;
  providerId: string;
  nodeName: string;
  region: string;
  planKey: string;
}): Promise<string> {
  return invokeCommand<string>("create_vpn_node_to_ready", {
    nodeId: params.nodeId,
    providerId: params.providerId,
    nodeName: params.nodeName,
    region: params.region,
    planKey: params.planKey,
  });
}

export async function createAndActivateClientTunnel(params: {
  tunnelId: string;
  nodeId: string;
  tunnelName: string;
  dnsServer: string | null;
  allowedIps: string[];
}): Promise<string> {
  return invokeCommand<string>("create_and_activate_client_tunnel", {
    tunnelId: params.tunnelId,
    nodeId: params.nodeId,
    tunnelName: params.tunnelName,
    dnsServer: params.dnsServer,
    allowedIps: params.allowedIps,
  });
}

/**
 * One baked `[Peer]` slot on a provisioned server.
 *
 * A node is created with a fixed number of these and can never gain more: the app cannot reach
 * a server after it exists, so the slot list it boots with is final. `label === null` means the
 * slot is unclaimed and free to give to a device.
 */
export interface NodePeerRecord {
  id: string;
  node_id: string;
  /** Position in the server’s peer list. Slot 0 is this PC. */
  slot_index: number;
  label: string | null;
  client_public_key: string;
  client_address_cidr: string;
  claimed_at: string | null;
  created_at: string;
}

/**
 * A device configuration and the QR symbol encoding it.
 *
 * `config_text` carries its `PrivateKey`, which is what makes it importable by the official
 * WireGuard app at all. WireGuard key material is not treated as a secret in this app — the
 * server it belongs to lives for hours and is then destroyed.
 */
export interface PeerConfigPayload {
  config_text: string;
  /** Width and height of the QR symbol, in modules. */
  qr_size: number;
  /** Row-major dark/light modules, `qr_size * qr_size` of them. */
  qr_modules: boolean[];
}

export async function listNodePeers(nodeId: string): Promise<NodePeerRecord[]> {
  return invokeCommand<NodePeerRecord[]>("list_node_peers", { nodeId });
}

export async function claimNodePeer(params: {
  nodeId: string;
  peerId: string;
  label: string;
}): Promise<NodePeerRecord> {
  return invokeCommand<NodePeerRecord>("claim_node_peer", {
    nodeId: params.nodeId,
    peerId: params.peerId,
    label: params.label,
  });
}

export async function getPeerConfig(params: {
  nodeId: string;
  peerId: string;
}): Promise<PeerConfigPayload> {
  return invokeCommand<PeerConfigPayload>("get_peer_config", {
    nodeId: params.nodeId,
    peerId: params.peerId,
  });
}

/** Writes the config into the app data directory and resolves to the path it wrote. */
export async function exportPeerConfig(params: {
  nodeId: string;
  peerId: string;
}): Promise<string> {
  return invokeCommand<string>("export_peer_config", {
    nodeId: params.nodeId,
    peerId: params.peerId,
  });
}

export async function destroyVpnNode(nodeId: string): Promise<void> {
  await invokeCommand<void>("destroy_vpn_node", { nodeId });
}

export interface ForceCleanupResult {
  node_id: string;
  correlation_id: string;
  local_tunnels_removed: number;
  remote_droplet_delete_attempted: boolean;
  remote_droplet_delete_succeeded: boolean;
  node_record_removed: boolean;
}

export async function forceCleanupVpnNode(nodeId: string): Promise<ForceCleanupResult> {
  return invokeCommand<ForceCleanupResult>("force_cleanup_vpn_node", { nodeId });
}

export async function listVpnNodes(): Promise<VpnNodeRecord[]> {
  return invokeCommand<VpnNodeRecord[]>("list_vpn_nodes");
}

export async function listProviders(): Promise<ProviderRecord[]> {
  return invokeCommand<ProviderRecord[]>("list_providers");
}

export async function suspendProvider(providerId: string): Promise<void> {
  await invokeCommand<void>("suspend_provider", { providerId });
}

export async function resumeProvider(providerId: string): Promise<void> {
  await invokeCommand<void>("resume_provider", { providerId });
}

export async function deleteProvider(providerId: string): Promise<void> {
  await invokeCommand<void>("delete_provider", { providerId });
}

export async function listTunnelsForNode(nodeId: string): Promise<TunnelRecord[]> {
  return invokeCommand<TunnelRecord[]>("list_tunnels_for_node", { nodeId });
}

/**
 * Registers and starts the tunnel service. Resolving means the service is *running*, not that the
 * tunnel carries traffic — poll {@link getTunnelPhase} for that.
 */
export async function connectTunnel(nodeId: string, tunnelId: string): Promise<void> {
  await invokeCommand<void>("connect_tunnel", { nodeId, tunnelId });
}

/**
 * How far along a connect is. Cheap enough to poll once a second while one is in flight.
 *
 * Costs one `sc query` process spawn and an in-process counter read — no credential-manager access
 * and no egress HTTP probe, unlike {@link getHealthPanelData}.
 */
export async function getTunnelPhase(
  nodeId: string,
  tunnelId: string,
): Promise<TunnelPhaseSnapshot> {
  return invokeCommand<TunnelPhaseSnapshot>("get_tunnel_phase", { nodeId, tunnelId });
}

export async function disconnectTunnel(nodeId: string, tunnelId: string): Promise<void> {
  await invokeCommand<void>("disconnect_tunnel", { nodeId, tunnelId });
}

export async function forceDisconnectNodeService(nodeId: string): Promise<void> {
  await invokeCommand<void>("force_disconnect_node_service", { nodeId });
}

/**
 * @param forceIpProbe bypasses the backend's egress-IP cache. Set it for a user-initiated
 * refresh; leave it off for the ambient poll, which would otherwise hammer a third-party service.
 */
export async function getHealthPanelData(
  nodeId: string,
  tunnelId?: string | null,
  forceIpProbe = false,
): Promise<HealthPanelData> {
  return invokeCommand<HealthPanelData>("get_health_panel_data", {
    nodeId,
    tunnelId: tunnelId ?? null,
    forceIpProbe,
  });
}

export async function getDiagnosticsBundle(): Promise<DiagnosticsBundle> {
  return invokeCommand<DiagnosticsBundle>("get_diagnostics_bundle");
}

export async function exportDiagnosticsBundle(): Promise<string> {
  return invokeCommand<string>("export_diagnostics_bundle");
}

export async function listAppLocations(): Promise<AppLocation[]> {
  return invokeCommand<AppLocation[]>("list_app_locations");
}

/** Opens one of the locations from `listAppLocations` in Explorer. Takes its key, never a path. */
export async function revealAppLocation(key: string): Promise<void> {
  return invokeCommand<void>("reveal_app_location", { key });
}