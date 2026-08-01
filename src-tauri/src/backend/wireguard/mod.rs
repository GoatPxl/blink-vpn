use std::ffi::OsString;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

pub const DEFAULT_CLIENT_ALLOWED_IPS: &[&str] = &["0.0.0.0/0", "::/0"];
pub const DEFAULT_CLIENT_KEEPALIVE_SECONDS: u16 = 25;

#[derive(Clone)]
pub struct WireGuardManager {
  runner: Arc<dyn CommandRunner>,
}

impl WireGuardManager {
  pub fn new() -> Self {
    Self {
      runner: Arc::new(ProcessCommandRunner),
    }
  }

  pub fn with_runner(runner: Arc<dyn CommandRunner>) -> Self {
    Self { runner }
  }

  pub fn ensure_installed(&self) -> Result<(), WireGuardError> {
    if !cfg!(target_os = "windows") {
      return Err(WireGuardError::UnsupportedPlatform {
        detail: "Phase 1 WireGuard integration is supported on Windows only.".to_string(),
      });
    }

    self
      .runner
      .run("wg", &["--version"], None)
      .map_err(|source| map_install_error(source, "wg"))?;

    self
      .runner
      .run("wireguard", &["/help"], None)
      .map_err(|source| map_install_error(source, "wireguard"))?;

    Ok(())
  }

  pub fn generate_client_keypair(&self) -> Result<ClientKeyPair, WireGuardError> {
    self.ensure_installed()?;

    let private_out = self
      .runner
      .run("wg", &["genkey"], None)
      .map_err(|source| WireGuardError::Command {
        command: "wg genkey".to_string(),
        detail: "WireGuard key generation failed locally.".to_string(),
        source,
      })?;
    let private_key = private_out.stdout.trim().to_string();

    if private_key.is_empty() {
      return Err(WireGuardError::InvalidOutput {
        detail: "WireGuard generated an empty private key output.".to_string(),
      });
    }

    let public_out = self
      .runner
      .run("wg", &["pubkey"], Some(&private_key))
      .map_err(|source| WireGuardError::Command {
        command: "wg pubkey".to_string(),
        detail: "WireGuard public key derivation failed locally.".to_string(),
        source,
      })?;
    let public_key = public_out.stdout.trim().to_string();

    if public_key.is_empty() {
      return Err(WireGuardError::InvalidOutput {
        detail: "WireGuard generated an empty public key output.".to_string(),
      });
    }

    Ok(ClientKeyPair {
      private_key,
      public_key,
    })
  }

  pub fn import_tunnel(&self, tunnel_name: &str, config_text: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    let mut file_path = std::env::temp_dir();
    file_path.push(format!("{tunnel_name}-{}.conf", Uuid::new_v4()));

    let mut file = std::fs::File::create(&file_path).map_err(|source| WireGuardError::Io {
      action: "create_tunnel_config_temp_file",
      detail: "Could not create temporary WireGuard config file for import.".to_string(),
      source,
    })?;

    file
      .write_all(config_text.as_bytes())
      .map_err(|source| WireGuardError::Io {
        action: "write_tunnel_config_temp_file",
        detail: "Could not write WireGuard config file for import.".to_string(),
        source,
      })?;

    let path_arg = file_path.as_os_str().to_string_lossy().to_string();
    let import_result = self
      .runner
      .run("wireguard", &["/installtunnelservice", &path_arg], None)
      .map_err(|source| WireGuardError::Command {
        command: "wireguard /installtunnelservice <config>".to_string(),
        detail: "Could not register tunnel service in WireGuard for Windows.".to_string(),
        source,
      });

    if let Err(err) = std::fs::remove_file(&file_path) {
      log::warn!(
        "Failed to clean up temporary WireGuard config file '{}': {}",
        file_path.display(),
        err
      );
    }

    import_result.map(|_| ())
  }

  pub fn activate_tunnel(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    let service_name = wireguard_service_name(tunnel_name);
    let start_result = self
      .runner
      .run("sc", &["start", &service_name], None)
      .map_err(|source| map_sc_tunnel_error(source, &service_name, "start"));

    match start_result {
      Ok(_) => Ok(()),
      Err(WireGuardError::Command {
        source: CommandExecError::NonZeroExit {
          code: Some(1056),
          ..
        },
        ..
      }) => Ok(()),
      Err(error) => Err(error),
    }
  }

