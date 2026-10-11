use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tauri::State;
use tauri::{AppHandle, Manager};

use crate::backend::Backend;
use crate::backend::app_preferences::{self, AppPreferences, CloseToTray};
use crate::backend::process::no_window_command;
use crate::commands::error::{CommandError, CommandErrorKind};
use crate::backend::db::{Database, ProviderRecord, ProviderStatus};
use crate::backend::logging;
use crate::backend::provider::{
  CredentialField, ProviderCredential, ProviderPlanPricing, ProviderRegion,
};
use crate::backend::orchestrator::{
  ConnectTunnelRequest, CreateClientTunnelRequest, CreateVpnNodeRequest, DisconnectTunnelRequest,
  ForceCleanupResult, KillSwitchState, SavedLocationView, TunnelPhase, TunnelSettings,
  UnexpectedDrop,
};
use crate::backend::db::{NodePeerRecord, TunnelRecord, VpnNodeRecord};
use crate::backend::secure_storage::SecureStorage;
use crate::backend::wireguard::{TunnelServiceState, WireGuardReadiness};

/// Runs a command body on the blocking thread pool.
///
/// Every command in this file does blocking work — subprocess spawns, `thread::sleep`, a modal UAC
/// prompt, SQLite, Windows Credential Manager, and `reqwest::blocking` HTTP. None of it belongs on
/// the UI thread, which is where Tauri runs a plain `#[tauri::command]` and which is why the window
/// went "Not Responding" during a connect.
///
/// It does not belong on an async worker either. `#[tauri::command(async)]` schedules the body on
/// the tokio *async* runtime, and `reqwest::blocking` builds its own runtime internally — dropping
/// one inside an async context panics with "Cannot drop a runtime in a context where blocking is
/// not allowed". That panic killed the IPC channel, so every command in the app failed at once.
///
/// `spawn_blocking` is the pool meant for exactly this: the UI thread stays free, and blocking
/// calls remain legal.
async fn run_blocking<T, F>(work: F) -> Result<T, CommandError>
where
  F: FnOnce() -> Result<T, CommandError> + Send + 'static,
  T: Send + 'static,
{
  match tauri::async_runtime::spawn_blocking(work).await {
    Ok(result) => result,
    Err(source) => Err(CommandError::new(
      CommandErrorKind::Unknown,
      format!("A background operation did not finish: {source}"),
      "Retry the action. If it keeps failing, restart the app and export diagnostics.",
    )),
  }
}

const BUILTIN_DIGITALOCEAN_PROVIDER_ID: &str = "provider-1";
const BUILTIN_DIGITALOCEAN_PROVIDER_NAME: &str = "DigitalOcean";
const BUILTIN_DIGITALOCEAN_PROVIDER_SLUG: &str = "digitalocean";
const BUILTIN_EC2_PROVIDER_ID: &str = "provider-aws-ec2";
const BUILTIN_EC2_PROVIDER_NAME: &str = "Amazon EC2";
const BUILTIN_EC2_PROVIDER_SLUG: &str = "aws-ec2";
const BUILTIN_LINODE_PROVIDER_ID: &str = "provider-linode";
const BUILTIN_LINODE_PROVIDER_NAME: &str = "Linode";
const BUILTIN_LINODE_PROVIDER_SLUG: &str = "linode";
const BUILTIN_GCE_PROVIDER_ID: &str = "provider-gce";
const BUILTIN_GCE_PROVIDER_NAME: &str = "Google Compute Engine";
const BUILTIN_GCE_PROVIDER_SLUG: &str = "gce";
const BUILTIN_OPENSTACK_PROVIDER_ID: &str = "provider-openstack";
const BUILTIN_OPENSTACK_PROVIDER_NAME: &str = "OpenStack";
const BUILTIN_OPENSTACK_PROVIDER_SLUG: &str = "openstack";
const BUILTIN_DOCKER_PROVIDER_ID: &str = "provider-docker";
const BUILTIN_DOCKER_PROVIDER_NAME: &str = "Docker";
const BUILTIN_DOCKER_PROVIDER_SLUG: &str = "docker";
/// Subdirectory of the app data directory that exported bundles are written to.
const DIAGNOSTICS_DIR_NAME: &str = "diagnostics";
/// Subdirectory that exported device configurations are written to.
const PEER_CONFIG_DIR_NAME: &str = "peer-configs";

#[derive(Debug, Clone, Serialize)]
pub struct HealthPanelData {
  pub node_id: String,
  pub tunnel_id: Option<String>,
  pub endpoint_ip: Option<String>,
  pub handshake_age_seconds: Option<u64>,
  pub rx_bytes: Option<u64>,
  pub tx_bytes: Option<u64>,
  /// The address the internet currently sees, measured in every connection state. `None` means
  /// the probe failed. Callers must not present `endpoint_ip` as though it were this value.
  pub external_ipv4: Option<String>,
  /// Present only when the machine has IPv6 egress. This tunnel is IPv4-only, so a value here
  /// while connected means IPv6 traffic is bypassing it.
  pub external_ipv6: Option<String>,
  pub tunnel_status: Option<String>,
  /// Whether the tunnel actually carries traffic — the value the UI turns green on.
  ///
  /// `tunnel_status` above tracks the Windows service, which reaches "connected" the moment it
  /// starts. The tunnel captures the default route at that instant, so between then and the first
  /// handshake the user has no internet while the old UI showed success.
  pub phase: TunnelPhase,
  /// `"peer"` or `"adapter"` — which tier produced the byte counters. Consumers differencing
  /// samples must reset their baseline when this changes.
  pub metrics_source: Option<String>,
  /// Session start time, or `None` when not connected or the start time is unknown.
  pub connected_at: Option<String>,
  /// Whether untunneled traffic is currently blocked, and why. Provided by WireGuard for Windows,
  /// not by this app — see `KillSwitchState`.
  pub kill_switch: KillSwitchState,
  /// Set when the tunnel stopped without the app asking; traffic is unprotected until reconnect.
  pub unexpected_drop: Option<UnexpectedDrop>,
}

