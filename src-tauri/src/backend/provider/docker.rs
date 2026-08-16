use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::backend::bootstrap::{BootstrapOutputContract, READINESS_MARKER_PATH};

use super::{
  Account, BootstrapWaitRequest, CredentialField, InitDelivery, LEGACY_SINGLE_CREDENTIAL_KEY,
  ProviderAdapterError, ProviderCredential, ProviderNodeStatus, ProviderPlanPricing, ProviderRegion,
  ProvisionedNode,
};

const DOCKER_REGION: &str = "docker-local";
const DOCKER_IMAGE: &str = "ubuntu:24.04";

/// The rendered payload is handed to the container as an environment variable at create time and
/// written to this path by the entrypoint.
///
/// This mirrors how a cloud provider accepts user-data: a string supplied with the create call,
/// never a file pushed in afterwards. It was previously a read-only bind mount, but a host bind
/// mount only works when the CLI can translate Windows paths for the daemon — with
/// `DOCKER_HOST=tcp://...` it cannot, and provisioning failed with `invalid volume specification`.
/// An environment variable has no such coupling and works against any daemon connection.
const DOCKER_INIT_PAYLOAD_ENV: &str = "GOATSTARE_INIT_PAYLOAD";
const DOCKER_INIT_SCRIPT_PATH: &str = "/goatstare-init.sh";

/// Container-internal WireGuard port. The host-side port is allocated per node by Docker.
const DOCKER_CONTAINER_WIREGUARD_PORT: u16 = 51820;

/// Publish on every interface of the daemon's host, not just its loopback.
///
/// Binding `127.0.0.1` binds loopback *in the daemon's own network namespace*. When the daemon
/// runs inside WSL2 — reached over `DOCKER_HOST=tcp://...` rather than through Docker Desktop —
/// that loopback is WSL2's, not Windows'. WSL2 forwards TCP to the Windows localhost but not UDP,
/// so every WireGuard handshake sent to `127.0.0.1:<port>` was silently discarded: the container
/// was healthy, its peer configured, and its iptables INPUT counters stayed at zero.
const DOCKER_WIREGUARD_PUBLISH: &str = "0.0.0.0::51820/udp";

/// Explicit override for the address the client should dial to reach published containers.
const DOCKER_ENDPOINT_HOST_ENV: &str = "GOATSTARE_DOCKER_ENDPOINT_HOST";

/// Readiness is observed through Docker's own healthcheck, which Docker runs inside the container
/// and reports as provider metadata via `docker inspect`. The app never execs anything itself.
const DOCKER_HEALTH_INTERVAL: &str = "2s";
const DOCKER_HEALTH_TIMEOUT: &str = "5s";
const DOCKER_HEALTH_RETRIES: &str = "3";

/// Failures during the start period do not mark the container unhealthy.
///
/// This must cover the whole bootstrap, not just container startup: on a bare base image the
/// script spends ~20s in `apt-get install` before the marker can possibly exist. With a short
/// start period the healthcheck flipped to `unhealthy` while bootstrap was still legitimately
/// running, and provisioning aborted on a node that would have come up fine. Real failures are
/// still caught immediately, because a failing script exits the container.
const DOCKER_HEALTH_START_PERIOD: &str = "180s";