  pub fn deactivate_tunnel(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    let service_name = wireguard_service_name(tunnel_name);
    let stop_result = self
      .runner
      .run("sc", &["stop", &service_name], None)
      .map_err(|source| map_sc_tunnel_error(source, &service_name, "stop"));

    match stop_result {
      Ok(_) => Ok(()),
      Err(WireGuardError::Command {
        source: CommandExecError::NonZeroExit {
          code: Some(1062),
          ..
        },
        ..
      }) => Ok(()),
      Err(error) => Err(error),
    }
  }

  pub fn unregister_tunnel(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    self
      .runner
      .run("wireguard", &["/uninstalltunnelservice", tunnel_name], None)
      .map_err(|source| WireGuardError::Command {
        command: format!("wireguard /uninstalltunnelservice {tunnel_name}"),
        detail: "Could not unregister WireGuard tunnel service.".to_string(),
        source,
      })?;

    Ok(())
  }

  pub fn read_tunnel_status(&self, tunnel_name: &str) -> Result<TunnelRuntimeStatus, WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    let service_name = wireguard_service_name(tunnel_name);
    let service_out = self
      .runner
      .run("sc", &["query", &service_name], None)
      .map_err(|source| map_sc_tunnel_error(source, &service_name, "query"))?;

    let state = parse_sc_state(&service_out.stdout)?;

    let dump_out = self
      .runner
      .run("wg", &["show", tunnel_name, "dump"], None)
      .map_err(|source| WireGuardError::Command {
        command: format!("wg show {tunnel_name} dump"),
        detail: "Could not read WireGuard tunnel peer stats.".to_string(),
        source,
      })?;

    let now_unix_seconds = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map_err(|_| WireGuardError::InvalidOutput {
        detail: "System time is before UNIX epoch; cannot compute handshake age.".to_string(),
      })?
      .as_secs();

    let peer_metrics = parse_wg_dump_metrics(now_unix_seconds, &dump_out.stdout)?;

    Ok(TunnelRuntimeStatus {
      tunnel_name: tunnel_name.to_string(),
      state,
      last_handshake_age_seconds: peer_metrics.last_handshake_age_seconds,
      rx_bytes: peer_metrics.rx_bytes,
      tx_bytes: peer_metrics.tx_bytes,
    })
  }

  pub fn build_client_config(
    &self,
    params: &ClientConfigParams,
  ) -> Result<String, WireGuardError> {
    build_client_config(params)
  }
}

#[derive(Debug, Clone)]
pub struct ClientKeyPair {
  pub private_key: String,
  pub public_key: String,
}