/// The connect-progress reading, polled once a second while a connect is in flight.
///
/// Carries the evidence next to the verdict so the UI can say *why* it is still waiting — elapsed
/// seconds and whether anything has come back through the tunnel — instead of showing an
/// unexplained spinner while the user's internet is down.
#[derive(Debug, Clone, Serialize)]
pub struct TunnelPhaseSnapshot {
  pub tunnel_id: String,
  pub tunnel_name: String,
  pub phase: TunnelPhase,
  /// The raw `sc query` verdict behind the phase, for display and diagnostics.
  pub service_state: TunnelServiceState,
  /// Seconds since this app asked the service to start; `null` when it was already running.
  pub elapsed_seconds: Option<u64>,
  /// Decrypted inbound octets. `> 0` is the unelevated proof that a handshake completed.
  pub rx_bytes: Option<u64>,
  pub tx_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsBundleResponse {
  pub generated_at: String,
  pub providers: Vec<crate::backend::db::ProviderRecord>,
  pub nodes: Vec<VpnNodeRecord>,
  pub tunnels: Vec<TunnelRecord>,
  pub activity: Vec<crate::backend::db::ActivityEvent>,
  pub redacted_notes: Vec<String>,
  pub wireguard_debug: crate::backend::wireguard::WireGuardDebugSnapshot,
}

/// A folder or file the user may need to open when reporting a problem.
///
/// The path is resolved in Rust and handed out for display only. `reveal_app_location` takes the
/// `key` back, never a path, so the frontend cannot ask the shell to open somewhere else.
#[derive(Debug, Clone, Serialize)]
pub struct AppLocation {
  pub key: String,
  pub label: String,
  pub description: String,
  pub path: String,
  /// False when the file or folder has not been created yet — an export that never ran, a log
  /// directory a fresh install has not written to.
  pub exists: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WireGuardReadinessResponse {
  pub installed: bool,
  pub executable_found: bool,
  pub service_available: bool,
  /// Installed WireGuard version from `wg --version`, or `None` when it could not be read.
  pub version: Option<String>,
  /// This app's own version, so the health page reports a real build rather than a made-up one.
  pub app_version: String,
  pub message: String,
  pub action_guidance: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WireGuardInstallResult {
  pub installed: bool,
  pub prerequisites_configured: bool,
  pub installer_path: String,
  pub message: String,
  pub next_action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DockerReadinessResponse {
  pub installed: bool,
  pub daemon_reachable: bool,
  pub cli_version: Option<String>,
  pub daemon_version: Option<String>,
  pub message: String,
  pub action_guidance: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderPricingResponse {
  pub provider_id: String,
  pub region: String,
  pub plans: Vec<ProviderPlanPricing>,
}

fn ensure_provider_present(
  db: &Database,
  provider_id: &str,
  provider_name: &str,
  provider_slug: &str,
) -> Result<(), CommandError> {
  if db
    .get_provider(provider_id)
    .map_err(CommandError::from)?
    .is_none()
  {
    db.create_provider(provider_id, provider_name, provider_slug, None)
      .map_err(CommandError::from)?;
  }

  Ok(())
}

fn ensure_builtin_providers(db: &Database, docker_dev_enabled: bool) -> Result<(), CommandError> {
  ensure_provider_present(
    db,
    BUILTIN_DIGITALOCEAN_PROVIDER_ID,
    BUILTIN_DIGITALOCEAN_PROVIDER_NAME,
    BUILTIN_DIGITALOCEAN_PROVIDER_SLUG,
  )?;

  ensure_provider_present(
    db,
    BUILTIN_EC2_PROVIDER_ID,
    BUILTIN_EC2_PROVIDER_NAME,
    BUILTIN_EC2_PROVIDER_SLUG,
  )?;

  ensure_provider_present(
    db,
    BUILTIN_LINODE_PROVIDER_ID,
    BUILTIN_LINODE_PROVIDER_NAME,
    BUILTIN_LINODE_PROVIDER_SLUG,
  )?;

  ensure_provider_present(
    db,
    BUILTIN_GCE_PROVIDER_ID,
    BUILTIN_GCE_PROVIDER_NAME,
    BUILTIN_GCE_PROVIDER_SLUG,
  )?;

  ensure_provider_present(
    db,
    BUILTIN_OPENSTACK_PROVIDER_ID,
    BUILTIN_OPENSTACK_PROVIDER_NAME,
    BUILTIN_OPENSTACK_PROVIDER_SLUG,
  )?;

  if docker_dev_enabled {
    ensure_provider_present(
      db,
      BUILTIN_DOCKER_PROVIDER_ID,
      BUILTIN_DOCKER_PROVIDER_NAME,
      BUILTIN_DOCKER_PROVIDER_SLUG,
    )?;
  }

  Ok(())
}

fn clear_provider_credential(
  db: &Database,
  secure_storage: &SecureStorage,
  provider_id: &str,
) -> Result<(), CommandError> {
  let provider = db
    .get_provider(provider_id)
    .map_err(CommandError::from)?
    .ok_or_else(|| CommandError::new(
      CommandErrorKind::MissingResource,
      format!("Provider '{}' was not found.", provider_id),
      "Refresh the dashboard so the provider list reloads, then retry.",
    ))?;

  let provider_nodes = db
    .list_vpn_nodes()
    .map_err(CommandError::from)?
    .into_iter()
    .filter(|node| node.provider_id == provider_id)
    .count();

  if provider_nodes > 0 {
    return Err(CommandError::new(
      CommandErrorKind::InvalidState,
      format!(
        "Cannot delete provider key while {} node{} are still attached.",
        provider_nodes,
        if provider_nodes == 1 { "" } else { "s" }
      ),
      "Destroy the dependent nodes first, then delete the key.",
    ));
  }

  if let Some(token_ref) = provider.token_ref.as_deref() {
    secure_storage
      .delete_provider_credential(token_ref)
      .map_err(CommandError::from)?;
  }

  db.update_provider_token_ref(provider_id, None)
    .map_err(CommandError::from)?;

  // The verdict belonged to the key that just went away.
  db.record_provider_validation(provider_id, None)
    .map_err(CommandError::from)?;

  if provider.status == ProviderStatus::Suspended {
    db.resume_provider(provider_id).map_err(CommandError::from)?;
  }

  Ok(())
}

#[tauri::command]
pub async fn validate_provider_token(provider_id: String, backend: State<'_, Backend>) -> Result<String, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  let identity = backend
    .provider
    .validate_provider_token(&backend.db, &backend.secure_storage, &provider_id)
    .map_err(CommandError::from)?;

  Ok(format!("Credential valid for {}", identity.describe()))
  })
  .await
}

#[tauri::command]
pub async fn provision_provider_node(
  provider_id: String,
  node_name: String,
  backend: State<'_, Backend>,
) -> Result<String, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  let provisioned = backend
    .orchestrator
    .provision_node_for_provider(
      &backend.provider,
      &backend.bootstrap,
      &backend.db,
      &backend.secure_storage,
      &provider_id,
      &node_name,
    )
    .map_err(CommandError::from)?;

  Ok(format!(
    "Node provisioned: id={}, public_ipv4={}",
    provisioned.node_id, provisioned.public_ipv4
  ))
  })
  .await
}

#[tauri::command]
pub async fn delete_provider_node(
  provider_id: String,
  node_id: String,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .destroy_node_for_provider(
      &backend.provider,
      &backend.db,
      &backend.secure_storage,
      &provider_id,
      &node_id,
    )
    .map_err(CommandError::from)
  })
  .await
}

/// Where a provider runs, from its adapter's curated table, with no credential and no network call.
/// For showing a provider's regions on the map before the user has a key for it.
#[tauri::command]
pub async fn list_known_provider_regions(
  provider_id: String,
  backend: State<'_, Backend>,
) -> Result<Vec<ProviderRegion>, CommandError> {
  let backend = backend.inner().clone();
  run_blocking(move || {
    backend
      .provider
      .known_regions_for_provider(&backend.db, &provider_id)
      .map_err(CommandError::from)
  })
  .await
}

