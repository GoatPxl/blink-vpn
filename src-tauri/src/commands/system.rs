use tauri::State;

use crate::backend::Backend;
use crate::backend::orchestrator::{CreateClientTunnelRequest, CreateVpnNodeRequest};

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
    .map_err(|err| err.to_string())?;

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
    .create_digitalocean_provider_with_token(
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