#[derive(Debug, Clone)]
pub struct ClientConfigParams {
  pub private_key: String,
  pub client_address_cidr: String,
  pub dns_server: Option<String>,
  pub server_public_key: String,
  pub server_endpoint_ip: String,
  pub server_listen_port: u16,
  pub allowed_ips: Vec<String>,
  pub persistent_keepalive_seconds: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TunnelServiceState {
  Running,
  Stopped,
  Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TunnelRuntimeStatus {
  pub tunnel_name: String,
  pub state: TunnelServiceState,
  pub last_handshake_age_seconds: Option<u64>,
  pub rx_bytes: Option<u64>,
  pub tx_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PeerMetrics {
  last_handshake_age_seconds: Option<u64>,
  rx_bytes: Option<u64>,
  tx_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
  pub stdout: String,
  pub stderr: String,
}

pub trait CommandRunner: Send + Sync {
  fn run(
    &self,
    program: &str,
    args: &[&str],
    stdin: Option<&str>,
  ) -> Result<CommandOutput, CommandExecError>;
}

#[derive(Debug)]
pub struct ProcessCommandRunner;

impl CommandRunner for ProcessCommandRunner {
  fn run(
    &self,
    program: &str,
    args: &[&str],
    stdin: Option<&str>,
  ) -> Result<CommandOutput, CommandExecError> {
    let mut command = Command::new(program);
    command.args(args);

    if stdin.is_some() {
      command.stdin(Stdio::piped());
    }

    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command
      .spawn()
      .map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => CommandExecError::NotFound {
          program: program.to_string(),
          detail: format!("Command '{program}' was not found in PATH."),
        },
        _ => CommandExecError::Spawn {
          program: program.to_string(),
          detail: format!("Could not start command '{program}'."),
          source,
        },
      })?;

    if let Some(input) = stdin {
      if let Some(mut pipe) = child.stdin.take() {
        pipe
          .write_all(input.as_bytes())
          .map_err(|source| CommandExecError::Io {
            program: program.to_string(),
            detail: "Could not write stdin to command process.".to_string(),
            source,
          })?;
      }
    }

    let output = child.wait_with_output().map_err(|source| CommandExecError::Io {
      program: program.to_string(),
      detail: "Could not collect command output.".to_string(),
      source,
    })?;

    let stdout = String::from_utf8(output.stdout).map_err(|source| CommandExecError::Decode {
      program: program.to_string(),
      detail: "Command stdout was not valid UTF-8.".to_string(),
      source,
    })?;
    let stderr = String::from_utf8(output.stderr).map_err(|source| CommandExecError::Decode {
      program: program.to_string(),
      detail: "Command stderr was not valid UTF-8.".to_string(),
      source,
    })?;

    if !output.status.success() {
      return Err(CommandExecError::NonZeroExit {
        program: program.to_string(),
        code: output.status.code(),
        detail: format_nonzero_detail(&stderr),
      });
    }

    Ok(CommandOutput { stdout, stderr })
  }
}

#[derive(Debug, Error)]
pub enum CommandExecError {
  #[error("Command not found: {program}. {detail}")]
  NotFound { program: String, detail: String },
  #[error("Failed to start command '{program}': {detail}")]
  Spawn {
    program: String,
    detail: String,
    #[source]
    source: std::io::Error,
  },
  #[error("Command '{program}' failed during IO: {detail}")]
  Io {
    program: String,
    detail: String,
    #[source]
    source: std::io::Error,
  },
  #[error("Command '{program}' output decode failed: {detail}")]
  Decode {
    program: String,
    detail: String,
    #[source]
    source: std::string::FromUtf8Error,
  },
  #[error("Command '{program}' exited with code {code:?}: {detail}")]
  NonZeroExit {
    program: String,
    code: Option<i32>,
    detail: String,
  },
}

#[derive(Debug, Error)]
pub enum WireGuardError {
  #[error("WireGuard is not installed or not available on PATH. {detail}")]
  NotInstalled { detail: String },
  #[error("WireGuard is unavailable on this platform. {detail}")]
  UnsupportedPlatform { detail: String },
  #[error("WireGuard command failed for '{command}': {detail}")]
  Command {
    command: String,
    detail: String,
    #[source]
    source: CommandExecError,
  },
  #[error("WireGuard output parsing failed: {detail}")]
  InvalidOutput { detail: String },
  #[error("WireGuard tunnel service '{tunnel_name}' was not found locally. {detail}")]
  TunnelNotFound { tunnel_name: String, detail: String },
  #[error("WireGuard IO operation '{action}' failed: {detail}")]
  Io {
    action: &'static str,
    detail: String,
    #[source]
    source: std::io::Error,
  },
}

pub fn build_client_config(params: &ClientConfigParams) -> Result<String, WireGuardError> {
  if params.private_key.trim().is_empty() {
    return Err(WireGuardError::InvalidOutput {
      detail: "Client private key cannot be empty when building config.".to_string(),
    });
  }
  if params.server_public_key.trim().is_empty() {
    return Err(WireGuardError::InvalidOutput {
      detail: "Server public key cannot be empty when building config.".to_string(),
    });
  }
  if params.server_endpoint_ip.trim().is_empty() {
    return Err(WireGuardError::InvalidOutput {
      detail: "Server endpoint IP cannot be empty when building config.".to_string(),
    });
  }
  if params.client_address_cidr.trim().is_empty() {
    return Err(WireGuardError::InvalidOutput {
      detail: "Client address CIDR cannot be empty when building config.".to_string(),
    });
  }

  let mut allowed_ips = params
    .allowed_ips
    .iter()
    .filter(|value| !value.trim().is_empty())
    .cloned()
    .collect::<Vec<_>>();

  if allowed_ips.is_empty() {
    allowed_ips = DEFAULT_CLIENT_ALLOWED_IPS
      .iter()
      .map(|value| value.to_string())
      .collect();
  }

  let dns_line = params
    .dns_server
    .as_ref()
    .filter(|value| !value.trim().is_empty())
    .map(|dns| format!("DNS = {dns}\n"))
    .unwrap_or_default();

  Ok(format!(
    "[Interface]\nPrivateKey = {}\nAddress = {}\n{}\n[Peer]\nPublicKey = {}\nAllowedIPs = {}\nEndpoint = {}:{}\nPersistentKeepalive = {}\n",
    params.private_key,
    params.client_address_cidr,
    dns_line,
    params.server_public_key,
    allowed_ips.join(", "),
    params.server_endpoint_ip,
    params.server_listen_port,
    params.persistent_keepalive_seconds
  ))
}

fn parse_wg_dump_metrics(now_unix_seconds: u64, dump: &str) -> Result<PeerMetrics, WireGuardError> {
  let mut saw_line = false;
  let mut newest_handshake: Option<u64> = None;
  let mut rx_total: u64 = 0;
  let mut tx_total: u64 = 0;
  let mut peer_count: u64 = 0;
  let mut row_index: usize = 0;

  for line in dump.lines() {
    if line.trim().is_empty() {
      continue;
    }

    saw_line = true;
    let columns = line.split('\t').collect::<Vec<_>>();

    if row_index == 0 {
      row_index += 1;
      continue;
    }

    if columns.len() < 8 {
      return Err(WireGuardError::InvalidOutput {
        detail: "Unexpected wg dump row format while parsing peer metrics.".to_string(),
      });
    }

    peer_count += 1;

    let handshake_index = columns.len() - 4;
    let rx_index = columns.len() - 3;
    let tx_index = columns.len() - 2;

    let handshake_epoch = columns[handshake_index].trim().parse::<u64>().map_err(|_| {
      WireGuardError::InvalidOutput {
        detail: "Could not parse latest handshake timestamp from wg dump.".to_string(),
      }
    })?;

    let rx = columns[rx_index].trim().parse::<u64>().map_err(|_| WireGuardError::InvalidOutput {
      detail: "Could not parse rx bytes from wg dump.".to_string(),
    })?;

    let tx = columns[tx_index].trim().parse::<u64>().map_err(|_| WireGuardError::InvalidOutput {
      detail: "Could not parse tx bytes from wg dump.".to_string(),
    })?;

    rx_total = rx_total.saturating_add(rx);
    tx_total = tx_total.saturating_add(tx);

    if handshake_epoch > 0 {
      newest_handshake = Some(newest_handshake.map_or(handshake_epoch, |value| value.max(handshake_epoch)));
    }

    row_index += 1;
  }

  if !saw_line {
    return Err(WireGuardError::InvalidOutput {
      detail: "wg dump output was empty; expected interface details.".to_string(),
    });
  }

  let last_handshake_age_seconds = newest_handshake.map(|epoch| now_unix_seconds.saturating_sub(epoch));

  Ok(PeerMetrics {
    last_handshake_age_seconds,
    rx_bytes: if peer_count == 0 { None } else { Some(rx_total) },
    tx_bytes: if peer_count == 0 { None } else { Some(tx_total) },
  })
}

fn parse_sc_state(raw_output: &str) -> Result<TunnelServiceState, WireGuardError> {
  for line in raw_output.lines() {
    let trimmed = line.trim();
    if !trimmed.to_ascii_uppercase().starts_with("STATE") {
      continue;
    }

    if trimmed.contains("RUNNING") {
      return Ok(TunnelServiceState::Running);
    }
    if trimmed.contains("STOPPED") {
      return Ok(TunnelServiceState::Stopped);
    }

    return Ok(TunnelServiceState::Unknown);
  }

  Err(WireGuardError::InvalidOutput {
    detail: "Could not parse tunnel service state from 'sc query' output.".to_string(),
  })
}

fn wireguard_service_name(tunnel_name: &str) -> String {
  format!("WireGuardTunnel${tunnel_name}")
}

fn validate_tunnel_name(tunnel_name: &str) -> Result<(), WireGuardError> {
  let is_valid = !tunnel_name.trim().is_empty()
    && tunnel_name
      .chars()
      .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');

  if is_valid {
    return Ok(());
  }

  Err(WireGuardError::InvalidOutput {
    detail: "Tunnel name must use only ASCII letters, numbers, '-' or '_' for Windows service compatibility."
      .to_string(),
  })
}

fn map_install_error(source: CommandExecError, command: &str) -> WireGuardError {
  match source {
    CommandExecError::NotFound { .. } => WireGuardError::NotInstalled {
      detail: format!(
        "Install WireGuard for Windows and ensure '{command}.exe' is available in PATH."
      ),
    },
    other => WireGuardError::Command {
      command: format!("{command} --version"),
      detail: "WireGuard command exists but failed during install check.".to_string(),
      source: other,
    },
  }
}

fn map_sc_tunnel_error(source: CommandExecError, service_name: &str, operation: &str) -> WireGuardError {
  if let CommandExecError::NonZeroExit {
    code: Some(1060),
    ..
  } = source
  {
    return WireGuardError::TunnelNotFound {
      tunnel_name: service_name.strip_prefix("WireGuardTunnel$").unwrap_or(service_name).to_string(),
      detail: "WireGuard tunnel service does not exist on this machine. Re-import the tunnel configuration."
        .to_string(),
    };
  }

  WireGuardError::Command {
    command: format!("sc {operation} {service_name}"),
    detail: "Could not complete WireGuard tunnel service operation.".to_string(),
    source,
  }
}

fn format_nonzero_detail(stderr: &str) -> String {
  let trimmed = stderr.trim();
  if trimmed.is_empty() {
    "Process exited with a non-zero code and no stderr output.".to_string()
  } else {
    let mut safe = OsString::from(trimmed).to_string_lossy().to_string();
    if safe.len() > 500 {
      safe.truncate(500);
      safe.push_str("...[truncated]");
    }
    safe
  }
}

#[cfg(test)]
mod tests {
  use super::{
    ClientConfigParams, DEFAULT_CLIENT_ALLOWED_IPS, DEFAULT_CLIENT_KEEPALIVE_SECONDS,
    TunnelServiceState, WireGuardError, build_client_config, parse_sc_state, parse_wg_dump_metrics,
  };

