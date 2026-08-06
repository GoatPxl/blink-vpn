use std::io::Write;
use std::process::Command;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::backend::bootstrap::{
  Bootstrap, BootstrapOutputContract, BootstrapParams, ReadinessMarker,
  DEFAULT_INTERFACE_NAME, DEFAULT_WIREGUARD_PORT,
};

use super::{
  Account, ProviderAdapterError, ProviderDropletStatus, ProviderPlanPricing,
  ProvisionedDroplet,
};

const DOCKER_REGION: &str = "docker-local";
const DOCKER_IMAGE: &str = "ubuntu:24.04";
const DOCKER_INIT_SCRIPT_PATH: &str = "/usr/local/bin/goatstare-docker-init.sh";
const DOCKER_WIREGUARD_PUBLISH: &str = "127.0.0.1:51820:51820/udp";

#[derive(Clone, Copy)]
struct DockerPlanSpec {
  key: &'static str,
  label: &'static str,
  vcpus: u64,
  memory_mb: u64,
  disk_gb: u64,
  transfer_tb: f64,
  monthly_usd: f64,
  hourly_usd: f64,
}

const DOCKER_PLAN_SPECS: [DockerPlanSpec; 3] = [
  DockerPlanSpec {
    key: "docker-small",
    label: "Docker Small",
    vcpus: 1,
    memory_mb: 1024,
    disk_gb: 25,
    transfer_tb: 1.0,
    monthly_usd: 6.0,
    hourly_usd: 0.009,
  },
  DockerPlanSpec {
    key: "docker-medium",
    label: "Docker Medium",
    vcpus: 2,
    memory_mb: 4096,
    disk_gb: 80,
    transfer_tb: 2.0,
    monthly_usd: 24.0,
    hourly_usd: 0.036,
  },
  DockerPlanSpec {
    key: "docker-large",
    label: "Docker Large",
    vcpus: 4,
    memory_mb: 8192,
    disk_gb: 160,
    transfer_tb: 4.0,
    monthly_usd: 48.0,
    hourly_usd: 0.072,
  },
];

#[derive(Clone)]
pub struct DockerProviderAdapter;

impl DockerProviderAdapter {
  pub fn new() -> Self {
    Self
  }

  fn validate_faux_token(token: &str) -> Result<(), ProviderAdapterError> {
    let value = token.trim();
    if value.is_empty() {
      return Err(ProviderAdapterError::Unsupported {
        detail: "Docker credential is empty. Save a non-empty Docker provider token first.".to_string(),
      });
    }

    if !(value.starts_with("dkr_v1_") || value.starts_with("docker_")) {
      return Err(ProviderAdapterError::Unsupported {
        detail:
          "Docker provider token must start with 'dkr_v1_' or 'docker_' for this phase.".to_string(),
      });
    }

    Ok(())
  }