/// The credential inputs a provider needs, so the UI can render them without knowing the provider.
#[tauri::command]
pub async fn list_provider_credential_fields(
  provider_id: String,
  backend: State<'_, Backend>,
) -> Result<Vec<CredentialField>, CommandError> {
  let backend = backend.inner().clone();
  run_blocking(move || {
    backend
      .provider
      .credential_fields_for_provider(&backend.db, &provider_id)
      .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn register_provider_credential(
  provider_id: String,
  provider_name: String,
  credentials: HashMap<String, String>,
  backend: State<'_, Backend>,
) -> Result<String, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  // The slug comes from the provider record rather than from the caller, so the UI no longer
  // picks a command name by slug and the backend stays the single source of truth for which
  // adapter a provider uses.
  let slug = backend
    .db
    .get_provider(&provider_id)
    .map_err(CommandError::from)?
    .map(|provider| provider.slug)
    .ok_or_else(|| CommandError::new(
      CommandErrorKind::MissingResource,
      format!("Provider '{}' was not found.", provider_id),
      "Refresh the dashboard so the provider list reloads, then retry.",
    ))?;

  // Every field the adapter declares must be present and non-empty. Catching it here means a
  // half-filled credential is rejected before it is written, rather than surfacing later as an
  // authentication failure with no indication of which field was missing.
  let declared = backend
    .provider
    .credential_fields_for_provider(&backend.db, &provider_id)
    .map_err(CommandError::from)?;

  for field in &declared {
    let supplied = credentials.get(&field.key).map(|value| value.trim()).unwrap_or("");
    if supplied.is_empty() {
      return Err(CommandError::new(
        CommandErrorKind::InvalidInput,
        format!("'{}' is required for this provider.", field.label),
        "Fill in every credential field, then save again.",
      ));
    }
  }

  let credential = ProviderCredential::from_fields(
    credentials
      .into_iter()
      .map(|(key, value)| (key, value.trim().to_string())),
  );

  let provider = backend
    .provider
    .upsert_provider_credential_for_slug(
      &backend.db,
      &backend.secure_storage,
      &provider_id,
      &provider_name,
      &slug,
      &credential,
    )
    .map_err(CommandError::from)?;

  let token_ref = provider
    .token_ref
    .unwrap_or_else(|| "<missing-token-ref>".to_string());

  Ok(format!("Provider registered with token_ref: {}", token_ref))
  })
  .await
}

#[tauri::command]
pub async fn list_provider_pricing(
  provider_id: String,
  region: String,
  backend: State<'_, Backend>,
) -> Result<ProviderPricingResponse, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  let plans = backend
    .provider
    .list_provider_plan_pricing(&backend.db, &backend.secure_storage, &provider_id, &region)
    .map_err(CommandError::from)?;

  Ok(ProviderPricingResponse {
    provider_id,
    region,
    plans,
  })
  })
  .await
}

#[tauri::command]
pub async fn list_provider_regions(
  provider_id: String,
  backend: State<'_, Backend>,
) -> Result<Vec<ProviderRegion>, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .provider
    .list_provider_regions(&backend.db, &backend.secure_storage, &provider_id)
    .map_err(CommandError::from)
  })
  .await
}

/// The node the app connects to by default, or `null` when the user has more than one node and
/// has not chosen between them.
#[tauri::command]
pub async fn get_current_node_id(backend: State<'_, Backend>) -> Result<Option<String>, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .get_current_node_id(&backend.db)
    .map_err(CommandError::from)
  })
  .await
}

/// Client tunnel parameters. Applied when a tunnel next starts; a running tunnel keeps the config
/// it was started with.
#[tauri::command]
pub async fn get_tunnel_settings(backend: State<'_, Backend>) -> Result<TunnelSettings, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .get_tunnel_settings(&backend.db)
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn set_tunnel_settings(
  dns_server: Option<String>,
  keepalive_seconds: u16,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .set_tunnel_settings(&backend.db, &TunnelSettings { dns_server, keepalive_seconds })
    .map_err(CommandError::from)
  })
  .await
}

/// App preferences: window behaviour, device names for new servers, and persisted UI state.
#[tauri::command]
pub async fn get_app_preferences(backend: State<'_, Backend>) -> Result<AppPreferences, CommandError> {
  let backend = backend.inner().clone();
  run_blocking(move || app_preferences::get(&backend.db).map_err(CommandError::from)).await
}

/// Stores the preferences and returns them as stored, with device names trimmed.
///
/// Takes the whole struct rather than one argument per field, so adding a preference does not
/// change this command's signature.
#[tauri::command]
pub async fn set_app_preferences(
  preferences: AppPreferences,
  app: AppHandle,
  backend: State<'_, Backend>,
) -> Result<AppPreferences, CommandError> {
  let backend = backend.inner().clone();
  run_blocking(move || {
    let stored = app_preferences::set(&backend.db, &preferences)
      .map_err(CommandError::from)?;
    // Updated only after the write lands, so the window never behaves differently from what a
    // restart would load.
    app.state::<CloseToTray>().set(stored.close_to_tray);
    Ok(stored)
  })
  .await
}