  #[test]
  fn client_config_generation_includes_required_peer_fields() {
    let config = build_client_config(&ClientConfigParams {
      private_key: "priv-key".to_string(),
      client_address_cidr: "10.44.0.2/32".to_string(),
      dns_server: Some("1.1.1.1".to_string()),
      server_public_key: "server-pub".to_string(),
      server_endpoint_ip: "203.0.113.10".to_string(),
      server_listen_port: 51820,
      allowed_ips: vec!["0.0.0.0/0".to_string()],
      persistent_keepalive_seconds: DEFAULT_CLIENT_KEEPALIVE_SECONDS,
    })
    .expect("config generation should succeed");

    assert!(config.contains("[Interface]"));
    assert!(config.contains("PrivateKey = priv-key"));
    assert!(config.contains("Address = 10.44.0.2/32"));
    assert!(config.contains("DNS = 1.1.1.1"));
    assert!(config.contains("PublicKey = server-pub"));
    assert!(config.contains("Endpoint = 203.0.113.10:51820"));
    assert!(config.contains("PersistentKeepalive = 25"));
  }

  #[test]
  fn client_config_generation_uses_default_allowed_ips_when_missing() {
    let config = build_client_config(&ClientConfigParams {
      private_key: "priv-key".to_string(),
      client_address_cidr: "10.44.0.2/32".to_string(),
      dns_server: None,
      server_public_key: "server-pub".to_string(),
      server_endpoint_ip: "203.0.113.10".to_string(),
      server_listen_port: 51820,
      allowed_ips: vec![],
      persistent_keepalive_seconds: DEFAULT_CLIENT_KEEPALIVE_SECONDS,
    })
    .expect("config generation should succeed");

    for value in DEFAULT_CLIENT_ALLOWED_IPS {
      assert!(config.contains(value));
    }
  }