/// Generous enough to cover an `apt-get install` on a bare base image. Kept in step with
/// `DOCKER_HEALTH_START_PERIOD`.
const DOCKER_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(180);

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
    // The stored credential is a field set; Docker uses the single-field form, which is also what
    // a credential saved before field sets existed parses as.
    let credential = ProviderCredential::parse(token);
    let value = credential.field(LEGACY_SINGLE_CREDENTIAL_KEY).unwrap_or("").trim();
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

  /// A Docker container name derived from the user's node name, with a random suffix.
  ///
  /// The suffix is what keeps two nodes the user happened to call the same thing from colliding.
  /// `create_node` force-removes any container already holding the exact name, so without it,
  /// provisioning a second "Primary Node" would destroy the first — a healthy, running node.
  ///
  /// It used to be the synthetic node id. That id no longer exists: the node id is the container id
  /// Docker itself returns, so the suffix only has to be unique, not meaningful.
  fn container_name(base_name: &str) -> String {
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

    let suffix = Uuid::new_v4().simple().to_string();
    format!("{name}-{}", &suffix[..8])
  }

  /// Address the *client* should dial to reach a published container port.
  ///
  /// This is not always `127.0.0.1`. It depends on where the daemon runs relative to this
  /// machine, and getting it wrong produces a container that looks perfectly healthy while every
  /// packet sent to it disappears.
  ///
  /// Resolved in order:
  /// 1. `GOATSTARE_DOCKER_ENDPOINT_HOST`, for setups this cannot infer.
  /// 2. A non-loopback host in `DOCKER_HOST`, which is the daemon's address by definition.
  /// 3. The WSL2 VM's address, when the daemon reports a WSL2 kernel. Published ports live on
  ///    that VM's interfaces, and Windows can reach them there but not via its own loopback.
  /// 4. `127.0.0.1`, correct for a daemon that publishes onto this machine directly.
  fn docker_endpoint_host() -> String {
    if let Some(explicit) = std::env::var(DOCKER_ENDPOINT_HOST_ENV)
      .ok()
      .map(|value| value.trim().to_string())
      .filter(|value| !value.is_empty())
    {
      return explicit;
    }

    if let Some(host) = Self::docker_host_from_env() {
      return host;
    }

    if let Some(host) = Self::wsl_daemon_host() {
      return host;
    }

    "127.0.0.1".to_string()
  }

  /// Host portion of a `tcp://host:port` DOCKER_HOST, when it is not loopback.
  fn docker_host_from_env() -> Option<String> {
    let raw = std::env::var("DOCKER_HOST").ok()?;
    let without_scheme = raw.split("://").nth(1)?;
    let host = without_scheme.split(':').next()?.trim();

    if host.is_empty() || host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()) {
      return None;
    }

    Some(host.to_string())
  }

  /// The WSL2 VM's IPv4 address, if the daemon is running inside WSL2.
  ///
  /// The address is assigned per WSL2 boot, so it is resolved at provision time and stored with
  /// the node rather than assumed to be stable.
  fn wsl_daemon_host() -> Option<String> {
    let kernel = Self::run_docker(
      &["info", "--format", "{{.KernelVersion}}"],
      "read Docker daemon kernel version",
    )
    .ok()?;

    if !kernel.to_ascii_lowercase().contains("microsoft-standard-wsl") {
      return None;
    }

    let output = Command::new("wsl.exe").args(["hostname", "-I"]).output().ok()?;
    if !output.status.success() {
      return None;
    }

    String::from_utf8_lossy(&output.stdout)
      .split_whitespace()
      .find_map(|candidate| candidate.parse::<std::net::Ipv4Addr>().ok())
      .map(|address| address.to_string())
  }

  /// Shell run by the container entrypoint: materialise the payload, run it, then idle.
  ///
  /// The payload arrives as an environment variable, so the container writes its own copy exactly
  /// as cloud-init writes user-data to disk before running it. `printf '%s'` is used rather than
  /// `echo` so nothing in the payload is interpreted, and the keepalive lives here rather than in
  /// the shared script, which must terminate normally under cloud-init.
  fn build_entrypoint_command() -> String {
    format!(
      "set -e; printf '%s' \"${DOCKER_INIT_PAYLOAD_ENV}\" > {DOCKER_INIT_SCRIPT_PATH}; chmod 700 {DOCKER_INIT_SCRIPT_PATH}; {DOCKER_INIT_SCRIPT_PATH}; exec tail -f /dev/null"
    )
  }

  /// Build the `docker run` invocation that delivers and executes the bootstrap payload.
  ///
  /// Three things make this a faithful emulation of a cloud provider handing over user-data:
  ///
  /// - the payload is supplied with the create call, not pushed in later via `docker exec`;
  /// - the script runs as the entrypoint, so the container's own lifecycle reflects bootstrap
  ///   success or failure, exactly as a failed cloud-init leaves an unusable droplet;
  /// - the host port is left for Docker to allocate, so nodes do not contend for one fixed port.
  fn build_container_run_args(
    container_name: &str,
    cpus: &str,
    memory: &str,
    init_payload: &str,
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
      "--cpus".to_string(),
      cpus.to_string(),
      "--memory".to_string(),
      memory.to_string(),
      "-e".to_string(),
      format!("{DOCKER_INIT_PAYLOAD_ENV}={init_payload}"),
      "--health-cmd".to_string(),
      format!("test -f {}", READINESS_MARKER_PATH),
      "--health-interval".to_string(),
      DOCKER_HEALTH_INTERVAL.to_string(),
      "--health-timeout".to_string(),
      DOCKER_HEALTH_TIMEOUT.to_string(),
      "--health-retries".to_string(),
      DOCKER_HEALTH_RETRIES.to_string(),
      "--health-start-period".to_string(),
      DOCKER_HEALTH_START_PERIOD.to_string(),
      "--entrypoint".to_string(),
      "/bin/bash".to_string(),
      DOCKER_IMAGE.to_string(),
      "-c".to_string(),
      Self::build_entrypoint_command(),
    ]
  }

  /// Ask Docker which host port it allocated for the container's WireGuard port.
  ///
  /// This is provider metadata, the Docker analogue of DigitalOcean reporting a droplet's public
  /// IPv4 — not an inspection of anything inside the server.
  fn resolve_published_wireguard_port(
    container_id: &str,
  ) -> Result<u16, ProviderAdapterError> {
    let raw = Self::run_docker(
      &[
        "port",
        container_id,
        &format!("{DOCKER_CONTAINER_WIREGUARD_PORT}/udp"),
      ],
      "resolve published WireGuard host port for Docker container",
    )?;

    // `docker port` prints one mapping per line, e.g. `127.0.0.1:49154`.
    raw
      .lines()
      .filter_map(|line| line.trim().rsplit_once(':'))
      .filter_map(|(_, port)| port.trim().parse::<u16>().ok())
      .find(|port| *port != 0)
      .ok_or_else(|| ProviderAdapterError::Unsupported {
        detail: format!(
          "Docker did not report a published host port for {DOCKER_CONTAINER_WIREGUARD_PORT}/udp (got '{raw}')."
        ),
      })
  }

  /// Whether a container with this id still exists, running or not.
  ///
  /// `docker ps --filter id=` matches on id prefix, and the node id is the full 64-character id
  /// Docker returned at create, so this cannot match a different container.
  fn container_exists(container_id: &str) -> Result<bool, ProviderAdapterError> {
    let output = Self::run_docker(
      &[
        "ps",
        "-a",
        "--filter",
        &format!("id={container_id}"),
        "--format",
        "{{.ID}}",
      ],
      "look up Docker container by id",
    )?;

    Ok(output.lines().any(|line| !line.trim().is_empty()))
  }

  fn container_runtime_status(container_id: &str) -> Result<String, ProviderAdapterError> {
    Self::run_docker(
      &["inspect", "--format", "{{.State.Status}}", container_id],
      "inspect Docker container runtime status",
    )
    .map(|value| value.trim().to_ascii_lowercase())
  }

  fn remove_containers(container_ids: &[String], action: &str) -> Result<(), ProviderAdapterError> {
    if container_ids.is_empty() {
      return Ok(());
    }

    let mut args = vec!["rm", "-f"];
    for id in container_ids {
      args.push(id.as_str());
    }

    let _ = Self::run_docker(&args, action)?;
    Ok(())
  }

  fn remove_container_by_exact_name(container_name: &str) -> Result<(), ProviderAdapterError> {
    let output = Self::run_docker(
      &[
        "ps",
        "-a",
        "--filter",
        &format!("name=^/{container_name}$"),
        "--format",
        "{{.ID}}",
      ],
      "find Docker container by exact name",
    )?;

    let ids = output
      .lines()
      .map(|line| line.trim().to_string())
      .filter(|line| !line.is_empty())
      .collect::<Vec<_>>();

    Self::remove_containers(&ids, "remove stale Docker container by exact name")
  }

  // Container inspection helpers below are test-only instrumentation.
  //
  // They are gated so they cannot drift back into the provisioning path, which must never exec
  // into or copy out of a provisioned node. Tests use them to verify container state after the
  // fact, which is observation rather than provisioning.

  #[cfg(test)]
  fn docker_exec(container_id: &str, script: &str, action: &str) -> Result<String, ProviderAdapterError> {
    // On Windows Docker hosts, `docker exec` can return empty stdout unless stdin stays attached.
    Self::run_docker(&["exec", "-i", container_id, "sh", "-c", script], action)
  }

  #[cfg(test)]
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

  #[cfg(test)]
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

  #[cfg(test)]
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

  /// The caller must supply a fully-rendered payload.
  ///
  /// This used to fall back to `render_docker_init(&BootstrapParams::default())`. That is no
  /// longer safe: a default payload carries no client public key, so it would silently provision
  /// a server with no `[Peer]` block — one that looks healthy and accepts no handshakes. Failing
  /// loudly here is the point.
  fn resolve_docker_init_payload(init_script: &str) -> Result<String, ProviderAdapterError> {
    if init_script.trim().is_empty() {
      return Err(ProviderAdapterError::Unsupported {
        detail:
          "Docker provisioning requires a rendered bootstrap payload containing the client peer. Render bootstrap before calling create."
            .to_string(),
      });
    }

    Ok(init_script.to_string())
  }

  fn container_health(container_id: &str) -> Result<String, ProviderAdapterError> {
    // Containers without a healthcheck have no .State.Health; the `if` keeps the template valid.
    Self::run_docker(
      &[
        "inspect",
        "--format",
        "{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}",
        container_id,
      ],
      "inspect Docker container health status",
    )
    .map(|value| value.trim().to_ascii_lowercase())
  }

  /// Tail of the container's output, for surfacing bootstrap failures.
  ///
  /// `docker logs` reads what the container already emitted; it does not execute anything, and it
  /// works on an exited container, which is exactly the case worth diagnosing.
  ///
  /// Both streams are merged deliberately: Docker sends container stderr to the CLI's stderr, so
  /// reading only stdout hid the error output from a failing bootstrap — which is the half that
  /// actually explains the failure.
  fn container_log_tail(container_id: &str) -> String {
    let output = Command::new("docker")
      .args(["logs", "--tail", "40", container_id])
      .output();

    match output {
      Ok(output) => {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let merged = format!("{}\n{}", stdout.trim_end(), stderr.trim_end());
        let merged = merged.trim().to_string();
        if merged.is_empty() {
          "<no container output yet>".to_string()
        } else {
          merged
        }
      }
      Err(error) => format!("<logs unavailable: {error}>"),
    }
  }

  /// Wait for Docker's healthcheck to report the bootstrap marker exists.
  ///
  /// This polls provider metadata only — the same shape as polling DigitalOcean for droplet
  /// status. Docker runs the health probe itself; the app never execs into the container, and no
  /// key material is read back. The server's public key is already known locally.
  fn wait_for_container_bootstrap(
    container_id: &str,
    timeout: Duration,
  ) -> Result<(), ProviderAdapterError> {
    let poll_interval = Duration::from_millis(1000);
    let started = std::time::Instant::now();

    loop {
      let status = Self::container_runtime_status(container_id)?;

      // The bootstrap script is the entrypoint, so a failed bootstrap exits the container. That
      // is the container-world equivalent of a droplet whose cloud-init died.
      if status == "exited" || status == "dead" {
        return Err(ProviderAdapterError::Unsupported {
          detail: format!(
            "Docker bootstrap failed: container is '{status}'. Recent output:\n{}",
            Self::container_log_tail(container_id)
          ),
        });
      }

      match Self::container_health(container_id)?.as_str() {
        "healthy" => return Ok(()),
        "unhealthy" => {
          return Err(ProviderAdapterError::Unsupported {
            detail: format!(
              "Docker bootstrap did not complete: healthcheck reported unhealthy. Recent output:\n{}",
              Self::container_log_tail(container_id)
            ),
          });
        }
        // `none` means the container was created without a healthcheck, which this adapter always
        // sets. Treat it as still-starting rather than failing, and let the timeout decide.
        _ => {}
      }

      if started.elapsed() >= timeout {
        return Err(ProviderAdapterError::Unsupported {
          detail: format!(
            "Docker bootstrap did not become ready within {}s. Recent output:\n{}",
            timeout.as_secs(),
            Self::container_log_tail(container_id)
          ),
        });
      }

      std::thread::sleep(poll_interval);
    }
  }

  // `build_client_peer_update_script`, `wire_client_peer`, and
  // `initialize_wireguard_server_in_container` were removed here.
  //
  // They staged keys with `docker cp`, ran the init script with `docker exec`, and copied the
  // readiness marker back out. None of that has a DigitalOcean equivalent, so it made the Docker
  // lane pass while the production path stayed broken. Everything the server needs now ships in
  // the payload at create time, and the app only polls provider metadata afterwards.
}

