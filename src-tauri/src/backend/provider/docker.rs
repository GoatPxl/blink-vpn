use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::bootstrap::{BootstrapOutputContract, DEFAULT_WIREGUARD_PORT};

use super::{Account, ProviderAdapterError, ProviderPlanPricing, ProvisionedDroplet};

const DOCKER_REGION: &str = "docker-local";
const DOCKER_PUBLIC_KEY_PLACEHOLDER: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
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
    let _ = Self::ensure_daemon_available()?;
    Ok(vec![DOCKER_REGION.to_string()])
  }

  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

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
    _droplet_id: u64,
    endpoint_ip: &str,
    _readiness_marker_path: &str,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    Ok(BootstrapOutputContract {
      server_endpoint_ip: endpoint_ip.to_string(),
      server_public_key: DOCKER_PUBLIC_KEY_PLACEHOLDER.to_string(),
      listen_port: DEFAULT_WIREGUARD_PORT,
    })
  }

  fn droplet_exists(&self, token: &str, droplet_id: u64) -> Result<bool, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    Ok(!Self::container_ids_for_droplet(droplet_id)?.is_empty())
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