#[tauri::command]
pub async fn set_current_node_id(
  node_id: Option<String>,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .set_current_node_id(&backend.db, node_id.as_deref())
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn list_vpn_nodes(backend: State<'_, Backend>) -> Result<Vec<VpnNodeRecord>, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .list_nodes(&backend.provider, &backend.db, &backend.secure_storage)
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn list_providers(backend: State<'_, Backend>) -> Result<Vec<ProviderRecord>, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  ensure_builtin_providers(&backend.db, backend.feature_flags.docker_dev)?;
  backend.db.list_providers().map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn suspend_provider(provider_id: String, backend: State<'_, Backend>) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  let provider = backend
    .db
    .get_provider(&provider_id)
    .map_err(CommandError::from)?
    .ok_or_else(|| CommandError::new(
      CommandErrorKind::MissingResource,
      format!("Provider '{}' was not found.", provider_id),
      "Refresh the dashboard so the provider list reloads, then retry.",
    ))?;

  if provider.status == crate::backend::db::ProviderStatus::Suspended {
    return Ok(());
  }

  backend
    .db
    .suspend_provider(&provider_id)
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn resume_provider(provider_id: String, backend: State<'_, Backend>) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  let provider = backend
    .db
    .get_provider(&provider_id)
    .map_err(CommandError::from)?
    .ok_or_else(|| CommandError::new(
      CommandErrorKind::MissingResource,
      format!("Provider '{}' was not found.", provider_id),
      "Refresh the dashboard so the provider list reloads, then retry.",
    ))?;

  if provider.status == crate::backend::db::ProviderStatus::Active {
    return Ok(());
  }

  backend
    .db
    .resume_provider(&provider_id)
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn delete_provider(provider_id: String, backend: State<'_, Backend>) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  clear_provider_credential(&backend.db, &backend.secure_storage, &provider_id)
  })
  .await
}

/// Servers the user can turn back on, with whether each is on right now.
#[tauri::command]
pub async fn list_saved_locations(
  backend: State<'_, Backend>,
) -> Result<Vec<SavedLocationView>, CommandError> {
  let backend = backend.inner().clone();
  run_blocking(move || {
    backend
      .orchestrator
      .list_saved_locations(&backend.provider, &backend.db)
      .map_err(CommandError::from)
  })
  .await
}

/// Provisions a saved location again. Blocks until the server is ready, like any create; the UI
/// follows progress by polling `node_id`.
#[tauri::command]
pub async fn turn_on_saved_location(
  saved_location_id: String,
  node_id: String,
  backend: State<'_, Backend>,
) -> Result<String, CommandError> {
  let backend = backend.inner().clone();
  run_blocking(move || {
    let result = backend
      .orchestrator
      .turn_on_saved_location(
        &backend.provider,
        &backend.bootstrap,
        &backend.db,
        &backend.secure_storage,
        &saved_location_id,
        &node_id,
      )
      .map_err(CommandError::from)?;
    Ok(result.node_id)
  })
  .await
}

/// Deletes a saved location's server to stop billing. The location stays.
#[tauri::command]
pub async fn turn_off_saved_location(
  saved_location_id: String,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  let backend = backend.inner().clone();
  run_blocking(move || {
    backend
      .orchestrator
      .turn_off_saved_location(
        &backend.provider,
        &backend.wireguard,
        &backend.db,
        &backend.secure_storage,
        &saved_location_id,
      )
      .map_err(CommandError::from)
  })
  .await
}

/// Removes a saved location. Refused while it is on.
#[tauri::command]
pub async fn forget_saved_location(
  saved_location_id: String,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  let backend = backend.inner().clone();
  run_blocking(move || {
    backend
      .orchestrator
      .forget_saved_location(&backend.db, &saved_location_id)
      .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn list_tunnels_for_node(
  node_id: String,
  backend: State<'_, Backend>,
) -> Result<Vec<TunnelRecord>, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .list_tunnels_for_node(&backend.db, &node_id)
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn destroy_vpn_node(node_id: String, backend: State<'_, Backend>) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .destroy_vpn_node(
      &backend.provider,
      &backend.wireguard,
      &backend.db,
      &backend.secure_storage,
      &node_id,
    )
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn force_cleanup_vpn_node(
  node_id: String,
  backend: State<'_, Backend>,
) -> Result<ForceCleanupResult, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .force_cleanup_vpn_node(
      &backend.provider,
      &backend.wireguard,
      &backend.db,
      &backend.secure_storage,
      &node_id,
    )
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn create_vpn_node_to_ready(
  node_id: String,
  provider_id: String,
  node_name: String,
  region: String,
  plan_key: String,
  backend: State<'_, Backend>,
) -> Result<String, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
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
        plan_key,
      },
    )
    .map_err(CommandError::from)?;

  Ok(format!(
    "Create node complete: node_id={}, external_ref={}, endpoint_ip={}, listen_port={}, correlation_id={}, idempotent_reuse={}",
    result.node_id,
    result.external_ref,
    result.server_endpoint_ip,
    result.listen_port,
    result.correlation_id,
    result.idempotent_reuse
  ))
  })
  .await
}

#[tauri::command]
pub async fn create_and_activate_client_tunnel(
  tunnel_id: String,
  node_id: String,
  tunnel_name: String,
  dns_server: Option<String>,
  allowed_ips: Vec<String>,
  backend: State<'_, Backend>,
) -> Result<String, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  let result = backend
    .orchestrator
    .create_and_activate_client_tunnel(
      &backend.wireguard,
      &backend.db,
      &backend.secure_storage,
      &CreateClientTunnelRequest {
        tunnel_id,
        node_id,
        tunnel_name,
        dns_server,
        allowed_ips,
      },
    )
    .map_err(CommandError::from)?;

  Ok(format!(
    "Tunnel configured: tunnel_id={}, tunnel_name={}, client_public_key={}",
    result.tunnel_id, result.tunnel_name, result.client_public_key
  ))
  })
  .await
}

#[tauri::command]
pub async fn activate_client_tunnel(tunnel_id: String, backend: State<'_, Backend>) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .activate_client_tunnel(&backend.wireguard, &backend.db, &backend.secure_storage, &tunnel_id)
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn deactivate_client_tunnel(
  tunnel_id: String,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .deactivate_client_tunnel(&backend.wireguard, &backend.db, &backend.secure_storage, &tunnel_id)
    .map_err(CommandError::from)
  })
  .await
}

/// How far along a connect is. Cheap enough to poll once a second while one is in flight.
///
/// Replaces `read_client_tunnel_status`, which returned a `format!`ed sentence no caller could
/// parse and which nothing on the frontend ever invoked.
#[tauri::command]
pub async fn get_tunnel_phase(
  node_id: String,
  tunnel_id: String,
  backend: State<'_, Backend>,
) -> Result<TunnelPhaseSnapshot, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  let reading = backend
    .orchestrator
    .get_tunnel_phase(&backend.wireguard, &backend.db, &node_id, &tunnel_id)
    .map_err(CommandError::from)?;

  Ok(TunnelPhaseSnapshot {
    tunnel_id: reading.tunnel_id,
    tunnel_name: reading.tunnel_name,
    phase: reading.phase,
    service_state: reading.service_state,
    elapsed_seconds: reading.elapsed_seconds,
    rx_bytes: reading.rx_bytes,
    tx_bytes: reading.tx_bytes,
  })
  })
  .await
}

#[tauri::command]
pub async fn connect_tunnel(
  node_id: String,
  tunnel_id: String,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .connect_tunnel(
      &backend.wireguard,
      &backend.db,
      &backend.secure_storage,
      &ConnectTunnelRequest { node_id, tunnel_id },
    )
    .map_err(CommandError::from)
  })
  .await
}

  #[tauri::command]
  pub async fn get_health_panel_data(
    node_id: String,
    tunnel_id: Option<String>,
    force_ip_probe: Option<bool>,
    backend: State<'_, Backend>,
  ) -> Result<HealthPanelData, CommandError> {
    // Cloned out of the managed state so the body can move onto a blocking thread.
    // Every field is an Arc or a PathBuf, so this is cheap.
    let backend = backend.inner().clone();
    run_blocking(move || {
    let summary = backend
      .orchestrator
      .get_health_summary(
        &backend.wireguard,
        &backend.db,
        &backend.secure_storage,
        &node_id,
        tunnel_id.as_deref(),
        force_ip_probe.unwrap_or(false),
      )
      .map_err(CommandError::from)?;

    Ok(HealthPanelData {
      node_id: summary.node_id,
      tunnel_id: summary.tunnel_id,
      endpoint_ip: summary.endpoint_ip,
      handshake_age_seconds: summary.handshake_age_seconds,
      rx_bytes: summary.rx_bytes,
      tx_bytes: summary.tx_bytes,
      external_ipv4: summary.external_ipv4,
      external_ipv6: summary.external_ipv6,
      tunnel_status: summary.tunnel_status.map(|status| status.to_string()),
      phase: summary.phase,
      metrics_source: summary.metrics_source.map(|source| source.as_str().to_string()),
      connected_at: summary.connected_at,
      kill_switch: summary.kill_switch,
      unexpected_drop: summary.unexpected_drop,
    })
      })
    .await
  }

  #[tauri::command]
  pub async fn get_diagnostics_bundle(backend: State<'_, Backend>) -> Result<DiagnosticsBundleResponse, CommandError> {
    // Cloned out of the managed state so the body can move onto a blocking thread.
    // Every field is an Arc or a PathBuf, so this is cheap.
    let backend = backend.inner().clone();
    run_blocking(move || {
    ensure_builtin_providers(&backend.db, backend.feature_flags.docker_dev)?;
    let bundle = backend
      .orchestrator
      .build_diagnostics_bundle(&backend.wireguard, &backend.db)
      .map_err(CommandError::from)?;

    Ok(DiagnosticsBundleResponse {
      generated_at: bundle.generated_at,
      providers: bundle.providers,
      nodes: bundle.nodes,
      tunnels: bundle.tunnels,
      activity: bundle.activity,
      redacted_notes: bundle.redacted_notes,
      wireguard_debug: bundle.wireguard_debug,
    })
      })
    .await
  }

