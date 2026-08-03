use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tauri::State;
use tauri::{AppHandle, Manager};

use crate::backend::Backend;
use crate::backend::db::{Database, ProviderRecord, ProviderStatus};
use crate::backend::logging;
use crate::backend::provider::ProviderPlanPricing;
use crate::backend::orchestrator::{
  ConnectTunnelRequest, CreateClientTunnelRequest, CreateVpnNodeRequest, DisconnectTunnelRequest,
  ForceCleanupResult,
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
  pub external_ipv4: Option<String>,
  pub tunnel_status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsBundleResponse {
  pub generated_at: String,
  pub providers: Vec<crate::backend::db::ProviderRecord>,
  pub nodes: Vec<VpnNodeRecord>,
  pub tunnels: Vec<TunnelRecord>,
  pub init_runs: Vec<crate::backend::db::InitRunLogRecord>,
  pub redacted_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WireGuardReadinessResponse {
  pub installed: bool,
  pub executable_found: bool,
  pub service_available: bool,
  pub version: Option<String>,
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
) -> Result<(), String> {
  if db
    .get_provider(provider_id)
    .map_err(|err| err.to_string())?
    .is_none()
  {
    db.create_provider(provider_id, provider_name, provider_slug, None)
      .map_err(|err| err.to_string())?;
  }

  Ok(())
}

fn ensure_builtin_providers(db: &Database, docker_dev_enabled: bool) -> Result<(), String> {
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
) -> Result<(), String> {
  let provider = db
    .get_provider(provider_id)
    .map_err(|err| err.to_string())?
    .ok_or_else(|| format!("Provider '{}' was not found.", provider_id))?;

  let provider_nodes = db
    .list_vpn_nodes()
    .map_err(|err| err.to_string())?
    .into_iter()
    .filter(|node| node.provider_id == provider_id)
    .count();

  if provider_nodes > 0 {
    return Err(format!(
      "Cannot delete provider key while {} node{} are still attached. Destroy dependent nodes first.",
      provider_nodes,
      if provider_nodes == 1 { "" } else { "s" }
    ));
  }

  if let Some(token_ref) = provider.token_ref.as_deref() {
    secure_storage
      .delete_provider_api_token(token_ref)
      .map_err(|err| err.to_string())?;
  }

  db.update_provider_token_ref(provider_id, None)
    .map_err(|err| err.to_string())?;

  if provider.status == ProviderStatus::Suspended {
    db.resume_provider(provider_id).map_err(|err| err.to_string())?;
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
pub fn register_docker_provider_token(
  provider_id: String,
  provider_name: String,
  api_token: String,
  backend: State<'_, Backend>,
) -> Result<String, String> {
  let provider = backend
    .provider
    .upsert_docker_provider_with_token(
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
pub fn list_provider_pricing(
  provider_id: String,
  region: String,
  backend: State<'_, Backend>,
) -> Result<ProviderPricingResponse, String> {
  let plans = backend
    .provider
    .list_provider_plan_pricing(&backend.db, &backend.secure_storage, &provider_id, &region)
    .map_err(|err| err.to_string())?;

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
) -> Result<Vec<String>, String> {
  backend
    .provider
    .list_provider_regions(&backend.db, &backend.secure_storage, &provider_id)
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn list_vpn_nodes(backend: State<'_, Backend>) -> Result<Vec<VpnNodeRecord>, String> {
  backend
    .orchestrator
    .list_nodes(&backend.db)
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn list_providers(backend: State<'_, Backend>) -> Result<Vec<ProviderRecord>, String> {
  ensure_builtin_providers(&backend.db, backend.feature_flags.docker_dev)?;
  backend.db.list_providers().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn suspend_provider(provider_id: String, backend: State<'_, Backend>) -> Result<(), String> {
  let provider = backend
    .db
    .get_provider(&provider_id)
    .map_err(|err| err.to_string())?
    .ok_or_else(|| format!("Provider '{}' was not found.", provider_id))?;

  if provider.status == crate::backend::db::ProviderStatus::Suspended {
    return Ok(());
  }

  backend
    .db
    .suspend_provider(&provider_id)
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn resume_provider(provider_id: String, backend: State<'_, Backend>) -> Result<(), String> {
  let provider = backend
    .db
    .get_provider(&provider_id)
    .map_err(|err| err.to_string())?
    .ok_or_else(|| format!("Provider '{}' was not found.", provider_id))?;

  if provider.status == crate::backend::db::ProviderStatus::Active {
    return Ok(());
  }

  backend
    .db
    .resume_provider(&provider_id)
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn delete_provider(provider_id: String, backend: State<'_, Backend>) -> Result<(), String> {
  clear_provider_credential(&backend.db, &backend.secure_storage, &provider_id)
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
pub fn force_cleanup_vpn_node(
  node_id: String,
  backend: State<'_, Backend>,
) -> Result<ForceCleanupResult, String> {
  backend
    .orchestrator
    .force_cleanup_vpn_node(
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
  plan_key: String,
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
        plan_key,
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
    ensure_builtin_providers(&backend.db, backend.feature_flags.docker_dev)?;
    let bundle = backend
      .orchestrator
      .build_diagnostics_bundle(&backend.db)
      .map_err(|err| err.to_string())?;

    Ok(DiagnosticsBundleResponse {
      generated_at: bundle.generated_at,
      providers: bundle.providers,
      nodes: bundle.nodes,
      tunnels: bundle.tunnels,
      init_runs: bundle.init_runs,
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

#[tauri::command]
pub fn check_wireguard_readiness(backend: State<'_, Backend>) -> WireGuardReadinessResponse {
  let readiness: WireGuardReadiness = backend.wireguard.check_readiness();
  WireGuardReadinessResponse {
    installed: readiness.installed,
    executable_found: readiness.executable_found,
    service_available: readiness.service_available,
    version: readiness.version,
    message: readiness.message,
    action_guidance: readiness.action_guidance,
  }
}

#[tauri::command]
pub fn check_docker_readiness(
  backend: State<'_, Backend>,
) -> Result<DockerReadinessResponse, String> {
  if !backend.feature_flags.docker_dev {
    return Err(
      "Docker development mode is disabled. Set GOATSTARE_DEV_DOCKER=1 and restart the app."
        .to_string(),
    );
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
pub fn open_wireguard_install_page() -> Result<(), String> {
  if !cfg!(target_os = "windows") {
    return Err("WireGuard install shortcut is supported on Windows only in Phase 1.".to_string());
  }

  Command::new("cmd")
    .args(["/C", "start", "", "https://www.wireguard.com/install/"])
    .spawn()
    .map_err(|error| format!("Could not open WireGuard install page: {error}"))?;

  Ok(())
}

#[tauri::command]
pub fn install_wireguard_with_prerequisites() -> Result<WireGuardInstallResult, String> {
  if !cfg!(target_os = "windows") {
    return Err("WireGuard installation is supported on Windows only for Phase 1.".to_string());
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
    return Err(
      "WireGuard installation was cancelled or failed. Accept the UAC prompt and complete the installer, then retry."
        .to_string(),
    );
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

$wgCommand = Get-Command wg -ErrorAction SilentlyContinue
$wireguardCommand = Get-Command wireguard -ErrorAction SilentlyContinue
$service = Get-Service -Name 'WireGuardManager' -ErrorAction SilentlyContinue

if (-not $wgCommand -or -not $wireguardCommand) {
  throw 'WireGuard commands are not available after installation.'
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
  use super::{clear_provider_credential, ensure_builtin_providers};
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

    assert!(error.contains("Destroy dependent nodes first"));
    let provider = db
      .get_provider("provider-1")
      .expect("provider lookup should succeed")
      .expect("provider row should still exist");
    assert_eq!(provider.token_ref.as_deref(), Some(token_ref.as_str()));

    let _ = fs::remove_file(path);
  }
}