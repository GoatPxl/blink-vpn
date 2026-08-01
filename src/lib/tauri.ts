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

export async function registerProviderToken(providerId: string, providerName: string, apiToken: string): Promise<string> {
  return invokeCommand<string>("register_digitalocean_provider_token", {
    provider_id: providerId,
    provider_name: providerName,
    api_token: apiToken,
  });
}

export async function validateProviderToken(providerId: string): Promise<string> {
  return invokeCommand<string>("validate_provider_token", { provider_id: providerId });
}

export async function createVpnNode(params: {
  nodeId: string;
  providerId: string;
  nodeName: string;
  region: string;
}): Promise<string> {
  return invokeCommand<string>("create_vpn_node_to_ready", {
    node_id: params.nodeId,
    provider_id: params.providerId,
    node_name: params.nodeName,
    region: params.region,
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
    tunnel_id: params.tunnelId,
    node_id: params.nodeId,
    tunnel_name: params.tunnelName,
    client_address_cidr: params.clientAddressCidr,
    dns_server: params.dnsServer,
    allowed_ips: params.allowedIps,
  });
}

export async function destroyVpnNode(nodeId: string): Promise<void> {
  await invokeCommand<void>("destroy_vpn_node", { node_id: nodeId });
}

export async function listVpnNodes(): Promise<VpnNodeRecord[]> {
  return invokeCommand<VpnNodeRecord[]>("list_vpn_nodes");
}

export async function listTunnelsForNode(nodeId: string): Promise<TunnelRecord[]> {
  return invokeCommand<TunnelRecord[]>("list_tunnels_for_node", { node_id: nodeId });
}

export async function connectTunnel(nodeId: string, tunnelId: string): Promise<void> {
  await invokeCommand<void>("connect_tunnel", { node_id: nodeId, tunnel_id: tunnelId });
}

export async function disconnectTunnel(nodeId: string, tunnelId: string): Promise<void> {
  await invokeCommand<void>("disconnect_tunnel", { node_id: nodeId, tunnel_id: tunnelId });
}

export async function getHealthPanelData(nodeId: string, tunnelId?: string | null): Promise<HealthPanelData> {
  return invokeCommand<HealthPanelData>("get_health_panel_data", {
    node_id: nodeId,
    tunnel_id: tunnelId ?? null,
  });
}

export async function getDiagnosticsBundle(): Promise<DiagnosticsBundle> {
  return invokeCommand<DiagnosticsBundle>("get_diagnostics_bundle");
}