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
  | "deleting";

export type TunnelStatus =
  | "new"
  | "ready"
  | "connected"
  | "disconnected"
  | "error"
  | "deleting";

export interface ProviderRecord {
  id: string;
  name: string;
  slug: string;
  token_ref: string | null;
  created_at: string;
  updated_at: string;
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
  external_ref: string | null;
  public_ipv4: string | null;
  server_public_key: string | null;
  error_reason: string | null;
  created_at: string;
  updated_at: string;
}

export interface TunnelRecord {
  id: string;
  node_id: string;
  name: string;
  status: TunnelStatus;
  interface_name: string | null;
  assigned_ipv4: string | null;
  last_handshake_age_seconds: number | null;
  rx_bytes: number | null;
  tx_bytes: number | null;
  stats_updated_at: string | null;
  last_error: string | null;
  created_at: string;
  updated_at: string;
}

export interface HealthPanelData {
  node_id: string;
  tunnel_id: string | null;
  endpoint_ip: string | null;
  handshake_age_seconds: number | null;
  rx_bytes: number | null;
  tx_bytes: number | null;
  external_ipv4: string | null;
  tunnel_status: string | null;
}

export interface DiagnosticsBundle {
  generated_at: string;
  providers: ProviderRecord[];
  nodes: VpnNodeRecord[];
  tunnels: TunnelRecord[];
  redacted_notes: string[];
}

export interface WireGuardReadiness {
  installed: boolean;
  executable_found: boolean;
  service_available: boolean;
  version: string | null;
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
  monthly_usd: number;
  hourly_usd: number;
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
    throw new Error("UI is rendering on the server.");
  }

  const tauriWindow = window as TauriWindow;
  if (!tauriWindow.__TAURI_INTERNALS__) {
    throw new Error("Running in browser mode. Start with npm run tauri:dev to reach Rust.");
  }

  return invoke<T>(command, args);
}

export async function invokeHello(name: string): Promise<string> {
  return invokeCommand<string>("hello", { name });
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

export async function registerProviderToken(
  providerId: string,
  providerName: string,
  apiToken: string,
  providerSlug: string = "digitalocean",
): Promise<string> {
  const command = providerSlug === "docker"
    ? "register_docker_provider_token"
    : "register_digitalocean_provider_token";

  return invokeCommand<string>(command, {
    providerId,
    providerName,
    apiToken,
  });
}

export async function validateProviderToken(providerId: string): Promise<string> {
  return invokeCommand<string>("validate_provider_token", { providerId });
}

export async function listProviderPricing(providerId: string, region: string): Promise<ProviderPricingResponse> {
  return invokeCommand<ProviderPricingResponse>("list_provider_pricing", { providerId, region });
}

export async function listProviderRegions(providerId: string): Promise<string[]> {
  return invokeCommand<string[]>("list_provider_regions", { providerId });
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
  clientAddressCidr: string;
  dnsServer: string | null;
  allowedIps: string[];
}): Promise<string> {
  return invokeCommand<string>("create_and_activate_client_tunnel", {
    tunnelId: params.tunnelId,
    nodeId: params.nodeId,
    tunnelName: params.tunnelName,
    clientAddressCidr: params.clientAddressCidr,
    dnsServer: params.dnsServer,
    allowedIps: params.allowedIps,
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

export async function listTunnelsForNode(nodeId: string): Promise<TunnelRecord[]> {
  return invokeCommand<TunnelRecord[]>("list_tunnels_for_node", { nodeId });
}

export async function connectTunnel(nodeId: string, tunnelId: string): Promise<void> {
  await invokeCommand<void>("connect_tunnel", { nodeId, tunnelId });
}

export async function disconnectTunnel(nodeId: string, tunnelId: string): Promise<void> {
  await invokeCommand<void>("disconnect_tunnel", { nodeId, tunnelId });
}

export async function getHealthPanelData(nodeId: string, tunnelId?: string | null): Promise<HealthPanelData> {
  return invokeCommand<HealthPanelData>("get_health_panel_data", {
    nodeId,
    tunnelId: tunnelId ?? null,
  });
}

export async function getDiagnosticsBundle(): Promise<DiagnosticsBundle> {
  return invokeCommand<DiagnosticsBundle>("get_diagnostics_bundle");
}

export async function exportDiagnosticsBundle(): Promise<string> {
  return invokeCommand<string>("export_diagnostics_bundle");
}