/// A device config plus the QR symbol that encodes it.
///
/// The config carries its `PrivateKey`, which is what makes it importable at all. That is
/// sanctioned here — WireGuard key material is not a secret in this app (AGENTS.md §6) — but it
/// still must not be written into the activity log or a diagnostics bundle, where it would be
/// bulk noise in a history meant to be read.
#[derive(Debug, Clone, Serialize)]
pub struct PeerConfigPayload {
  pub config_text: String,
  /// Width and height of the QR symbol, in modules.
  pub qr_size: usize,
  /// Row-major dark/light modules, `qr_size * qr_size` of them.
  pub qr_modules: Vec<bool>,
}

#[tauri::command]
pub async fn list_node_peers(
  node_id: String,
  backend: State<'_, Backend>,
) -> Result<Vec<NodePeerRecord>, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .list_node_peers(&backend.db, &node_id)
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn claim_node_peer(
  node_id: String,
  peer_id: String,
  label: String,
  backend: State<'_, Backend>,
) -> Result<NodePeerRecord, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .claim_node_peer(&backend.db, &node_id, &peer_id, &label)
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn get_peer_config(
  node_id: String,
  peer_id: String,
  backend: State<'_, Backend>,
) -> Result<PeerConfigPayload, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  let config_text = backend
    .orchestrator
    .build_peer_client_config(&backend.db, &backend.secure_storage, &node_id, &peer_id)
    .map_err(CommandError::from)?;

  let matrix = crate::backend::qr::encode(&config_text).map_err(|source| {
    CommandError::new(
      CommandErrorKind::Unknown,
      format!("Could not build a QR code for this device: {source}"),
      "Copy the configuration text instead, or export it to a file.",
    )
  })?;

  Ok(PeerConfigPayload {
    config_text,
    qr_size: matrix.size,
    qr_modules: matrix.modules,
  })
  })
  .await
}

/// Write a device config to disk and return where it went.
///
/// Follows `export_diagnostics_bundle`: the app picks the location and returns the path, rather
/// than taking one from the caller. `reveal_app_location` then opens it by key — that command
/// takes a key and never a path precisely so nothing running in the webview can aim the shell.
#[tauri::command]
pub async fn export_peer_config(
  node_id: String,
  peer_id: String,
  app: AppHandle,
  backend: State<'_, Backend>,
) -> Result<String, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  let config_text = backend
    .orchestrator
    .build_peer_client_config(&backend.db, &backend.secure_storage, &node_id, &peer_id)
    .map_err(CommandError::from)?;

  let peers = backend
    .orchestrator
    .list_node_peers(&backend.db, &node_id)
    .map_err(CommandError::from)?;

  let peer = peers
    .into_iter()
    .find(|peer| peer.id == peer_id)
    .ok_or_else(|| CommandError::invalid_input(
      "That device slot no longer exists.",
      "Refresh the node and try again.",
    ))?;

  let peer_config_dir = app
    .path()
    .app_data_dir()
    .map_err(CommandError::from)?
    .join(PEER_CONFIG_DIR_NAME);

  std::fs::create_dir_all(&peer_config_dir).map_err(CommandError::from)?;

  // WireGuard for Windows and the mobile apps derive the tunnel name from the file name, and
  // both restrict it to the same character set a tunnel name uses.
  let file_path = peer_config_dir.join(format!(
    "{}.conf",
    sanitize_config_file_stem(peer.label.as_deref().unwrap_or("device"), peer.slot_index)
  ));

  std::fs::write(&file_path, config_text).map_err(CommandError::from)?;

  Ok(file_path.display().to_string())
  })
  .await
}

/// A file stem WireGuard will accept, with the slot index appended so two devices sharing a name
/// cannot overwrite each other.
fn sanitize_config_file_stem(label: &str, slot_index: i64) -> String {
  let mut cleaned = String::new();
  let mut last_was_separator = false;

  for character in label.trim().chars() {
    if character.is_ascii_alphanumeric() {
      cleaned.push(character);
      last_was_separator = false;
    } else if !last_was_separator && !cleaned.is_empty() {
      cleaned.push('-');
      last_was_separator = true;
    }
  }

  let stem = cleaned.trim_matches('-');
  let stem = if stem.is_empty() { "device" } else { stem };

  // 32 leaves room for the suffix inside the 63-character limit a tunnel name has.
  let stem: String = stem.chars().take(32).collect();
  format!("{stem}-{slot_index}")
}

#[tauri::command]
pub async fn export_diagnostics_bundle(
  app: AppHandle,
  backend: State<'_, Backend>,
) -> Result<String, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  let bundle = backend
    .orchestrator
    .build_diagnostics_bundle(&backend.wireguard, &backend.db)
    .map_err(CommandError::from)?;

  let bundle_json = serde_json::to_string_pretty(&bundle).map_err(CommandError::from)?;
  let sanitized_json = logging::redact_sensitive_text(&bundle_json);

  let diagnostics_dir = app
    .path()
    .app_data_dir()
    .map_err(CommandError::from)?
    .join(DIAGNOSTICS_DIR_NAME);

  std::fs::create_dir_all(&diagnostics_dir).map_err(CommandError::from)?;

  let file_path = diagnostics_dir.join(format!("diagnostics-{}.json", bundle.generated_at));
  std::fs::write(&file_path, sanitized_json).map_err(CommandError::from)?;

  Ok(file_path.display().to_string())
  })
  .await
}

#[tauri::command]
pub async fn disconnect_tunnel(
  node_id: String,
  tunnel_id: String,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .disconnect_tunnel(
      &backend.wireguard,
      &backend.db,
      &backend.secure_storage,
      &DisconnectTunnelRequest { node_id, tunnel_id },
    )
    .map_err(CommandError::from)
  })
  .await
}