  fn run_docker(args: &[&str], action: &str) -> Result<String, ProviderAdapterError> {
    let output = Command::new("docker")
      .args(args)
      .output()
      .map_err(|error| ProviderAdapterError::Unsupported {
        detail: format!("Docker command failed for {action}: {error}"),
      })?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "Docker command failed for {action} (exit {:?}): {}",
          output.status.code(),
          if stderr.is_empty() {
            "unknown error".to_string()
          } else {
            stderr
          }
        ),
      });
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
  }

  fn ensure_daemon_available() -> Result<String, ProviderAdapterError> {
    Self::run_docker(&["info", "--format", "{{.ServerVersion}}"], "docker daemon availability")
  }

  fn now_epoch_seconds() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
      Ok(value) => value.as_secs().to_string(),
      Err(_) => "0".to_string(),
    }
  }

  fn plan_for_key(plan_key: &str) -> Option<DockerPlanSpec> {
    DOCKER_PLAN_SPECS
      .iter()
      .copied()
      .find(|spec| spec.key == plan_key)
  }

  fn container_name(base_name: &str, droplet_id: u64) -> String {
    let sanitized = base_name
      .chars()
      .map(|ch| {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
          ch
        } else {
          '-'
        }
      })
      .collect::<String>();

    let compact = sanitized.trim_matches('-');
    let name = if compact.is_empty() {
      "goatstare-node"
    } else {
      compact
    };

    format!("{name}-{droplet_id}")
  }

  fn docker_public_endpoint() -> &'static str {
    "127.0.0.1"
  }

  fn build_container_run_args(
    container_name: &str,
    droplet_label: &str,
    cpus: &str,
    memory: &str,
  ) -> Vec<String> {
    vec![
      "run".to_string(),
      "-d".to_string(),
      "--cap-add".to_string(),
      "NET_ADMIN".to_string(),
      "--sysctl".to_string(),
      "net.ipv4.ip_forward=1".to_string(),
      "-p".to_string(),
      DOCKER_WIREGUARD_PUBLISH.to_string(),
      "--name".to_string(),
      container_name.to_string(),
      "--label".to_string(),
      "goatstare.provider=docker".to_string(),
      "--label".to_string(),
      droplet_label.to_string(),
      "--cpus".to_string(),
      cpus.to_string(),
      "--memory".to_string(),
      memory.to_string(),
      DOCKER_IMAGE.to_string(),
      "sh".to_string(),
      "-c".to_string(),
      "while true; do sleep 3600; done".to_string(),
    ]
  }

  fn container_ids_for_droplet(droplet_id: u64) -> Result<Vec<String>, ProviderAdapterError> {
    let filter = format!("goatstare.droplet_id={droplet_id}");
    let output = Self::run_docker(
      &[
        "ps",
        "-a",
        "--filter",
        "label=goatstare.provider=docker",
        "--filter",
        &format!("label={filter}"),
        "--format",
        "{{.ID}}",
      ],
      "list containers by droplet id",
    )?;

    Ok(
      output
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect(),
    )
  }

  fn container_id_for_droplet(droplet_id: u64) -> Result<String, ProviderAdapterError> {
    let ids = Self::container_ids_for_droplet(droplet_id)?;
    ids
      .into_iter()
      .next()
      .ok_or_else(|| ProviderAdapterError::Unsupported {
        detail: format!(
          "Could not locate Docker container for droplet id {} during bootstrap readiness.",
          droplet_id
        ),
      })
  }

  fn container_runtime_status(container_id: &str) -> Result<String, ProviderAdapterError> {
    Self::run_docker(
      &["inspect", "--format", "{{.State.Status}}", container_id],
      "inspect Docker container runtime status",
    )
    .map(|value| value.trim().to_ascii_lowercase())
  }

  fn docker_exec(container_id: &str, script: &str, action: &str) -> Result<String, ProviderAdapterError> {
    // On Windows Docker hosts, `docker exec` can return empty stdout unless stdin stays attached.
    Self::run_docker(&["exec", "-i", container_id, "sh", "-c", script], action)
  }

  fn copy_container_file_to_host_text(
    container_id: &str,
    container_path: &str,
    action: &str,
  ) -> Result<String, ProviderAdapterError> {
    let mut host_path = std::env::temp_dir();
    let stamp = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_nanos())
      .unwrap_or(0);
    host_path.push(format!("goatstare-docker-copy-{stamp}.tmp"));

    let source = format!("{container_id}:{container_path}");
    let destination = host_path.to_string_lossy().to_string();

    let copy_result = Self::run_docker(&["cp", &source, &destination], action);
    if let Err(error) = copy_result {
      let _ = std::fs::remove_file(&host_path);
      return Err(error);
    }

    let content = std::fs::read_to_string(&host_path).map_err(|source| {
      ProviderAdapterError::Unsupported {
        detail: format!(
          "Docker copied '{}' to '{}' but host read failed: {}",
          container_path,
          host_path.display(),
          source
        ),
      }
    })?;

    let _ = std::fs::remove_file(&host_path);

    Ok(content)
  }

  fn copy_container_file_to_host_text_with_retries(
    container_id: &str,
    container_path: &str,
    action: &str,
    attempts: usize,
  ) -> Result<String, ProviderAdapterError> {
    let total_attempts = attempts.max(1);
    let mut last_error: Option<ProviderAdapterError> = None;

    for attempt in 0..total_attempts {
      match Self::copy_container_file_to_host_text(container_id, container_path, action) {
        Ok(content) => return Ok(content),
        Err(error) => {
          last_error = Some(error);
          if attempt + 1 < total_attempts {
            let delay_ms = 150 * (attempt as u64 + 1);
            std::thread::sleep(Duration::from_millis(delay_ms));
          }
        }
      }
    }

    Err(last_error.unwrap_or_else(|| ProviderAdapterError::Unsupported {
      detail: format!(
        "Docker copy from '{}' failed without returning a concrete error.",
        container_path
      ),
    }))
  }

  fn read_container_text_with_fallback(
    container_id: &str,
    container_path: &str,
    copy_action: &str,
    exec_action: &str,
  ) -> Result<String, ProviderAdapterError> {
    match Self::copy_container_file_to_host_text_with_retries(
      container_id,
      container_path,
      copy_action,
      5,
    ) {
      Ok(content) => Ok(content),
      Err(copy_error) => {
        let content = Self::docker_exec(
          container_id,
          &format!("set -eu; test -f '{container_path}'; cat '{container_path}';"),
          exec_action,
        )?;

        if content.trim().is_empty() {
          return Err(copy_error);
        }

        Ok(content)
      }
    }
  }

  fn copy_host_text_to_container(
    container_id: &str,
    content: &str,
    container_path: &str,
    action: &str,
  ) -> Result<(), ProviderAdapterError> {
    let mut host_path = std::env::temp_dir();
    let stamp = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_nanos())
      .unwrap_or(0);
    host_path.push(format!("goatstare-docker-copy-in-{stamp}.tmp"));

    std::fs::write(&host_path, content).map_err(|source| ProviderAdapterError::Unsupported {
      detail: format!(
        "Could not write temporary host file '{}' for Docker copy: {}",
        host_path.display(),
        source
      ),
    })?;

    let source = host_path.to_string_lossy().to_string();
    let destination = format!("{container_id}:{container_path}");
    let copy_result = Self::run_docker(&["cp", &source, &destination], action);

    let _ = std::fs::remove_file(&host_path);

    copy_result.map(|_| ())
  }

  fn stage_docker_init_script(
    container_id: &str,
    init_script: &str,
  ) -> Result<(), ProviderAdapterError> {
    if init_script.trim().is_empty() {
      return Err(ProviderAdapterError::Unsupported {
        detail: "Docker init payload was empty; bootstrap render must provide a non-empty init script."
          .to_string(),
      });
    }

    Self::copy_host_text_to_container(
      container_id,
      init_script,
      DOCKER_INIT_SCRIPT_PATH,
      "copy Docker init script into container",
    )?;

    let _ = Self::docker_exec(
      container_id,
      &format!("set -eu; chmod 700 {DOCKER_INIT_SCRIPT_PATH};"),
      "set Docker init script permissions",
    )?;

    Ok(())
  }

  fn generate_host_wireguard_keypair() -> Result<(String, String), ProviderAdapterError> {
    let private_out = Command::new("wg")
      .arg("genkey")
      .output()
      .map_err(|error| ProviderAdapterError::Unsupported {
        detail: format!(
          "Could not execute host WireGuard key generation via 'wg genkey': {}",
          error
        ),
      })?;

    if !private_out.status.success() {
      let stderr = String::from_utf8_lossy(&private_out.stderr).trim().to_string();
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "Host WireGuard key generation failed via 'wg genkey' (exit {:?}): {}",
          private_out.status.code(),
          if stderr.is_empty() {
            "unknown error".to_string()
          } else {
            stderr
          }
        ),
      });
    }

    let private_key = String::from_utf8_lossy(&private_out.stdout).trim().to_string();
    if private_key.is_empty() {
      return Err(ProviderAdapterError::Unsupported {
        detail: "Host WireGuard key generation produced an empty private key.".to_string(),
      });
    }

    let mut pub_cmd = Command::new("wg")
      .arg("pubkey")
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .spawn()
      .map_err(|error| ProviderAdapterError::Unsupported {
        detail: format!(
          "Could not execute host WireGuard public key derivation via 'wg pubkey': {}",
          error
        ),
      })?;

    {
      let stdin = pub_cmd
        .stdin
        .as_mut()
        .ok_or_else(|| ProviderAdapterError::Unsupported {
          detail: "Host WireGuard public key derivation could not open stdin pipe."
            .to_string(),
        })?;

      stdin
        .write_all(private_key.as_bytes())
        .map_err(|error| ProviderAdapterError::Unsupported {
          detail: format!(
            "Could not send private key to host WireGuard public key derivation: {}",
            error
          ),
        })?;
    }

    let public_out = pub_cmd
      .wait_with_output()
      .map_err(|error| ProviderAdapterError::Unsupported {
        detail: format!(
          "Host WireGuard public key derivation process failed while waiting for output: {}",
          error
        ),
      })?;

    if !public_out.status.success() {
      let stderr = String::from_utf8_lossy(&public_out.stderr).trim().to_string();
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "Host WireGuard public key derivation failed via 'wg pubkey' (exit {:?}): {}",
          public_out.status.code(),
          if stderr.is_empty() {
            "unknown error".to_string()
          } else {
            stderr
          }
        ),
      });
    }

    let public_key = String::from_utf8_lossy(&public_out.stdout).trim().to_string();
    if public_key.is_empty() {
      return Err(ProviderAdapterError::Unsupported {
        detail: "Host WireGuard public key derivation produced an empty public key.".to_string(),
      });
    }

    Ok((private_key, public_key))
  }

  fn resolve_docker_init_payload(init_script: &str) -> Result<String, ProviderAdapterError> {
    if !init_script.trim().is_empty() {
      return Ok(init_script.to_string());
    }

    Bootstrap::new()
      .render_docker_init(&BootstrapParams::default())
      .map(|artifact| artifact.cloud_init_user_data)
      .map_err(|source| ProviderAdapterError::Unsupported {
        detail: format!(
          "Could not render default Docker init payload for direct Docker adapter provisioning: {}",
          source
        ),
      })
  }

  fn initialize_wireguard_server_in_container(
    container_id: &str,
    readiness_marker_path: &str,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
    let (server_private_key, server_public_key) = Self::generate_host_wireguard_keypair()?;

    let _ = Self::docker_exec(
      container_id,
      "set -eu; mkdir -p /etc/wireguard;",
      "prepare Docker WireGuard directory before init script",
    )?;

    Self::copy_host_text_to_container(
      container_id,
      &format!("{}\n", server_private_key),
      "/etc/wireguard/server_private.key",
      "copy Docker server private key into container",
    )?;

    Self::copy_host_text_to_container(
      container_id,
      &format!("{}\n", server_public_key),
      "/etc/wireguard/server_public.key",
      "copy Docker server public key into container",
    )?;

    let execute_init_script = |action: &str| {
      Self::docker_exec(
        container_id,
        &format!(
          "set -eu; {script}; for attempt in 1 2 3 4 5; do if [ -f '{marker}' ]; then exit 0; fi; sleep 1; done; exit 62;",
          script = DOCKER_INIT_SCRIPT_PATH,
          marker = readiness_marker_path,
        ),
        action,
      )
    };

    let _ = execute_init_script("execute Docker init script")?;
    if server_public_key.len() < 40 {
      return Err(ProviderAdapterError::Unsupported {
        detail: "Docker init script returned an invalid WireGuard server public key."
          .to_string(),
      });
    }

    let marker_script = format!(
      "set -eu; mkdir -p \"$(dirname '{readiness_marker_path}')\"; printf '{{\"status\":\"ready\",\"server_public_key\":\"%s\",\"listen_port\":%d}}\\n' '{server_public_key}' {listen_port} > '{readiness_marker_path}';",
      readiness_marker_path = readiness_marker_path,
      server_public_key = server_public_key,
      listen_port = DEFAULT_WIREGUARD_PORT,
    );

    let _ = Self::docker_exec(
      container_id,
      &marker_script,
      "write Docker bootstrap readiness marker",
    )?;

    let marker_content = Self::copy_container_file_to_host_text_with_retries(
      container_id,
      readiness_marker_path,
      "copy Docker readiness marker from container",
      5,
    )?;

    let marker: ReadinessMarker = serde_json::from_str(&marker_content).map_err(|source| {
      ProviderAdapterError::Unsupported {
        detail: format!(
          "Docker bootstrap readiness marker JSON was invalid at '{}': {}",
          readiness_marker_path, source
        ),
      }
    })?;

    Ok(BootstrapOutputContract {
      server_endpoint_ip: String::new(),
      server_public_key: marker.server_public_key,
      listen_port: marker.listen_port,
    })
  }

  fn build_client_peer_update_script(
    client_public_key: &str,
    client_address_cidr: &str,
    interface_name: &str,
  ) -> Result<String, ProviderAdapterError> {
    let public_key = client_public_key.trim();
    let client_cidr = client_address_cidr.trim();
    let iface = interface_name.trim();

    if public_key.is_empty() {
      return Err(ProviderAdapterError::Unsupported {
        detail: "Docker client peer update needs a non-empty client public key.".to_string(),
      });
    }

    if client_cidr.is_empty() {
      return Err(ProviderAdapterError::Unsupported {
        detail: "Docker client peer update needs a non-empty client CIDR.".to_string(),
      });
    }

    if iface.is_empty() {
      return Err(ProviderAdapterError::Unsupported {
        detail: "Docker client peer update needs a non-empty interface name.".to_string(),
      });
    }

    Ok(format!(
      "set -eu; mkdir -p /etc/wireguard /var/lib/goatstare; fragment=$(mktemp /tmp/goatstare-peer-XXXXXX.conf); printf '[Peer]\nPublicKey = {public_key}\nAllowedIPs = {client_cidr}\n' > \"$fragment\"; cat \"$fragment\" >> /etc/wireguard/{iface}.conf; cp \"$fragment\" /var/lib/goatstare/{iface}-client-peer.conf; rm -f \"$fragment\"; ip link show {iface} >/dev/null 2>&1; wg set {iface} peer {public_key} allowed-ips {client_cidr};"
    ))
  }

  pub fn wire_client_peer(
    droplet_id: u64,
    client_public_key: &str,
    client_address_cidr: &str,
  ) -> Result<(), ProviderAdapterError> {
    let container_id = Self::container_id_for_droplet(droplet_id)?;
    let script = Self::build_client_peer_update_script(
      client_public_key,
      client_address_cidr,
      DEFAULT_INTERFACE_NAME,
    )?;

    let _ = Self::docker_exec(
      &container_id,
      &script,
      "apply WireGuard client peer to Docker server container",
    )?;

    Ok(())
  }
}

