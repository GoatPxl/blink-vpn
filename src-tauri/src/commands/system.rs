use tauri::State;

use crate::backend::Backend;

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