impl super::ProviderAdapter for DockerProviderAdapter {
  fn provider_name(&self) -> &'static str {
    "docker"
  }

  fn credential_fields(&self) -> Vec<CredentialField> {
    vec![CredentialField {
      key: "api_token".to_string(),
      label: "API key".to_string(),
      placeholder: "dkr_v1_...".to_string(),
      secret: true,
      help: Some("Local-only placeholder for the Docker development provider.".to_string()),
    }]
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

  fn list_regions(&self, token: &str) -> Result<Vec<ProviderRegion>, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    Ok(vec![self.describe_region(DOCKER_REGION)])
  }

  /// The dev provider runs on this machine, so it has no geography to report. Leaving the fields
  /// empty is the honest answer and renders as the slug.
  fn describe_region(&self, slug: &str) -> ProviderRegion {
    ProviderRegion::unknown(slug)
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

  fn init_delivery(&self) -> InitDelivery {
    InitDelivery::DockerInit
  }

  fn create_node(
    &self,
    token: &str,
    node_name: &str,
    _region: &str,
    size_slug: &str,
    init_payload: &str,
  ) -> Result<ProvisionedNode, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    let plan = Self::plan_for_key(size_slug).ok_or_else(|| ProviderAdapterError::Unsupported {
      detail: format!("Unsupported Docker plan '{size_slug}'. Expected docker-small, docker-medium, or docker-large."),
    })?;

    let container_name = Self::container_name(node_name);
    let cpus = plan.vcpus.to_string();
    let memory = format!("{}m", plan.memory_mb);

    // Normalize to LF. A CRLF script fails inside the container with an opaque `bad interpreter`
    // error, and this code runs on Windows.
    let docker_init_payload = Self::resolve_docker_init_payload(init_payload)?.replace("\r\n", "\n");

    let run_args = Self::build_container_run_args(
      &container_name,
      &cpus,
      &memory,
      &docker_init_payload,
    );
    let run_arg_refs = run_args.iter().map(String::as_str).collect::<Vec<_>>();
    Self::remove_container_by_exact_name(&container_name)?;

    // No port-conflict recovery here any more. It used to force-remove *any* GoatStare container
    // publishing 51820/udp, which silently destroyed a healthy running node whenever a second one
    // was provisioned. Docker now allocates a free host port per node, so nodes cannot contend.
    let container_id = Self::run_docker(&run_arg_refs, "create Docker compute container")?;

    let endpoint_port = match Self::resolve_published_wireguard_port(&container_id) {
      Ok(port) => port,
      Err(error) => {
        // Leave nothing half-provisioned behind if the endpoint cannot be determined.
        let _ = Self::remove_containers(
          &[container_id.clone()],
          "remove Docker container after port resolution failure",
        );
        return Err(error);
      }
    };

    Ok(ProvisionedNode {
      node_id: container_id,
      public_ipv4: Self::docker_endpoint_host(),
      endpoint_port,
    })
  }

  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    let container_id = request.node_id.as_str();
    Self::wait_for_container_bootstrap(container_id, DOCKER_BOOTSTRAP_TIMEOUT)?;

    let endpoint_port = Self::resolve_published_wireguard_port(container_id)?;

    Ok(BootstrapOutputContract {
      server_endpoint_ip: if request.endpoint_ip.trim().is_empty() {
        Self::docker_endpoint_host()
      } else {
        request.endpoint_ip.clone()
      },
      // Echoed from what the caller generated, not read back off the server. The app has known
      // this key since before the container existed.
      server_public_key: request.server_keypair.public_key.clone(),
      // The port the *client* dials, which for Docker is the published host port.
      listen_port: endpoint_port,
    })
  }

  fn node_exists(&self, token: &str, node_id: &str) -> Result<bool, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    Self::container_exists(node_id)
  }

  fn node_status(&self, token: &str, node_id: &str) -> Result<ProviderNodeStatus, ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    if !Self::container_exists(node_id)? {
      return Ok(ProviderNodeStatus::Deleted);
    }

    Ok(match Self::container_runtime_status(node_id)?.as_str() {
      "running" => ProviderNodeStatus::Active,
      "created" | "restarting" => ProviderNodeStatus::New,
      "removing" => ProviderNodeStatus::Archive,
      "paused" | "exited" | "dead" => ProviderNodeStatus::Off,
      _ => ProviderNodeStatus::Unknown,
    })
  }

  fn delete_node(&self, token: &str, node_id: &str) -> Result<(), ProviderAdapterError> {
    Self::validate_faux_token(token)?;
    let _ = Self::ensure_daemon_available()?;

    if !Self::container_exists(node_id)? {
      return Ok(());
    }

    // The payload lives only in the container's environment, so removing the container removes
    // it. Nothing is staged on the host that could outlive the node.
    Self::remove_containers(
      &[node_id.to_string()],
      "remove Docker compute container",
    )?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::{
    DockerProviderAdapter, DOCKER_CONTAINER_WIREGUARD_PORT, DOCKER_IMAGE, DOCKER_INIT_PAYLOAD_ENV,
    DOCKER_INIT_SCRIPT_PATH,
    DOCKER_REGION, DOCKER_WIREGUARD_PUBLISH,
  };
  use crate::backend::bootstrap::{Bootstrap, BootstrapParams, READINESS_MARKER_PATH};
  use crate::backend::crypto::keypair::{
    generate_client_keypair, generate_server_keypair, WireGuardKeypair,
  };

  /// Render a real bootstrap payload the way the orchestrator does.
  ///
  /// Provisioning no longer accepts an empty script, because a defaulted payload would carry no
  /// client peer. Returns the payload alongside the keypairs so tests can assert that exactly
  /// these keys reach the container.
  fn render_real_docker_payload() -> (String, WireGuardKeypair, WireGuardKeypair) {
    let server = generate_server_keypair().expect("server keypair should generate for real test");
    let client = generate_client_keypair().expect("client keypair should generate for real test");

    let params = BootstrapParams {
      client_public_key: client.public_key.clone(),
      ..BootstrapParams::default()
    };

    let payload = Bootstrap::new()
      .render_docker_init(&params, &server.private_key, &server.public_key)
      .expect("docker init render should succeed")
      .init_payload;

    (payload, server, client)
  }
  use crate::backend::provider::{BootstrapWaitRequest, ProviderAdapter};
  use std::time::{SystemTime, UNIX_EPOCH};

  fn docker_readiness_debug_snapshot(container_id: &str) -> String {
    let mut details = vec![format!("container_id={container_id}")];

    if let Ok(state) = DockerProviderAdapter::run_docker(
      &["inspect", container_id, "--format", "{{.State.Status}}"],
      "inspect Docker container state for readiness debug",
    ) {
      details.push(format!("container_state={}", state.trim()));
    }

    if let Ok(marker_probe) = DockerProviderAdapter::docker_exec(
      container_id,
      &format!(
        "set -eu; if [ -f '{marker}' ]; then echo marker=present; cat '{marker}'; else echo marker=missing; fi",
        marker = READINESS_MARKER_PATH
      ),
      "probe Docker readiness marker state",
    ) {
      details.push(format!("marker_probe={} ", marker_probe.trim()));
    }

    if let Ok(log_tail) = DockerProviderAdapter::run_docker(
      &["logs", "--tail", "80", container_id],
      "collect Docker container logs for readiness debug",
    ) {
      details.push(format!("container_logs_tail={} ", log_tail.trim()));
    }

    details.join(" | ")
  }

  #[test]
  fn docker_provider_uses_ubuntu_24_base_image() {
    assert_eq!(DOCKER_IMAGE, "ubuntu:24.04");

    let client = generate_client_keypair().expect("client keypair should generate");
    let server = generate_server_keypair().expect("server keypair should generate");
    let params = BootstrapParams {
      client_public_key: client.public_key,
      ..BootstrapParams::default()
    };

    let install_script = Bootstrap::new()
      .render_docker_init(&params, &server.private_key, &server.public_key)
      .expect("docker init render should succeed")
      .init_payload;
    assert!(install_script.contains("apt-get update"));
    assert!(install_script.contains("apt-get install -y --no-install-recommends wireguard-tools iproute2 iptables"));
    assert!(!install_script.contains("apk add"));
  }

  #[test]
  fn lists_static_region_without_docker_daemon() {
    let adapter = DockerProviderAdapter::new();

    let result = adapter.list_regions("docker_dev_token").expect("docker region catalog should be static");

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].slug, DOCKER_REGION);
    // The dev provider runs on this machine and has no geography to claim.
    assert_eq!(result[0].city, None);
    assert_eq!(result[0].country_code, None);
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
  fn real_docker_bootstrap_init_phase_sets_up_wireguard_host() {
    let adapter = DockerProviderAdapter::new();
    let token = "dkr_v1_local_dev_mock_token_2026";
    let suffix = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_secs())
      .unwrap_or(0);
    let node_name = format!("docker-bootstrap-phase-{suffix}");
    let (payload, server_keypair, client_keypair) = render_real_docker_payload();
    let server_public_key_copy = server_keypair.public_key.clone();

    let provisioned = adapter
      .create_node(token, &node_name, DOCKER_REGION, "docker-small", &payload)
      .expect("docker create should succeed with a local daemon");

    let contract_result = adapter.wait_for_bootstrap_ready(
      token,
      &BootstrapWaitRequest {
        node_id: provisioned.node_id.clone(),
        endpoint_ip: provisioned.public_ipv4.clone(),
        server_keypair,
      },
    );

    match contract_result {
      Ok(contract) => {
        assert_eq!(contract.server_endpoint_ip, provisioned.public_ipv4);

        // The contract reports the port the *client* dials, which for Docker is the ephemeral
        // published host port, not the container's internal 51820.
        assert_eq!(contract.listen_port, provisioned.endpoint_port);
        assert_ne!(
          contract.listen_port, DOCKER_CONTAINER_WIREGUARD_PORT,
          "host port should be ephemeral so nodes cannot contend for one fixed port"
        );

        // The public key was known locally before the container existed; it is echoed, not read
        // back off the server.
        assert_eq!(contract.server_public_key, server_public_key_copy);

        // The node id *is* the container id now, so there is nothing to look up.
        let container_id = provisioned.node_id.clone();

        DockerProviderAdapter::docker_exec(
          &container_id,
          &format!(
            "set -eu; ip link show wg0 >/dev/null 2>&1; ip -o link show wg0 | grep -q 'UP'; wg show wg0 >/dev/null 2>&1; wg show wg0 listen-port | grep -qx '{internal_port}'; wg show wg0 public-key | grep -qx '{server_public_key}'; wg show wg0 peers | grep -qF '{client_public_key}';",
            internal_port = DOCKER_CONTAINER_WIREGUARD_PORT,
            server_public_key = contract.server_public_key,
            client_public_key = client_keypair.public_key,
          ),
          "validate WireGuard runtime state is active after Docker init",
        )
        .expect("wg0 runtime contract should be active after bootstrap");

        let exists = adapter
          .node_exists(token, &provisioned.node_id)
          .expect("droplet existence check should succeed");
        assert!(exists);
      }
      Err(error) => {
        let debug_snapshot = docker_readiness_debug_snapshot(&provisioned.node_id);
        let _ = adapter.delete_node(token, &provisioned.node_id);
        panic!(
          "docker init script should produce a readiness contract: {:?} | {}",
          error, debug_snapshot
        );
      }
    }

    adapter
      .delete_node(token, &provisioned.node_id)
      .expect("docker cleanup should succeed");

    let exists_after = adapter
      .node_exists(token, &provisioned.node_id)
      .expect("post-delete existence check should succeed");
    assert!(!exists_after);
  }

  #[test]
  #[ignore = "Requires local Docker daemon and WireGuard tooling inside the test container"]
  fn real_docker_server_boots_with_client_peer_already_present() {
    let adapter = DockerProviderAdapter::new();
    let token = "dkr_v1_local_dev_mock_token_2026";
    let suffix = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_secs())
      .unwrap_or(0);
    let node_name = format!("docker-connect-phase-{suffix}");
    let (payload, server_keypair, client_keypair) = render_real_docker_payload();

    let provisioned = adapter
      .create_node(token, &node_name, DOCKER_REGION, "docker-small", &payload)
      .expect("docker create should succeed with a local daemon");

    let contract = adapter
      .wait_for_bootstrap_ready(
        token,
        &BootstrapWaitRequest {
          node_id: provisioned.node_id.clone(),
          endpoint_ip: provisioned.public_ipv4.clone(),
          server_keypair,
        },
      )
      .expect("bootstrap contract should be available");

    // The node id *is* the container id now, so there is nothing to look up.
    let container_id = provisioned.node_id.clone();

    let server_config = DockerProviderAdapter::read_container_text_with_fallback(
      &container_id,
      "/etc/wireguard/wg0.conf",
      "copy WireGuard server config from container",
      "read WireGuard server config from container via shell fallback",
    )
    .expect("server config should be readable from container");

    // The peer arrived in the bootstrap payload. Nothing wired it in afterwards.
    assert!(server_config.contains("[Interface]"));
    assert!(server_config.contains("[Peer]"));
    assert!(server_config.contains("Address = 10.66.0.1/24"));
    assert!(server_config.contains("ListenPort = 51820"));
    assert!(
      server_config.contains(&format!("PublicKey = {}", client_keypair.public_key)),
      "server must already carry the client peer baked at render time"
    );
    assert!(server_config.contains("AllowedIPs = 10.66.0.2/32"));

    assert_eq!(contract.server_endpoint_ip, provisioned.public_ipv4);

    adapter
      .delete_node(token, &provisioned.node_id)
      .expect("docker cleanup should succeed");
  }

  /// End-to-end: provision a real container and emit the client config that reaches it.
  ///
  /// This is the artifact for manual verification — import the written `.conf` into the Windows
  /// WireGuard client and confirm a handshake. Every value in it is known locally: the client
  /// private key and the server public key were both generated here before the container existed,
  /// and the endpoint is what the adapter reported. Nothing was read back off the server.
  ///
  /// The container is deliberately left running so the config can be tested against it. Remove it
  /// with `docker rm -f` when finished.
  #[test]
  #[ignore = "Requires local Docker daemon; leaves a container running for manual verification"]
  fn real_docker_emits_connectable_client_config() {
    use crate::backend::wireguard::{build_client_config, ClientConfigParams};

    let adapter = DockerProviderAdapter::new();
    let token = "dkr_v1_local_dev_mock_token_2026";
    let suffix = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_secs())
      .unwrap_or(0);

    let (payload, server_keypair, client_keypair) = render_real_docker_payload();
    let node_name = format!("docker-clientconf-{suffix}");

    let provisioned = adapter
      .create_node(token, &node_name, DOCKER_REGION, "docker-small", &payload)
      .expect("docker node should provision");

    let contract = adapter
      .wait_for_bootstrap_ready(
        token,
        &BootstrapWaitRequest {
          node_id: provisioned.node_id.clone(),
          endpoint_ip: provisioned.public_ipv4.clone(),
          server_keypair,
        },
      )
      .expect("bootstrap should become ready");

    let config = build_client_config(&ClientConfigParams {
      private_key: client_keypair.private_key.clone(),
      client_address_cidr: "10.66.0.2/32".to_string(),
      dns_server: None,
      server_public_key: contract.server_public_key.clone(),
      server_endpoint_ip: contract.server_endpoint_ip.clone(),
      server_listen_port: contract.listen_port,
      // Route only the tunnel subnet: this config is for verifying a handshake, not for taking
      // over the host's default route.
      allowed_ips: vec!["10.66.0.0/24".to_string()],
      persistent_keepalive_seconds: 25,
    })
    .expect("client config should render");

    assert!(config.contains("[Interface]"));
    assert!(config.contains("[Peer]"));
    assert!(config.contains(&format!("PublicKey = {}", contract.server_public_key)));
    assert!(config.contains(&format!(
      "Endpoint = {}:{}",
      contract.server_endpoint_ip, contract.listen_port
    )));

    let path = std::env::temp_dir().join(format!("goatstare-verify-{suffix}.conf"));
    std::fs::write(&path, config.replace("\r\n", "\n")).expect("client config should be writable");

    let container = provisioned.node_id.clone();

    println!("\n=== client config written to: {} ===", path.display());
    println!(
      "=== endpoint {}:{} ===",
      contract.server_endpoint_ip, contract.listen_port
    );
    println!("=== import it into WireGuard for Windows to verify the handshake ===");
    println!("=== when done, clean up with:  docker rm -f {container} ===");
    println!("=== or remove every GoatStare container:  npm run docker:cleanup ===\n");
  }

  /// Two nodes must be able to exist at once.
  ///
  /// Direct regression test for a data-loss bug: the host port was a fixed
  /// `127.0.0.1:51820:51820/udp`, so a second node could not bind, and the recovery path
  /// force-removed *any* GoatStare container publishing that port — silently destroying the first,
  /// healthy node while leaving its database row behind.
  #[test]
  #[ignore = "Requires local Docker daemon"]
  fn real_docker_second_node_does_not_evict_the_first() {
    let adapter = DockerProviderAdapter::new();
    let token = "dkr_v1_local_dev_mock_token_2026";
    let suffix = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_nanos())
      .unwrap_or(0);

    let (payload_one, _s1, _c1) = render_real_docker_payload();
    let first = adapter
      .create_node(
        token,
        &format!("docker-coexist-a-{suffix}"),
        DOCKER_REGION,
        "docker-small",
        &payload_one,
      )
      .expect("first docker node should provision");

    let (payload_two, _s2, _c2) = render_real_docker_payload();
    let second_result = adapter.create_node(
      token,
      &format!("docker-coexist-b-{suffix}"),
      DOCKER_REGION,
      "docker-small",
      &payload_two,
    );

    let cleanup = |ids: &[&str]| {
      for id in ids {
        let _ = adapter.delete_node(token, id);
      }
    };

    let second = match second_result {
      Ok(second) => second,
      Err(error) => {
        cleanup(&[first.node_id.as_str()]);
        panic!("second docker node should provision alongside the first: {error:?}");
      }
    };

    let first_still_exists = adapter
      .node_exists(token, &first.node_id)
      .unwrap_or(false);
    let ports_differ = first.endpoint_port != second.endpoint_port;

    cleanup(&[first.node_id.as_str(), second.node_id.as_str()]);

    assert!(
      first_still_exists,
      "provisioning a second node must not remove the first"
    );
    assert!(
      ports_differ,
      "each node must get its own host port (got {} twice)",
      first.endpoint_port
    );
  }

  /// Provisioning must refuse an empty payload rather than silently substituting a default one.
  /// A defaulted payload carries no client peer, which would yield a server that looks healthy
  /// and accepts no handshakes.
  #[test]
  fn docker_provisioning_requires_a_rendered_payload() {
    let result = DockerProviderAdapter::resolve_docker_init_payload("   ");

    let err = result.expect_err("an empty payload must be rejected");
    assert!(err.to_string().contains("client peer"));
  }

  fn demo_run_args() -> Vec<String> {
    let (payload, _server, _client) = render_real_docker_payload();

    DockerProviderAdapter::build_container_run_args("demo-container", "1", "1024m", &payload)
  }

  /// The host port must be left for Docker to allocate.
  ///
  /// A fixed `127.0.0.1:51820:51820/udp` meant a second node could not bind, and the recovery
  /// path force-removed the first node's container to free the port — silently destroying a
  /// healthy node. An empty host port makes contention impossible.
  #[test]
  fn docker_publishes_an_ephemeral_host_port_per_node() {
    let args = demo_run_args();

    assert!(args.contains(&"-p".to_string()));
    assert!(args.contains(&DOCKER_WIREGUARD_PUBLISH.to_string()));
    assert!(
      DOCKER_WIREGUARD_PUBLISH.contains("::"),
      "host port must be left empty so Docker allocates a free one per node"
    );
    assert!(
      !args.iter().any(|arg| arg == "127.0.0.1:51820:51820/udp"),
      "the fixed host port that caused nodes to evict each other must not come back"
    );
  }

  /// The payload is supplied with the create call and run as the entrypoint — the container-world
  /// equivalent of cloud-init user-data.
  #[test]
  fn docker_delivers_payload_as_create_time_user_data() {
    let args = demo_run_args();

    assert!(args.contains(&"-e".to_string()));
    assert!(
      args
        .iter()
        .any(|arg| arg.starts_with(&format!("{DOCKER_INIT_PAYLOAD_ENV}=")) && arg.contains("[Peer]")),
      "the rendered payload must travel with the create call"
    );
    assert!(args.contains(&"--entrypoint".to_string()));
    assert!(args.contains(&"/bin/bash".to_string()));
    assert!(
      args.iter().any(|arg| arg.contains(DOCKER_INIT_SCRIPT_PATH)),
      "the entrypoint must materialise and run the payload"
    );
    // Keepalive belongs to the container command, not the shared script, which must terminate
    // normally under cloud-init.
    assert!(args.iter().any(|arg| arg.contains("tail -f /dev/null")));
    assert!(
      !args.iter().any(|arg| arg.contains("while true; do sleep")),
      "the container no longer idles waiting to be exec'd into"
    );
    // A host bind mount only works when the CLI can translate Windows paths for the daemon, which
    // fails outright under DOCKER_HOST=tcp://...
    assert!(
      !args.contains(&"-v".to_string()),
      "payload delivery must not depend on host path translation"
    );
  }

  /// `printf '%s'` rather than `echo`, so nothing inside the payload is interpreted.
  #[test]
  fn entrypoint_materialises_payload_without_interpreting_it() {
    let command = DockerProviderAdapter::build_entrypoint_command();

    assert!(command.contains(&format!("printf '%s' \"${DOCKER_INIT_PAYLOAD_ENV}\"")));
    assert!(command.contains(&format!("> {DOCKER_INIT_SCRIPT_PATH}")));
    assert!(command.contains("chmod 700"));
    assert!(command.contains("exec tail -f /dev/null"));
  }

  /// Readiness is observed through Docker's healthcheck, which Docker runs itself and reports as
  /// provider metadata. The app never execs into the container.
  #[test]
  fn docker_reports_readiness_through_its_own_healthcheck() {
    let args = demo_run_args();

    assert!(args.contains(&"--health-cmd".to_string()));
    assert!(args.contains(&format!("test -f {READINESS_MARKER_PATH}")));
    assert!(args.contains(&"--health-interval".to_string()));
    assert!(args.contains(&"--health-retries".to_string()));
  }

  #[test]
  fn docker_endpoint_host_is_reachable_and_not_assumed_loopback() {
    let host = DockerProviderAdapter::docker_endpoint_host();

    // Must be a dialable address. `0.0.0.0` is a bind address, not a destination, and an empty or
    // whitespace-bearing value means resolution fell through to something unusable.
    let parsed = host
      .parse::<std::net::IpAddr>()
      .unwrap_or_else(|_| panic!("endpoint host must be an IP address, got '{host}'"));

    assert!(!parsed.is_unspecified(), "0.0.0.0 is not a dialable endpoint");
    assert!(!parsed.is_multicast(), "a multicast address is not a dialable endpoint");
  }

}