#[tauri::command]
pub async fn force_disconnect_node_service(
  node_id: String,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  backend
    .orchestrator
    .force_disconnect_node_service(&backend.wireguard, &backend.db, &node_id)
    .map_err(CommandError::from)
  })
  .await
}

/// Returns `Result` purely so this can run off the UI thread: Tauri rejects a command that both
/// borrows state and returns a bare value. It has no failure mode of its own — `check_readiness`
/// reports "not installed" as data — so the `Err` arm is unreachable in practice, and the frontend
/// binding needs no new error handling.
#[tauri::command]
pub async fn check_wireguard_readiness(
  app: tauri::AppHandle,
  backend: State<'_, Backend>,
) -> Result<WireGuardReadinessResponse, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  let readiness: WireGuardReadiness = backend.wireguard.check_readiness();
  Ok(WireGuardReadinessResponse {
    installed: readiness.installed,
    executable_found: readiness.executable_found,
    service_available: readiness.service_available,
    version: readiness.version,
    app_version: app.package_info().version.to_string(),
    message: readiness.message,
    action_guidance: readiness.action_guidance,
  })
  })
  .await
}

#[tauri::command]
pub async fn check_docker_readiness(
  backend: State<'_, Backend>,
) -> Result<DockerReadinessResponse, CommandError> {
  // Cloned out of the managed state so the body can move onto a blocking thread.
  // Every field is an Arc or a PathBuf, so this is cheap.
  let backend = backend.inner().clone();
  run_blocking(move || {
  if !backend.feature_flags.docker_dev {
    return Err(CommandError::dependency(
      "Docker development mode is disabled.",
      "Set BLINK_DEV_DOCKER=1 and restart the app.",
    ));
  }

  let cli_result = no_window_command("docker").args(["--version"]).output();
  let (installed, cli_version, cli_error) = match cli_result {
    Ok(output) if output.status.success() => {
      let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
      (true, Some(version), None)
    }
    Ok(output) => {
      let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
      (false, None, Some(stderr))
    }
    Err(error) => (false, None, Some(error.to_string())),
  };

  if !installed {
    return Ok(DockerReadinessResponse {
      installed: false,
      daemon_reachable: false,
      cli_version: None,
      daemon_version: None,
      message: "Docker CLI is not available on this machine.".to_string(),
      action_guidance: format!(
        "Install Docker Desktop for Windows, start Docker, and retry. {}",
        cli_error.unwrap_or_default()
      )
      .trim()
      .to_string(),
    });
  }

  let daemon_result = no_window_command("docker")
    .args(["info", "--format", "{{.ServerVersion}}"])
    .output();

  match daemon_result {
    Ok(output) if output.status.success() => {
      let daemon_version = String::from_utf8_lossy(&output.stdout).trim().to_string();
      Ok(DockerReadinessResponse {
        installed: true,
        daemon_reachable: true,
        cli_version,
        daemon_version: Some(daemon_version),
        message: "Docker CLI and daemon are available for development emulation.".to_string(),
        action_guidance: "You can use Docker development mode workflows from Setup and the map.".to_string(),
      })
    }
    Ok(output) => {
      let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
      Ok(DockerReadinessResponse {
        installed: true,
        daemon_reachable: false,
        cli_version,
        daemon_version: None,
        message: "Docker CLI is installed, but daemon is not reachable.".to_string(),
        action_guidance: format!(
          "Start Docker Desktop and retry readiness validation. {}",
          stderr
        )
        .trim()
        .to_string(),
      })
    }
    Err(error) => Ok(DockerReadinessResponse {
      installed: true,
      daemon_reachable: false,
      cli_version,
      daemon_version: None,
      message: "Docker CLI is installed, but daemon check failed.".to_string(),
      action_guidance: format!(
        "Start Docker Desktop and retry readiness validation. {}",
        error
      )
      .trim()
      .to_string(),
    }),
  }
  })
  .await
}

/// Keys `list_app_locations` reports and `reveal_app_location` accepts.
///
/// The two commands share this list so a location can never be shown without being openable, or
/// openable without being shown.
const APP_LOCATION_KEYS: [&str; 5] =
  ["diagnostics", "peer-configs", "logs", "tunnels", "database"];

/// Takes the base directories rather than an `AppHandle` so the catalog can be tested without a
/// running Tauri app — an unknown key must be rejected, and that is worth proving.
fn resolve_app_location(
  key: &str,
  app_data_dir: &std::path::Path,
  app_log_dir: &std::path::Path,
) -> Result<AppLocation, CommandError> {
  let (label, description, path) = match key {
    "diagnostics" => (
      "Diagnostics exports",
      "Redacted bundles written by Export.",
      app_data_dir.join(DIAGNOSTICS_DIR_NAME),
    ),
    "peer-configs" => (
      "Device configurations",
      "WireGuard .conf files exported for phones and other devices.",
      app_data_dir.join(PEER_CONFIG_DIR_NAME),
    ),
    "logs" => (
      "Application log",
      "Written by the app itself. Warnings and errors only in release builds.",
      app_log_dir.to_path_buf(),
    ),
    "tunnels" => (
      "Tunnel configuration",
      "The .conf files handed to the WireGuard tunnel service.",
      crate::backend::wireguard::tunnel_config_dir_path(),
    ),
    "database" => (
      "Local database",
      "Provider, node, tunnel and activity records. Holds secret references, never secrets.",
      app_data_dir.join(crate::backend::db::DATABASE_FILE_NAME),
    ),
    _ => {
      return Err(CommandError::invalid_input(
        format!("'{key}' is not a location this app can open."),
        "Choose one of the listed locations.",
      ));
    }
  };

  Ok(AppLocation {
    key: key.to_string(),
    label: label.to_string(),
    description: description.to_string(),
    exists: path.exists(),
    path: path.display().to_string(),
  })
}

#[tauri::command]
pub async fn list_app_locations(app: AppHandle) -> Result<Vec<AppLocation>, CommandError> {
  run_blocking(move || {
  let app_data_dir = app.path().app_data_dir()?;
  let app_log_dir = app.path().app_log_dir()?;

  APP_LOCATION_KEYS
    .iter()
    .map(|key| resolve_app_location(key, &app_data_dir, &app_log_dir))
    .collect()
  })
  .await
}

/// Opens one of the known locations in Explorer.
///
/// Takes a **key, never a path**: accepting a caller-supplied path would let anything running in
/// the webview hand an arbitrary target to the shell. The path is resolved here.
#[tauri::command]
pub async fn reveal_app_location(key: String, app: AppHandle) -> Result<(), CommandError> {
  run_blocking(move || {
  if !cfg!(target_os = "windows") {
    return Err(CommandError::dependency(
      "Opening a folder is supported on Windows only.",
      "Open the path shown next to the location manually.",
    ));
  }

  let location = resolve_app_location(&key, &app.path().app_data_dir()?, &app.path().app_log_dir()?)?;
  let path = PathBuf::from(&location.path);

  if !path.exists() {
    return Err(CommandError::invalid_input(
      format!("{} has not been created yet.", location.label),
      "It appears once the app has written something there.",
    ));
  }

  // `/select,` highlights a file inside its folder; for a directory it opens the parent with the
  // directory selected, which is the behaviour a user expects from "reveal".
  no_window_command("explorer.exe")
    .arg(format!("/select,{}", path.display()))
    .spawn()
    .map_err(|error| format!("Could not open the folder: {error}"))?;

  Ok(())
  })
  .await
}