impl super::ProviderAdapter for DockerProviderAdapter {
  fn provider_name(&self) -> &'static str {
    "docker"
  }

  fn default_regions(&self) -> &'static [&'static str] {
    &[DOCKER_REGION]
  }

  fn validate_token(&self, token: &str) -> Result<Account, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let daemon_version = Self::ensure_daemon_available()?;

    Ok(Account {
      uuid: "docker-local".to_string(),
      email: "docker-local@host".to_string(),
      status: format!("daemon:{daemon_version}"),
    })
  }

  fn list_regions(&self, token: &str) -> Result<Vec<String>, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    Ok(vec![DOCKER_REGION.to_string()])
  }

  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError> {
    Self::validate_faux_token(token)?;

    if region.trim() != DOCKER_REGION {
      return Ok(Vec::new());
    }

    let source_timestamp = Self::now_epoch_seconds();

    Ok(
      DOCKER_PLAN_SPECS
        .iter()
        .map(|plan| ProviderPlanPricing {
          plan_key: plan.key.to_string(),
          plan_label: plan.label.to_string(),
          size_slug: plan.key.to_string(),
          region: DOCKER_REGION.to_string(),
          memory_mb: plan.memory_mb,
          vcpus: plan.vcpus,
          disk_gb: plan.disk_gb,
          transfer_tb: plan.transfer_tb,
          transfer_note: format!(
            "Synthetic plan estimate for local Docker runtime: up to {:.2} TB transfer equivalent.",
            plan.transfer_tb
          ),
          monthly_usd: plan.monthly_usd,
          hourly_usd: plan.hourly_usd,
          estimated: true,
          source_timestamp: source_timestamp.clone(),
        })
        .collect(),
    )
  }

  fn create_droplet(
    &self,
    token: &str,
    droplet_name: &str,
    _region: &str,
    size_slug: &str,
    cloud_init_user_data: &str,
  ) -> Result<ProvisionedDroplet, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    let plan = Self::plan_for_key(size_slug).ok_or_else(|| ProviderAdapterError::Unsupported {
      detail: format!("Unsupported Docker plan '{size_slug}'. Expected docker-small, docker-medium, or docker-large."),
    })?;

    let droplet_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
      Ok(value) => value.as_millis() as u64,
      Err(_) => 1,
    };

    let container_name = Self::container_name(droplet_name, droplet_id);
    let droplet_label = format!("goatstare.droplet_id={droplet_id}");
    let cpus = plan.vcpus.to_string();
    let memory = format!("{}m", plan.memory_mb);

    let run_args = Self::build_container_run_args(&container_name, &droplet_label, &cpus, &memory);
    let run_arg_refs = run_args.iter().map(String::as_str).collect::<Vec<_>>();
    let container_id = Self::run_docker(&run_arg_refs, "create Docker compute container").map_err(|source| {
      match source {
        ProviderAdapterError::Unsupported { detail } => {
          let lower = detail.to_ascii_lowercase();
          if lower.contains("port is already allocated")
            || (lower.contains("51820") && lower.contains("bind"))
          {
            ProviderAdapterError::Unsupported {
              detail: format!(
                "{} Port 51820/udp on localhost is already in use. Stop the conflicting service/container, then retry Docker node provisioning.",
                detail
              ),
            }
          } else {
            ProviderAdapterError::Unsupported { detail }
          }
        }
        other => other,
      }
    })?;

    let docker_init_payload = Self::resolve_docker_init_payload(cloud_init_user_data)?;
    Self::stage_docker_init_script(&container_id, &docker_init_payload)?;

    let public_ipv4 = Self::docker_public_endpoint().to_string();

    Ok(ProvisionedDroplet {
      droplet_id,
      public_ipv4,
    })
  }

  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    droplet_id: u64,
    endpoint_ip: &str,
    readiness_marker_path: &str,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    let container_id = Self::container_id_for_droplet(droplet_id)?;
    let mut contract =
      Self::initialize_wireguard_server_in_container(&container_id, readiness_marker_path)?;
    contract.server_endpoint_ip = if endpoint_ip.trim().is_empty() {
      Self::docker_public_endpoint().to_string()
    } else {
      endpoint_ip.to_string()
    };

    Ok(contract)
  }

  fn droplet_exists(&self, token: &str, droplet_id: u64) -> Result<bool, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    Ok(!Self::container_ids_for_droplet(droplet_id)?.is_empty())
  }

  fn droplet_status(&self, token: &str, droplet_id: u64) -> Result<ProviderDropletStatus, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    let ids = Self::container_ids_for_droplet(droplet_id)?;
    if ids.is_empty() {
      return Ok(ProviderDropletStatus::Deleted);
    }

    let mut statuses = Vec::with_capacity(ids.len());
    for container_id in ids {
      statuses.push(Self::container_runtime_status(&container_id)?);
    }

    if statuses.iter().any(|status| status == "running") {
      return Ok(ProviderDropletStatus::Active);
    }

    if statuses.iter().any(|status| status == "created" || status == "restarting") {
      return Ok(ProviderDropletStatus::New);
    }

    if statuses.iter().any(|status| status == "removing") {
      return Ok(ProviderDropletStatus::Archive);
    }

    if statuses
      .iter()
      .all(|status| status == "paused" || status == "exited" || status == "dead")
    {
      return Ok(ProviderDropletStatus::Off);
    }

    Ok(ProviderDropletStatus::Unknown)
  }

  fn delete_droplet(&self, token: &str, droplet_id: u64) -> Result<(), ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    let ids = Self::container_ids_for_droplet(droplet_id)?;
    if ids.is_empty() {
      return Ok(());
    }

    let mut args = vec!["rm", "-f"];
    let dynamic_ids: Vec<String> = ids;
    for id in &dynamic_ids {
      args.push(id.as_str());
    }

    let _ = Self::run_docker(&args, "remove Docker compute container")?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::{
    DockerProviderAdapter, DOCKER_IMAGE, DOCKER_REGION, DOCKER_WIREGUARD_PUBLISH,
  };
  use crate::backend::bootstrap::{Bootstrap, BootstrapParams, READINESS_MARKER_PATH};
  use crate::backend::provider::ProviderAdapter;
  use std::time::{SystemTime, UNIX_EPOCH};

  #[test]
  fn docker_provider_uses_ubuntu_24_base_image() {
    assert_eq!(DOCKER_IMAGE, "ubuntu:24.04");

    let install_script = Bootstrap::new()
      .render_docker_init(&BootstrapParams::default())
      .expect("docker init render should succeed")
      .cloud_init_user_data;
    assert!(install_script.contains("apt-get update"));
    assert!(install_script.contains("apt-get install -y --no-install-recommends wireguard-tools iproute2 iptables"));
    assert!(!install_script.contains("apk add"));
  }

  #[test]
  fn lists_static_region_without_docker_daemon() {
    let adapter = DockerProviderAdapter::new();

    let result = adapter.list_regions("docker_dev_token").expect("docker region catalog should be static");

    assert_eq!(result, vec![DOCKER_REGION.to_string()]);
  }

  #[test]
  fn lists_static_plan_pricing_without_docker_daemon() {
    let adapter = DockerProviderAdapter::new();

    let result = adapter
      .list_plan_pricing("docker_dev_token", DOCKER_REGION)
      .expect("docker plan catalog should be static");

    assert_eq!(result.len(), 3);
    assert_eq!(result[0].region, DOCKER_REGION);
    assert!(result.iter().all(|plan| plan.estimated));
  }

  #[test]
  #[ignore = "Requires local Docker daemon and network package install inside test container"]
  fn real_docker_create_init_and_delete_roundtrip() {
    let adapter = DockerProviderAdapter::new();
    let token = "dkr_v1_local_dev_mock_token_2026";
    let suffix = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_secs())
      .unwrap_or(0);
    let node_name = format!("docker-real-roundtrip-{suffix}");

    let provisioned = adapter
      .create_droplet(token, &node_name, DOCKER_REGION, "docker-small", "")
      .expect("docker create should succeed with a local daemon");

    let contract_result = adapter.wait_for_bootstrap_ready(
      token,
      provisioned.droplet_id,
      &provisioned.public_ipv4,
      READINESS_MARKER_PATH,
    );

    match contract_result {
      Ok(contract) => {
        assert_eq!(contract.server_endpoint_ip, provisioned.public_ipv4);
        assert_eq!(contract.listen_port, 51820);
        assert!(contract.server_public_key.len() >= 40);

        let container_id = DockerProviderAdapter::container_id_for_droplet(provisioned.droplet_id)
          .expect("container id should resolve for provisioned droplet");
        let server_conf = DockerProviderAdapter::copy_container_file_to_host_text(
          &container_id,
          "/etc/wireguard/wg0.conf",
          "copy WireGuard server config from Docker container",
        )
        .expect("server config should be readable from container");

        assert!(server_conf.contains("[Interface]"));
        assert!(server_conf.contains("Address = 10.66.0.1/24"));
        assert!(server_conf.contains("ListenPort = 51820"));
        assert!(server_conf.contains("PrivateKey = "));
        assert!(server_conf.contains("SaveConfig = false"));

        DockerProviderAdapter::docker_exec(
          &container_id,
          "set -eu; ip link show wg0 >/dev/null 2>&1; ip -o link show wg0 | grep -q 'UP'; wg show wg0 >/dev/null 2>&1;",
          "validate WireGuard interface is active after Docker init",
        )
        .expect("wg0 should exist and be active after bootstrap");

        let marker_content = DockerProviderAdapter::copy_container_file_to_host_text(
          &container_id,
          READINESS_MARKER_PATH,
          "copy Docker readiness marker from container",
        )
        .expect("readiness marker should be readable from container");
        assert!(marker_content.contains("\"status\":\"ready\""));
        assert!(marker_content.contains("\"listen_port\":51820"));
        assert!(marker_content.contains(&contract.server_public_key));

        let exists = adapter
          .droplet_exists(token, provisioned.droplet_id)
          .expect("droplet existence check should succeed");
        assert!(exists);
      }
      Err(error) => {
        let _ = adapter.delete_droplet(token, provisioned.droplet_id);
        panic!(
          "docker init script should produce a readiness contract: {:?}",
          error
        );
      }
    }

    adapter
      .delete_droplet(token, provisioned.droplet_id)
      .expect("docker cleanup should succeed");

    let exists_after = adapter
      .droplet_exists(token, provisioned.droplet_id)
      .expect("post-delete existence check should succeed");
    assert!(!exists_after);
  }

  #[test]
  #[ignore = "Requires local Docker daemon and WireGuard tooling inside the test container"]
  fn real_docker_server_config_accepts_client_peer_roundtrip() {
    let adapter = DockerProviderAdapter::new();
    let token = "dkr_v1_local_dev_mock_token_2026";
    let suffix = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_secs())
      .unwrap_or(0);
    let node_name = format!("docker-peer-roundtrip-{suffix}");

    let provisioned = adapter
      .create_droplet(token, &node_name, DOCKER_REGION, "docker-small", "")
      .expect("docker create should succeed with a local daemon");

    let contract = adapter
      .wait_for_bootstrap_ready(
        token,
        provisioned.droplet_id,
        &provisioned.public_ipv4,
        READINESS_MARKER_PATH,
      )
      .expect("bootstrap contract should be available");

    DockerProviderAdapter::wire_client_peer(
      provisioned.droplet_id,
      "client-pub-key",
      "10.66.0.2/32",
    )
    .expect("client peer wiring should succeed");

    let container_id = DockerProviderAdapter::container_id_for_droplet(provisioned.droplet_id)
      .expect("container id should resolve for provisioned droplet");
    let _server_conf = DockerProviderAdapter::docker_exec(
      &container_id,
      "cat /etc/wireguard/wg0.conf",
      "read WireGuard server config after client peer wiring",
    )
    .expect("server config should be readable from container");

    DockerProviderAdapter::docker_exec(
      &container_id,
      "set -eu; ip link show wg0 >/dev/null 2>&1; wg show wg0 peers | grep -qx 'client-pub-key';",
      "validate client peer is active on Docker WireGuard interface",
    )
    .expect("client peer should be present on wg0");

    assert_eq!(contract.server_endpoint_ip, provisioned.public_ipv4);

    adapter
      .delete_droplet(token, provisioned.droplet_id)
      .expect("docker cleanup should succeed");
  }

  #[test]
  fn client_peer_update_script_mentions_peer_and_allowed_ips() {
    let script = DockerProviderAdapter::build_client_peer_update_script(
      "client-pub-key",
      "10.66.0.2/32",
      "wg0",
    )
    .expect("peer update script should build");

    assert!(script.contains("wg set wg0 peer client-pub-key allowed-ips 10.66.0.2/32"));
    assert!(script.contains("[Peer]"));
    assert!(script.contains("AllowedIPs = 10.66.0.2/32"));
    assert!(script.contains("/etc/wireguard/wg0.conf"));
    assert!(!script.contains("if command -v wg"));
  }

  #[test]
  fn docker_container_run_args_publish_wireguard_udp_on_localhost() {
    let args = DockerProviderAdapter::build_container_run_args(
      "demo-container",
      "goatstare.droplet_id=42",
      "1",
      "1024m",
    );

    assert!(args.contains(&"-p".to_string()));
    assert!(args.contains(&DOCKER_WIREGUARD_PUBLISH.to_string()));
  }

  #[test]
  fn docker_public_endpoint_is_localhost() {
    assert_eq!(DockerProviderAdapter::docker_public_endpoint(), "127.0.0.1");
  }

}
