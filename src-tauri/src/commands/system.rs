use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tauri::State;
use tauri::{AppHandle, Manager};

use crate::backend::Backend;
use crate::commands::error::{CommandError, CommandErrorKind};
use crate::backend::db::{Database, ProviderRecord, ProviderStatus};
use crate::backend::logging;
use crate::backend::provider::{ProviderPlanPricing, ProviderRegion};
use crate::backend::orchestrator::{
  ConnectTunnelRequest, CreateClientTunnelRequest, CreateVpnNodeRequest, DisconnectTunnelRequest,
  ForceCleanupResult, KillSwitchState, TunnelSettings, UnexpectedDrop,
};
use crate::backend::db::{TunnelRecord, VpnNodeRecord};
use crate::backend::secure_storage::SecureStorage;
use crate::backend::wireguard::WireGuardReadiness;

const BUILTIN_DIGITALOCEAN_PROVIDER_ID: &str = "provider-1";
const BUILTIN_DIGITALOCEAN_PROVIDER_NAME: &str = "DigitalOcean";
const BUILTIN_DIGITALOCEAN_PROVIDER_SLUG: &str = "digitalocean";
const BUILTIN_DOCKER_PROVIDER_ID: &str = "provider-docker";
const BUILTIN_DOCKER_PROVIDER_NAME: &str = "Docker";
const BUILTIN_DOCKER_PROVIDER_SLUG: &str = "docker";

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

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsBundleResponse {
  pub generated_at: String,
  pub providers: Vec<crate::backend::db::ProviderRecord>,
  pub nodes: Vec<VpnNodeRecord>,
  pub tunnels: Vec<TunnelRecord>,
  pub init_runs: Vec<crate::backend::db::InitRunLogRecord>,
  pub redacted_notes: Vec<String>,
  pub wireguard_debug: crate::backend::wireguard::WireGuardDebugSnapshot,
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
      .delete_provider_api_token(token_ref)
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
pub fn hello(name: Option<String>, backend: State<'_, Backend>) -> String {
  let caller = name.unwrap_or_else(|| "operator".to_string());
  let provider = backend.provider.active().provider_name();
  format!(
    "Hello, {caller}. Rust backend is reachable (provider adapter: {provider})."
  )
}

#[tauri::command]
pub fn validate_provider_token(provider_id: String, backend: State<'_, Backend>) -> Result<String, CommandError> {
  let account = backend
    .provider
    .validate_provider_token(&backend.db, &backend.secure_storage, &provider_id)
    .map_err(CommandError::from)?;

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
) -> Result<String, CommandError> {
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
    .map_err(CommandError::from)?;

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
) -> Result<(), CommandError> {
  backend
    .orchestrator
    .destroy_droplet_for_provider(
      &backend.provider,
      &backend.db,
      &backend.secure_storage,
      &provider_id,
      droplet_id,
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn register_provider_token(
  provider_id: String,
  provider_name: String,
  api_token: String,
  backend: State<'_, Backend>,
) -> Result<String, CommandError> {
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

  let provider = backend
    .provider
    .upsert_provider_token_for_slug(
      &backend.db,
      &backend.secure_storage,
      &provider_id,
      &provider_name,
      &slug,
      &api_token,
    )
    .map_err(CommandError::from)?;

  let token_ref = provider
    .token_ref
    .unwrap_or_else(|| "<missing-token-ref>".to_string());

  Ok(format!("Provider registered with token_ref: {}", token_ref))
}

#[tauri::command]
pub fn list_provider_pricing(
  provider_id: String,
  region: String,
  backend: State<'_, Backend>,
) -> Result<ProviderPricingResponse, CommandError> {
  let plans = backend
    .provider
    .list_provider_plan_pricing(&backend.db, &backend.secure_storage, &provider_id, &region)
    .map_err(CommandError::from)?;

  Ok(ProviderPricingResponse {
    provider_id,
    region,
    plans,
  })
}

#[tauri::command]
pub fn list_provider_regions(
  provider_id: String,
  backend: State<'_, Backend>,
) -> Result<Vec<ProviderRegion>, CommandError> {
  backend
    .provider
    .list_provider_regions(&backend.db, &backend.secure_storage, &provider_id)
    .map_err(CommandError::from)
}

/// The node the app connects to by default, or `null` when the user has more than one node and
/// has not chosen between them.
#[tauri::command]
pub fn get_current_node_id(backend: State<'_, Backend>) -> Result<Option<String>, CommandError> {
  backend
    .orchestrator
    .get_current_node_id(&backend.db)
    .map_err(CommandError::from)
}

/// Client tunnel parameters. Applied when a tunnel next starts; a running tunnel keeps the config
/// it was started with.
#[tauri::command]
pub fn get_tunnel_settings(backend: State<'_, Backend>) -> Result<TunnelSettings, CommandError> {
  backend
    .orchestrator
    .get_tunnel_settings(&backend.db)
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn set_tunnel_settings(
  dns_server: Option<String>,
  keepalive_seconds: u16,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  backend
    .orchestrator
    .set_tunnel_settings(&backend.db, &TunnelSettings { dns_server, keepalive_seconds })
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn set_current_node_id(
  node_id: Option<String>,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  backend
    .orchestrator
    .set_current_node_id(&backend.db, node_id.as_deref())
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn list_vpn_nodes(backend: State<'_, Backend>) -> Result<Vec<VpnNodeRecord>, CommandError> {
  backend
    .orchestrator
    .list_nodes(&backend.provider, &backend.db, &backend.secure_storage)
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn list_providers(backend: State<'_, Backend>) -> Result<Vec<ProviderRecord>, CommandError> {
  ensure_builtin_providers(&backend.db, backend.feature_flags.docker_dev)?;
  backend.db.list_providers().map_err(CommandError::from)
}

#[tauri::command]
pub fn suspend_provider(provider_id: String, backend: State<'_, Backend>) -> Result<(), CommandError> {
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
}

#[tauri::command]
pub fn resume_provider(provider_id: String, backend: State<'_, Backend>) -> Result<(), CommandError> {
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
}

#[tauri::command]
pub fn delete_provider(provider_id: String, backend: State<'_, Backend>) -> Result<(), CommandError> {
  clear_provider_credential(&backend.db, &backend.secure_storage, &provider_id)
}

#[tauri::command]
pub fn list_tunnels_for_node(
  node_id: String,
  backend: State<'_, Backend>,
) -> Result<Vec<TunnelRecord>, CommandError> {
  backend
    .orchestrator
    .list_tunnels_for_node(&backend.db, &node_id)
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn destroy_vpn_node(node_id: String, backend: State<'_, Backend>) -> Result<(), CommandError> {
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
}

#[tauri::command]
pub fn force_cleanup_vpn_node(
  node_id: String,
  backend: State<'_, Backend>,
) -> Result<ForceCleanupResult, CommandError> {
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
}

#[tauri::command]
pub fn create_vpn_node_to_ready(
  node_id: String,
  provider_id: String,
  node_name: String,
  region: String,
  plan_key: String,
  backend: State<'_, Backend>,
) -> Result<String, CommandError> {
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
) -> Result<String, CommandError> {
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
        client_address_cidr,
        dns_server,
        allowed_ips,
      },
    )
    .map_err(CommandError::from)?;

  Ok(format!(
    "Tunnel configured: tunnel_id={}, tunnel_name={}, client_public_key={}",
    result.tunnel_id, result.tunnel_name, result.client_public_key
  ))
}

#[tauri::command]
pub fn activate_client_tunnel(tunnel_id: String, backend: State<'_, Backend>) -> Result<(), CommandError> {
  backend
    .orchestrator
    .activate_client_tunnel(&backend.wireguard, &backend.db, &backend.secure_storage, &tunnel_id)
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn deactivate_client_tunnel(
  tunnel_id: String,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  backend
    .orchestrator
    .deactivate_client_tunnel(&backend.wireguard, &backend.db, &backend.secure_storage, &tunnel_id)
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn read_client_tunnel_status(
  tunnel_id: String,
  backend: State<'_, Backend>,
) -> Result<String, CommandError> {
  let status = backend
    .orchestrator
    .read_client_tunnel_status(&backend.wireguard, &backend.db, &tunnel_id)
    .map_err(CommandError::from)?;

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
) -> Result<(), CommandError> {
  backend
    .orchestrator
    .connect_tunnel(
      &backend.wireguard,
      &backend.db,
      &backend.secure_storage,
      &ConnectTunnelRequest { node_id, tunnel_id },
    )
    .map_err(CommandError::from)
}

  #[tauri::command]
  pub fn get_health_panel_data(
    node_id: String,
    tunnel_id: Option<String>,
    force_ip_probe: Option<bool>,
    backend: State<'_, Backend>,
  ) -> Result<HealthPanelData, CommandError> {
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
      metrics_source: summary.metrics_source.map(|source| source.as_str().to_string()),
      connected_at: summary.connected_at,
      kill_switch: summary.kill_switch,
      unexpected_drop: summary.unexpected_drop,
    })
  }

  #[tauri::command]
  pub fn get_diagnostics_bundle(backend: State<'_, Backend>) -> Result<DiagnosticsBundleResponse, CommandError> {
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
      init_runs: bundle.init_runs,
      redacted_notes: bundle.redacted_notes,
      wireguard_debug: bundle.wireguard_debug,
    })
  }

#[tauri::command]
pub fn export_diagnostics_bundle(
  app: AppHandle,
  backend: State<'_, Backend>,
) -> Result<String, CommandError> {
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
    .join("diagnostics");

  std::fs::create_dir_all(&diagnostics_dir).map_err(CommandError::from)?;

  let file_path = diagnostics_dir.join(format!("diagnostics-{}.json", bundle.generated_at));
  std::fs::write(&file_path, sanitized_json).map_err(CommandError::from)?;

  Ok(file_path.display().to_string())
}

#[tauri::command]
pub fn disconnect_tunnel(
  node_id: String,
  tunnel_id: String,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  backend
    .orchestrator
    .disconnect_tunnel(
      &backend.wireguard,
      &backend.db,
      &backend.secure_storage,
      &DisconnectTunnelRequest { node_id, tunnel_id },
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn force_disconnect_node_service(
  node_id: String,
  backend: State<'_, Backend>,
) -> Result<(), CommandError> {
  backend
    .orchestrator
    .force_disconnect_node_service(&backend.wireguard, &backend.db, &node_id)
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn check_wireguard_readiness(
  app: tauri::AppHandle,
  backend: State<'_, Backend>,
) -> WireGuardReadinessResponse {
  let readiness: WireGuardReadiness = backend.wireguard.check_readiness();
  WireGuardReadinessResponse {
    installed: readiness.installed,
    executable_found: readiness.executable_found,
    service_available: readiness.service_available,
    version: readiness.version,
    app_version: app.package_info().version.to_string(),
    message: readiness.message,
    action_guidance: readiness.action_guidance,
  }
}

#[tauri::command]
pub fn check_docker_readiness(
  backend: State<'_, Backend>,
) -> Result<DockerReadinessResponse, CommandError> {
  if !backend.feature_flags.docker_dev {
    return Err(CommandError::dependency(
      "Docker development mode is disabled.",
      "Set GOATSTARE_DEV_DOCKER=1 and restart the app.",
    ));
  }

  let cli_result = Command::new("docker").args(["--version"]).output();
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

  let daemon_result = Command::new("docker")
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
        action_guidance: "You can use Docker development mode workflows from Settings and Node planning surfaces.".to_string(),
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
}

#[tauri::command]
pub fn open_wireguard_install_page() -> Result<(), CommandError> {
  if !cfg!(target_os = "windows") {
    return Err(CommandError::dependency(
      "WireGuard install shortcut is supported on Windows only in Phase 1.",
      "Install WireGuard manually from wireguard.com/install.",
    ));
  }

  Command::new("cmd")
    .args(["/C", "start", "", "https://www.wireguard.com/install/"])
    .spawn()
    .map_err(|error| format!("Could not open WireGuard install page: {error}"))?;

  Ok(())
}

#[tauri::command]
pub fn install_wireguard_with_prerequisites() -> Result<WireGuardInstallResult, CommandError> {
  if !cfg!(target_os = "windows") {
    return Err(CommandError::dependency(
      "WireGuard installation is supported on Windows only for Phase 1.",
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

  let launch_result = Command::new("powershell")
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
}

fn build_wireguard_install_script() -> Result<PathBuf, std::io::Error> {
  let script_content = r#"
$ErrorActionPreference = 'Stop'

$downloadUrl = 'https://download.wireguard.com/windows-client/wireguard-installer.exe'
$tempDir = Join-Path $env:TEMP 'goatstare-wireguard-installer'
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
  path.push(format!("goatstare-wireguard-install-{}.ps1", uuid::Uuid::new_v4()));
  fs::write(&path, script_content)?;
  Ok(path)
}

#[cfg(test)]
mod tests {
  use super::{clear_provider_credential, ensure_builtin_providers, CommandErrorKind};
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

    std::env::temp_dir().join(format!("goatstare-system-{name}-{unique}.db"))
  }

  fn create_test_db(name: &str) -> (Database, PathBuf) {
    let path = test_db_path(name);
    let db = Database::new(path.clone());
    db.run_migrations().expect("migrations should succeed");
    (db, path)
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

  #[test]
  fn clear_provider_credential_keeps_provider_row_and_resets_state() {
    let (db, path) = create_test_db("clear-provider-credential");
    let secure_storage = SecureStorage::dev_mock();
    let token_ref = secure_storage
      .store_provider_api_token("provider-1", "dop_v1_secret")
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
        .get_provider_api_token(&token_ref)
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
      .store_provider_api_token("provider-1", "dop_v1_secret")
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