#[tauri::command]
pub async fn open_wireguard_install_page() -> Result<(), CommandError> {
  run_blocking(move || {
    open_in_browser(
      "https://www.wireguard.com/install/",
      "Install WireGuard manually from wireguard.com/install.",
    )
  })
  .await
}

/// Opens the page where a provider issues the credential Blink needs.
///
/// Takes a provider id, never a URL: the address comes from the adapter's own declaration, so the
/// webview cannot use this to open anything else.
#[tauri::command]
pub async fn open_provider_token_page(
  provider_id: String,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  let backend = backend.inner().clone();
  run_blocking(move || {
    let fields = backend
      .provider
      .credential_fields_for_provider(&backend.db, &provider_id)
      .map_err(CommandError::from)?;
    let url = fields
      .iter()
      .find_map(|field| field.token_page_url.clone())
      .ok_or_else(|| {
        CommandError::invalid_input(
          "This provider has no single page that issues keys.",
          "Create the credential in your cloud's console, then paste it here.",
        )
      })?;
    open_in_browser(&url, "Open your provider's console in a browser and create the key there.")
  })
  .await
}

/// Plain https with none of the characters `cmd` treats specially.
fn is_safe_browser_url(url: &str) -> bool {
  url.starts_with("https://")
    && url.len() > "https://".len()
    && !url.chars().any(|c| c.is_whitespace() || matches!(c, '&' | '|' | '^' | '<' | '>' | '"' | '%' | '(' | ')'))
}

/// Opens an https URL in the default browser.
///
/// `cmd /C start` is a shell, so the URL is refused unless it is plain https with no characters
/// cmd treats specially. Every caller passes a constant from this codebase; the check is so that
/// stays the only way it can be used.
fn open_in_browser(url: &str, fallback: &str) -> Result<(), CommandError> {
  if !cfg!(target_os = "windows") {
    return Err(CommandError::dependency("Opening a browser is supported on Windows only.", fallback));
  }
  if !is_safe_browser_url(url) {
    return Err(CommandError::invalid_input("That address cannot be opened safely.", fallback));
  }

  no_window_command("cmd")
    .args(["/C", "start", "", url])
    .spawn()
    .map_err(|error| CommandError::dependency(format!("Could not open the browser: {error}"), fallback))?;
  Ok(())
}

#[tauri::command]
pub async fn install_wireguard_with_prerequisites() -> Result<WireGuardInstallResult, CommandError> {
  run_blocking(move || {
  if !cfg!(target_os = "windows") {
    return Err(CommandError::dependency(
      "WireGuard installation is supported on Windows only.",
      "Install WireGuard manually from wireguard.com/install.",
    ));
  }

  let script_path = build_wireguard_install_script().map_err(|error| {
    format!(
      "Failed to prepare WireGuard installer workflow: {}",
      crate::backend::logging::redact_sensitive_text(&error.to_string())
    )
  })?;

  let script_path_str = script_path.to_string_lossy().replace('"', "`\"");

  let launch_result = no_window_command("powershell")
    .args([
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      &format!(
        "Start-Process -FilePath 'powershell' -Verb RunAs -Wait -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','{}'",
        script_path_str
      ),
    ])
    .status();

  let _ = fs::remove_file(&script_path);

  let status = launch_result.map_err(|error| {
    format!(
      "Failed to launch elevated WireGuard installer flow: {}",
      crate::backend::logging::redact_sensitive_text(&error.to_string())
    )
  })?;

  if !status.success() {
    return Err(CommandError::dependency(
      "WireGuard installation was cancelled or did not complete.",
      "Accept the elevation prompt and finish the installer, then retry.",
    ));
  }

  Ok(WireGuardInstallResult {
    installed: true,
    prerequisites_configured: true,
    installer_path: "C:\\Program Files\\WireGuard\\wireguard.exe".to_string(),
    message: "WireGuard installer completed and prerequisites were configured.".to_string(),
    next_action: "Click Re-check to verify readiness and continue onboarding.".to_string(),
  })
  })
  .await
}

fn build_wireguard_install_script() -> Result<PathBuf, std::io::Error> {
  let script_content = r#"
$ErrorActionPreference = 'Stop'

$downloadUrl = 'https://download.wireguard.com/windows-client/wireguard-installer.exe'
$tempDir = Join-Path $env:TEMP 'blink-wireguard-installer'
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

$installerPath = Join-Path $tempDir 'wireguard-installer.exe'
Invoke-WebRequest -UseBasicParsing -Uri $downloadUrl -OutFile $installerPath

if (-not (Test-Path $installerPath)) {
  throw 'WireGuard installer download failed.'
}

Start-Process -FilePath $installerPath -Wait

$wireGuardDir = 'C:\Program Files\WireGuard'
if (-not (Test-Path $wireGuardDir)) {
  throw 'WireGuard installation directory not found after installer run.'
}

$machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
if (-not $machinePath) { $machinePath = '' }
if ($machinePath -notlike "*$wireGuardDir*") {
  $newPath = if ($machinePath.Trim().Length -gt 0) { "$machinePath;$wireGuardDir" } else { $wireGuardDir }
  [Environment]::SetEnvironmentVariable('Path', $newPath, 'Machine')
}

$wireguardCommand = Get-Command wireguard -ErrorAction SilentlyContinue
$service = Get-Service -Name 'WireGuardManager' -ErrorAction SilentlyContinue

if (-not $wireguardCommand) {
  throw 'WireGuard command is not available after installation.'
}

if (-not $service) {
  throw 'WireGuardManager service is not available after installation.'
}
"#;

  let mut path = std::env::temp_dir();
  path.push(format!("blink-wireguard-install-{}.ps1", uuid::Uuid::new_v4()));
  fs::write(&path, script_content)?;
  Ok(path)
}

#[cfg(test)]
mod tests {

  #[test]
  fn browser_urls_must_be_plain_https() {
    use super::is_safe_browser_url;
    assert!(is_safe_browser_url("https://cloud.linode.com/profile/tokens"));
    assert!(is_safe_browser_url("https://console.aws.amazon.com/iam/home#/security_credentials"));
    for refused in [
      "http://example.com",
      "https://",
      "file:///C:/Windows",
      "https://example.com/&calc",
      "https://example.com/|calc",
      "https://example.com/ x",
      "https://example.com/%PATH%",
    ] {
      assert!(!is_safe_browser_url(refused), "{refused} must be refused");
    }
  }

