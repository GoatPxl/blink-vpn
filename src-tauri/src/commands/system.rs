use serde::Serialize;
use tauri::State;
use tauri::{AppHandle, Manager};

use crate::backend::Backend;
use crate::backend::logging;
use crate::backend::orchestrator::{
  ConnectTunnelRequest, CreateClientTunnelRequest, CreateVpnNodeRequest, DisconnectTunnelRequest,
};
use crate::backend::db::{TunnelRecord, VpnNodeRecord};

#[derive(Debug, Clone, Serialize)]
pub struct HealthPanelData {
  pub node_id: String,
  pub tunnel_id: Option<String>,
  pub endpoint_ip: Option<String>,
  pub handshake_age_seconds: Option<u64>,
  pub rx_bytes: Option<u64>,
  pub tx_bytes: Option<u64>,
  pub external_ipv4: Option<String>,
  pub tunnel_status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsBundleResponse {
  pub generated_at: String,
  pub providers: Vec<crate::backend::db::ProviderRecord>,
  pub nodes: Vec<VpnNodeRecord>,
  pub tunnels: Vec<TunnelRecord>,
  pub redacted_notes: Vec<String>,
}

#[tauri::command]
pub fn hello(name: Option<String>, backend: State<'_, Backend>) -> String {
  let caller = name.unwrap_or_else(|| "operator".to_string());
  let provider = backend.provider.active().provider_name();
  format!(
    "Hello, {caller}. Rust backend is reachable (provider adapter: {provider})."
  )
}

#[tauri::command]
pub fn validate_provider_token(provider_id: String, backend: State<'_, Backend>) -> Result<String, String> {
  let account = backend
    .provider
    .validate_provider_token(&backend.db, &backend.secure_storage, &provider_id)
    .map_err(|err| err.to_string())?;

  Ok(format!(
    "Provider token valid for account {} ({})",
    account.uuid, account.status
  ))
}

#[tauri::command]
pub fn provision_provider_droplet(
  provider_id: String,
  droplet_name: String,
  backend: State<'_, Backend>,
) -> Result<String, String> {
  let droplet = backend
    .orchestrator
    .provision_droplet_for_provider(
      &backend.provider,
      &backend.bootstrap,
      &backend.db,
      &backend.secure_storage,
      &provider_id,
      &droplet_name,
    )
    .map_err(|err| {
      let user_error = err.to_user_facing();
      format!("{} Next: {}", user_error.message, user_error.next_action)
    })?;

  Ok(format!(
    "Droplet provisioned: id={}, public_ipv4={}",
    droplet.droplet_id, droplet.public_ipv4
  ))
}

#[tauri::command]
pub fn delete_provider_droplet(
  provider_id: String,
  droplet_id: u64,
  backend: State<'_, Backend>,
) -> Result<(), String> {
  backend
    .orchestrator
    .destroy_droplet_for_provider(
      &backend.provider,
      &backend.db,
      &backend.secure_storage,
      &provider_id,
      droplet_id,
    )
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn register_digitalocean_provider_token(
  provider_id: String,
  provider_name: String,
  api_token: String,
  backend: State<'_, Backend>,
) -> Result<String, String> {
  let provider = backend
    .provider
    .upsert_digitalocean_provider_with_token(
      &backend.db,
      &backend.secure_storage,
      &provider_id,
      &provider_name,
      &api_token,
    )
    .map_err(|err| err.to_string())?;

  let token_ref = provider
    .token_ref
    .unwrap_or_else(|| "<missing-token-ref>".to_string());

  Ok(format!(
    "Provider registered with token_ref: {}",
    token_ref
  ))
}

#[tauri::command]
pub fn list_vpn_nodes(backend: State<'_, Backend>) -> Result<Vec<VpnNodeRecord>, String> {
  backend
    .orchestrator
    .list_nodes(&backend.db)
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn list_tunnels_for_node(
  node_id: String,
  backend: State<'_, Backend>,
) -> Result<Vec<TunnelRecord>, String> {
  backend
    .orchestrator
    .list_tunnels_for_node(&backend.db, &node_id)
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn destroy_vpn_node(node_id: String, backend: State<'_, Backend>) -> Result<(), String> {
  backend
    .orchestrator
    .destroy_vpn_node(
      &backend.provider,
      &backend.wireguard,
      &backend.db,
      &backend.secure_storage,
      &node_id,
    )
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn create_vpn_node_to_ready(
  node_id: String,
  provider_id: String,
  node_name: String,
  region: String,
  backend: State<'_, Backend>,
) -> Result<String, String> {
  let result = backend
    .orchestrator
    .create_vpn_node_to_ready(
      &backend.provider,
      &backend.bootstrap,
      &backend.db,
      &backend.secure_storage,
      &CreateVpnNodeRequest {
        node_id,
        provider_id,
        node_name,
        region,
      },
    )
    .map_err(|err| err.to_string())?;

  Ok(format!(
    "Create node complete: node_id={}, droplet_id={}, endpoint_ip={}, listen_port={}, correlation_id={}, idempotent_reuse={}",
    result.node_id,
    result.droplet_id,
    result.server_endpoint_ip,
    result.listen_port,
    result.correlation_id,
    result.idempotent_reuse
  ))
}

#[tauri::command]
pub fn create_and_activate_client_tunnel(
  tunnel_id: String,
  node_id: String,
  tunnel_name: String,
  client_address_cidr: String,
  dns_server: Option<String>,
  allowed_ips: Vec<String>,
  backend: State<'_, Backend>,
) -> Result<String, String> {
  let result = backend
    .orchestrator
    .create_and_activate_client_tunnel(
      &backend.wireguard,
      &backend.db,
      &CreateClientTunnelRequest {
        tunnel_id,
        node_id,
        tunnel_name,
        client_address_cidr,
        dns_server,
        allowed_ips,
      },
    )
    .map_err(|err| err.to_string())?;

  Ok(format!(
    "Tunnel connected: tunnel_id={}, tunnel_name={}, client_public_key={}",
    result.tunnel_id, result.tunnel_name, result.client_public_key
  ))
}

#[tauri::command]
pub fn activate_client_tunnel(tunnel_id: String, backend: State<'_, Backend>) -> Result<(), String> {
  backend
    .orchestrator
    .activate_client_tunnel(&backend.wireguard, &backend.db, &tunnel_id)
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn deactivate_client_tunnel(
  tunnel_id: String,
  backend: State<'_, Backend>,
) -> Result<(), String> {
  backend
    .orchestrator
    .deactivate_client_tunnel(&backend.wireguard, &backend.db, &tunnel_id)
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn read_client_tunnel_status(
  tunnel_id: String,
  backend: State<'_, Backend>,
) -> Result<String, String> {
  let status = backend
    .orchestrator
    .read_client_tunnel_status(&backend.wireguard, &backend.db, &tunnel_id)
    .map_err(|err| err.to_string())?;

  Ok(format!(
    "Tunnel status: name={}, state={:?}, handshake_age_seconds={:?}, rx_bytes={:?}, tx_bytes={:?}",
    status.tunnel_name,
    status.state,
    status.last_handshake_age_seconds,
    status.rx_bytes,
    status.tx_bytes
  ))
}

#[tauri::command]
pub fn connect_tunnel(
  node_id: String,
  tunnel_id: String,
  backend: State<'_, Backend>,
) -> Result<(), String> {
  backend
    .orchestrator
    .connect_tunnel(
      &backend.wireguard,
      &backend.db,
      &ConnectTunnelRequest { node_id, tunnel_id },
    )
    .map_err(|err| {
      let user_error = err.to_user_facing();
      format!("{} Next: {}", user_error.message, user_error.next_action)
    })
}

  #[tauri::command]
  pub fn get_health_panel_data(
    node_id: String,
    tunnel_id: Option<String>,
    backend: State<'_, Backend>,
  ) -> Result<HealthPanelData, String> {
    let summary = backend
      .orchestrator
      .get_health_summary(&backend.wireguard, &backend.db, &node_id, tunnel_id.as_deref())
      .map_err(|err| err.to_string())?;

    Ok(HealthPanelData {
      node_id: summary.node_id,
      tunnel_id: summary.tunnel_id,
      endpoint_ip: summary.endpoint_ip,
      handshake_age_seconds: summary.handshake_age_seconds,
      rx_bytes: summary.rx_bytes,
      tx_bytes: summary.tx_bytes,
      external_ipv4: summary.external_ipv4,
      tunnel_status: summary.tunnel_status.map(|status| status.to_string()),
    })
  }

  #[tauri::command]
  pub fn get_diagnostics_bundle(backend: State<'_, Backend>) -> Result<DiagnosticsBundleResponse, String> {
    let bundle = backend
      .orchestrator
      .build_diagnostics_bundle(&backend.db)
      .map_err(|err| err.to_string())?;

    Ok(DiagnosticsBundleResponse {
      generated_at: bundle.generated_at,
      providers: bundle.providers,
      nodes: bundle.nodes,
      tunnels: bundle.tunnels,
      redacted_notes: bundle.redacted_notes,
    })
  }

#[tauri::command]
pub fn export_diagnostics_bundle(
  app: AppHandle,
  backend: State<'_, Backend>,
) -> Result<String, String> {
  let bundle = backend
    .orchestrator
    .build_diagnostics_bundle(&backend.db)
    .map_err(|err| err.to_string())?;

  let bundle_json = serde_json::to_string_pretty(&bundle).map_err(|err| err.to_string())?;
  let sanitized_json = logging::redact_sensitive_text(&bundle_json);

  let diagnostics_dir = app
    .path()
    .app_data_dir()
    .map_err(|err| err.to_string())?
    .join("diagnostics");

  std::fs::create_dir_all(&diagnostics_dir).map_err(|err| err.to_string())?;

  let file_path = diagnostics_dir.join(format!("diagnostics-{}.json", bundle.generated_at));
  std::fs::write(&file_path, sanitized_json).map_err(|err| err.to_string())?;

  Ok(file_path.display().to_string())
}

#[tauri::command]
pub fn disconnect_tunnel(
  node_id: String,
  tunnel_id: String,
  backend: State<'_, Backend>,
) -> Result<(), String> {
  backend
    .orchestrator
    .disconnect_tunnel(
      &backend.wireguard,
      &backend.db,
      &DisconnectTunnelRequest { node_id, tunnel_id },
    )
    .map_err(|err| {
      let user_error = err.to_user_facing();
      format!("{} Next: {}", user_error.message, user_error.next_action)
    })
}