  #[test]
  fn parse_sc_state_detects_running_and_stopped() {
    let running = "STATE              : 4  RUNNING";
    let stopped = "STATE              : 1  STOPPED";

    assert_eq!(parse_sc_state(running).expect("running parse"), TunnelServiceState::Running);
    assert_eq!(parse_sc_state(stopped).expect("stopped parse"), TunnelServiceState::Stopped);
  }

  #[test]
  fn parse_wg_dump_metrics_extracts_handshake_and_totals() {
    let now = 1_700_000_100_u64;
    let dump = "iface-private\tiface-public\t51820\toff\npeerpub\t(none)\t203.0.113.10:51820\t0.0.0.0/0\t1700000000\t128\t256\t25\npeerpub2\t(none)\t203.0.113.11:51820\t0.0.0.0/0\t1700000050\t10\t20\t25\n";

    let metrics = parse_wg_dump_metrics(now, dump).expect("parse should succeed");
    assert_eq!(metrics.last_handshake_age_seconds, Some(50));
    assert_eq!(metrics.rx_bytes, Some(138));
    assert_eq!(metrics.tx_bytes, Some(276));
  }

  #[test]
  fn parse_wg_dump_metrics_empty_output_errors() {
    let err = parse_wg_dump_metrics(100, "").expect_err("empty output should error");
    match err {
      WireGuardError::InvalidOutput { .. } => {}
      other => panic!("expected invalid output error, got {other}"),
    }
  }
}