  use super::{
    clear_provider_credential, ensure_builtin_providers, resolve_app_location, CommandErrorKind,
    APP_LOCATION_KEYS,
  };
  use crate::backend::db::{Database, ProviderStatus};
  use crate::backend::secure_storage::SecureStorage;
  use std::fs;
  use std::path::PathBuf;
  use std::time::{SystemTime, UNIX_EPOCH};

  fn test_db_path(name: &str) -> PathBuf {
    let unique = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("system time should be after unix epoch")
      .as_nanos();

    std::env::temp_dir().join(format!("blink-system-{name}-{unique}.db"))
  }

  fn create_test_db(name: &str) -> (Database, PathBuf) {
    let path = test_db_path(name);
    let db = Database::new(path.clone());
    db.run_migrations().expect("migrations should succeed");
    (db, path)
  }

  /// The reveal command resolves paths from a key rather than accepting one, so anything running
  /// in the webview cannot hand an arbitrary target to the Windows shell. An unrecognised key must
  /// be refused rather than falling through to some default.
  #[test]
  fn an_unknown_location_key_is_rejected_as_invalid_input() {
    let error = resolve_app_location(
      "c:\\windows\\system32",
      &PathBuf::from("data"),
      &PathBuf::from("logs"),
    )
    .expect_err("an unlisted key must not resolve");

    assert_eq!(error.kind, CommandErrorKind::InvalidInput);
  }

  /// The listing and the opener share one catalog; a key in one but not the other would either
  /// show a location that cannot be opened or open one that is never shown.
  #[test]
  fn every_listed_location_key_resolves() {
    for key in APP_LOCATION_KEYS {
      let location = resolve_app_location(key, &PathBuf::from("data"), &PathBuf::from("logs"))
        .unwrap_or_else(|_| panic!("listed key '{key}' should resolve"));

      assert_eq!(location.key, key);
      assert!(!location.label.is_empty(), "'{key}' needs a label");
      assert!(!location.path.is_empty(), "'{key}' needs a path");
    }
  }

  #[test]
  fn ensure_builtin_providers_recreates_missing_builtin_rows() {
    let (db, path) = create_test_db("restore-builtins");

    db.delete_provider("provider-docker")
      .expect("seeded docker provider should delete cleanly");

    ensure_builtin_providers(&db, true).expect("builtin providers should be restored");

    let providers = db.list_providers().expect("provider listing should succeed");
    assert!(providers.iter().any(|provider| provider.id == "provider-1"));
    assert!(providers.iter().any(|provider| provider.id == "provider-docker"));

    let _ = fs::remove_file(path);
  }

  /// Four commands call `ensure_builtin_providers`, and the dashboard fires them together on load.
  ///
  /// They used to be serialized by Tauri running every command on the UI thread, so the
  /// check-then-insert inside could never interleave. Now that commands run on the blocking pool,
  /// the losers of that race hit a unique-constraint violation and the whole dashboard came back
  /// empty. The insert has to be idempotent, and repeating it must not duplicate rows.
  #[test]
  fn ensuring_builtin_providers_repeatedly_is_idempotent() {
    let (db, path) = create_test_db("ensure-builtins-idempotent");

    for _ in 0..5 {
      ensure_builtin_providers(&db, true).expect("repeat ensure must not fail on an existing row");
    }

    let providers = db.list_providers().expect("provider listing should succeed");
    assert_eq!(
      providers.iter().filter(|provider| provider.id == "provider-1").count(),
      1,
      "the built-in DigitalOcean row must exist exactly once"
    );
    assert_eq!(
      providers.iter().filter(|provider| provider.id == "provider-docker").count(),
      1,
      "the built-in Docker row must exist exactly once"
    );

    let _ = fs::remove_file(path);
  }

  /// A second insert of an existing provider must not clobber what is already stored — in
  /// particular the credential reference, which would silently disconnect the user's saved key.
  #[test]
  fn re_creating_an_existing_provider_leaves_its_stored_row_intact() {
    let (db, path) = create_test_db("create-provider-idempotent");

    db.update_provider("provider-1", "DigitalOcean", "digitalocean", Some("ref-abc"))
      .expect("provider should accept a token ref");

    db.create_provider("provider-1", "Something Else", "digitalocean", None)
      .expect("re-creating an existing provider must succeed rather than error");

    let provider = db
      .get_provider("provider-1")
      .expect("query should succeed")
      .expect("provider should still exist");

    assert_eq!(provider.name, "DigitalOcean", "the existing name must survive");
    assert_eq!(
      provider.token_ref.as_deref(),
      Some("ref-abc"),
      "the stored credential reference must survive"
    );

    let _ = fs::remove_file(path);
  }

  #[test]
  fn clear_provider_credential_keeps_provider_row_and_resets_state() {
    let (db, path) = create_test_db("clear-provider-credential");
    let secure_storage = SecureStorage::dev_mock();
    let token_ref = secure_storage
      .store_provider_credential("provider-1", "dop_v1_secret")
      .expect("token store should succeed");

    db.update_provider("provider-1", "DigitalOcean", "digitalocean", Some(&token_ref))
      .expect("provider should accept token ref update");
    db.suspend_provider("provider-1")
      .expect("provider suspend should succeed");

    clear_provider_credential(&db, &secure_storage, "provider-1")
      .expect("credential clear should succeed without attached nodes");

    let provider = db
      .get_provider("provider-1")
      .expect("provider lookup should succeed")
      .expect("provider row should still exist");

    assert_eq!(provider.token_ref, None);
    assert_eq!(provider.status, ProviderStatus::Active);
    // The verdict described the removed key; carrying it over would mark the next key verified.
    assert_eq!(provider.last_validation_status, None);
    assert_eq!(
      secure_storage
        .get_provider_credential(&token_ref)
        .expect("secret lookup should succeed"),
      None
    );

    let _ = fs::remove_file(path);
  }

  #[test]
  fn clear_provider_credential_rejects_when_nodes_are_attached() {
    let (db, path) = create_test_db("clear-provider-credential-attached-node");
    let secure_storage = SecureStorage::dev_mock();
    let token_ref = secure_storage
      .store_provider_credential("provider-1", "dop_v1_secret")
      .expect("token store should succeed");

    db.update_provider("provider-1", "DigitalOcean", "digitalocean", Some(&token_ref))
      .expect("provider should accept token ref update");
    db.create_vpn_node("node-1", "provider-1", "Primary", "nyc1")
      .expect("node creation should succeed");

    let error = clear_provider_credential(&db, &secure_storage, "provider-1")
      .expect_err("credential clear should fail with attached nodes");

    // Assert on the machine-readable kind, not on message wording.
    assert_eq!(error.kind, CommandErrorKind::InvalidState);
    assert!(error.next_action.contains("Destroy the dependent nodes"));
    let provider = db
      .get_provider("provider-1")
      .expect("provider lookup should succeed")
      .expect("provider row should still exist");
    assert_eq!(provider.token_ref.as_deref(), Some(token_ref.as_str()));

    let _ = fs::remove_file(path);
  }
}

