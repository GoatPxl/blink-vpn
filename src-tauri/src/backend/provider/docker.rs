use std::io::Write;
use std::process::Command;
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::bootstrap::{
  BootstrapOutputContract, DEFAULT_CLIENT_CIDR, DEFAULT_INTERFACE_NAME,
  DEFAULT_SERVER_INTERFACE_CIDR, DEFAULT_WIREGUARD_PORT,
};

use super::{
  Account, ProviderAdapterError, ProviderDropletStatus, ProviderPlanPricing,
  ProvisionedDroplet,
};

const DOCKER_REGION: &str = "docker-local";
const DOCKER_IMAGE: &str = "alpine:3.20";

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

  fn initialize_wireguard_server_in_container(
    container_id: &str,
    readiness_marker_path: &str,
  ) -> Result<String, ProviderAdapterError> {
    let (server_private_key, server_public_key) = Self::generate_host_wireguard_keypair()?;

    let _ = Self::docker_exec(
      container_id,
      "set -eu; if ! command -v wg >/dev/null 2>&1; then apk add --no-cache wireguard-tools >/dev/null 2>&1; fi; mkdir -p /etc/wireguard; umask 077;",
      "prepare WireGuard tooling and directory in Docker container",
    )?;

    Self::copy_host_text_to_container(
      container_id,
      &format!("{}\n", server_private_key),
      "/tmp/goatstare-server-private.key",
      "copy WireGuard server private key to Docker container temp file",
    )?;

    Self::copy_host_text_to_container(
      container_id,
      &format!("{}\n", server_public_key),
      "/tmp/goatstare-server-public.key",
      "copy WireGuard server public key to Docker container temp file",
    )?;

    let _ = Self::docker_exec(
      container_id,
      "set -eu; mkdir -p /etc/wireguard; mv /tmp/goatstare-server-private.key /etc/wireguard/server_private.key; mv /tmp/goatstare-server-public.key /etc/wireguard/server_public.key;",
      "place WireGuard key files in Docker container",
    )?;

    let server_conf = format!(
      "[Interface]\nAddress = {server_cidr}\nListenPort = {listen_port}\nPrivateKey = {private_key}\nSaveConfig = false\n\nPostUp = iptables -A FORWARD -i {interface_name} -j ACCEPT; iptables -A FORWARD -o {interface_name} -j ACCEPT; iptables -t nat -A POSTROUTING -s {client_cidr} -j MASQUERADE\nPostDown = iptables -D FORWARD -i {interface_name} -j ACCEPT; iptables -D FORWARD -o {interface_name} -j ACCEPT; iptables -t nat -D POSTROUTING -s {client_cidr} -j MASQUERADE\n",
      server_cidr = DEFAULT_SERVER_INTERFACE_CIDR,
      listen_port = DEFAULT_WIREGUARD_PORT,
      private_key = server_private_key,
      interface_name = DEFAULT_INTERFACE_NAME,
      client_cidr = DEFAULT_CLIENT_CIDR,
    );

    Self::copy_host_text_to_container(
      container_id,
      &server_conf,
      &format!("/tmp/goatstare-{}.conf", DEFAULT_INTERFACE_NAME),
      "copy WireGuard server config to Docker container temp file",
    )?;

    let _ = Self::docker_exec(
      container_id,
      &format!(
        "set -eu; mkdir -p /etc/wireguard; mv /tmp/goatstare-{iface}.conf /etc/wireguard/{iface}.conf;",
        iface = DEFAULT_INTERFACE_NAME
      ),
      "place WireGuard server config in Docker container",
    )?;

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

    if server_public_key.len() < 40 {
      return Err(ProviderAdapterError::Unsupported {
        detail: "Docker init script returned an invalid WireGuard server public key."
          .to_string(),
      });
    }

    Ok(server_public_key)
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
      "set -eu; mkdir -p /etc/wireguard /var/lib/goatstare; fragment=$(mktemp /tmp/goatstare-peer-XXXXXX.conf); printf '[Peer]\nPublicKey = {public_key}\nAllowedIPs = {client_cidr}\n' > \"$fragment\"; cat \"$fragment\" >> /etc/wireguard/{iface}.conf; cp \"$fragment\" /var/lib/goatstare/{iface}-client-peer.conf; rm -f \"$fragment\"; if command -v wg >/dev/null 2>&1 && ip link show {iface} >/dev/null 2>&1; then wg set {iface} peer {public_key} allowed-ips {client_cidr}; fi; true"
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
    _cloud_init_user_data: &str,
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

    let _container_id = Self::run_docker(
      &[
        "run",
        "-d",
        "--name",
        &container_name,
        "--label",
        "goatstare.provider=docker",
        "--label",
        &droplet_label,
        "--cpus",
        &cpus,
        "--memory",
        &memory,
        DOCKER_IMAGE,
        "sh",
        "-c",
        "while true; do sleep 3600; done",
      ],
      "create Docker compute container",
    )?;

    let endpoint_ip = Self::run_docker(
      &[
        "inspect",
        "--format",
        "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
        &container_name,
      ],
      "inspect Docker container endpoint",
    )?;

    let public_ipv4 = if endpoint_ip.trim().is_empty() {
      "127.0.0.1".to_string()
    } else {
      endpoint_ip
    };

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
    let server_public_key =
      Self::initialize_wireguard_server_in_container(&container_id, readiness_marker_path)?;

    Ok(BootstrapOutputContract {
      server_endpoint_ip: endpoint_ip.to_string(),
      server_public_key,
      listen_port: DEFAULT_WIREGUARD_PORT,
    })
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
  use super::{DockerProviderAdapter, DOCKER_REGION};
  use crate::backend::bootstrap::READINESS_MARKER_PATH;
  use std::time::{SystemTime, UNIX_EPOCH};
  use crate::backend::provider::ProviderAdapter;

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
  }

}
