use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use defguard_wireguard_rs::host::Host as WireguardHost;
use defguard_wireguard_rs::key::Key as WireguardKey;
use defguard_wireguard_rs::net::IpAddrMask as WireguardIpAddrMask;
use defguard_wireguard_rs::peer::Peer as WireguardPeer;
use defguard_wireguard_rs::{InterfaceConfiguration as WireguardInterfaceConfiguration, Kernel as WireguardKernel, WGApi as WireguardApi, WireguardInterfaceApi};
use serde::Serialize;
use thiserror::Error;
use wireguard_nt::{Adapter as WireguardNtAdapter, Error as WireguardNtError, Wireguard as WireguardNtRuntime};

use crate::backend::db::WireGuardConnectionMode;
use crate::backend::logging;

pub const DEFAULT_CLIENT_ALLOWED_IPS: &[&str] = &["0.0.0.0/0", "::/0"];
pub const DEFAULT_CLIENT_KEEPALIVE_SECONDS: u16 = 25;
const ACTIVATE_POLL_ATTEMPTS: usize = 60;
const ACTIVATE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const TRACE_HISTORY_LIMIT: usize = 120;
static ADMIN_RUNTIME_ACCESS_DENIED: AtomicBool = AtomicBool::new(false);
static PROCESS_ELEVATION_IS_ADMIN: OnceLock<bool> = OnceLock::new();

#[derive(Clone)]
pub struct WireGuardManager {
  runner: Arc<dyn CommandRunner>,
  debug_trace: Arc<Mutex<WireGuardDebugTrace>>,
}

impl WireGuardManager {
  pub fn new() -> Self {
    Self {
      runner: Arc::new(ProcessCommandRunner),
      debug_trace: Arc::new(Mutex::new(WireGuardDebugTrace::default())),
    }
  }

  pub fn with_runner(runner: Arc<dyn CommandRunner>) -> Self {
    Self {
      runner,
      debug_trace: Arc::new(Mutex::new(WireGuardDebugTrace::default())),
    }
  }

  fn should_force_legacy_for_admin_mode(&self) -> bool {
    ADMIN_RUNTIME_ACCESS_DENIED.load(Ordering::Relaxed)
  }

  fn mark_force_legacy_for_admin_mode(&self) {
    ADMIN_RUNTIME_ACCESS_DENIED.store(true, Ordering::Relaxed);
  }

  fn should_use_legacy_for_admin_mode(&self) -> bool {
    if self.should_force_legacy_for_admin_mode() {
      return true;
    }

    if !self.is_process_elevated_for_admin_runtime() {
      self.mark_force_legacy_for_admin_mode();
      return true;
    }

    false
  }

  fn is_process_elevated_for_admin_runtime(&self) -> bool {
    if cfg!(test) {
      return true;
    }

    *PROCESS_ELEVATION_IS_ADMIN.get_or_init(|| detect_windows_elevated_process(self.runner.as_ref()))
  }

  pub fn debug_snapshot(&self) -> WireGuardDebugSnapshot {
    let guard = self
      .debug_trace
      .lock()
      .expect("wireguard debug trace lock should work");

    WireGuardDebugSnapshot {
      commands: guard.commands.iter().cloned().collect(),
      last_import_tunnel_name: guard.last_import_tunnel_name.clone(),
      last_import_config_redacted: guard.last_import_config_redacted.clone(),
      last_import_config_temp_path: guard.last_import_config_temp_path.clone(),
    }
  }

  fn record_command(&self, command: &str) {
    let mut guard = self
      .debug_trace
      .lock()
      .expect("wireguard debug trace lock should work");
    if guard.commands.len() >= TRACE_HISTORY_LIMIT {
      guard.commands.pop_front();
    }
    guard
      .commands
      .push_back(format!("{} | {}", chrono_like_timestamp(), command));
  }

  fn record_last_import_snapshot(
    &self,
    tunnel_name: &str,
    temp_path: &str,
    config_text: &str,
  ) {
    let mut guard = self
      .debug_trace
      .lock()
      .expect("wireguard debug trace lock should work");
    guard.last_import_tunnel_name = Some(tunnel_name.to_string());
    guard.last_import_config_temp_path = Some(temp_path.to_string());
    guard.last_import_config_redacted = Some(logging::redact_sensitive_text(config_text));
  }

  pub fn record_connect_trace_snapshot(&self, tunnel_name: &str, config_text: &str) {
    self.record_last_import_snapshot(
      tunnel_name,
      "in-memory://connect_tunnel_config",
      config_text,
    );
  }

  pub fn capture_runtime_connect_trace_snapshot(
    &self,
    tunnel_name: &str,
  ) -> Result<(), WireGuardError> {
    self.capture_runtime_connect_trace_snapshot_for_mode(
      WireGuardConnectionMode::User,
      tunnel_name,
    )
  }

  pub fn capture_runtime_connect_trace_snapshot_for_mode(
    &self,
    mode: WireGuardConnectionMode,
    tunnel_name: &str,
  ) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    match mode {
      WireGuardConnectionMode::Admin => {
        let host = self.read_runtime_host_data(tunnel_name)?;
        let snapshot = format_runtime_snapshot(&host);
        self.record_command(&format!(
          "defguard_wireguard_rs::WGApi::read_interface_data {} ({})",
          tunnel_name,
          mode.as_str()
        ));
        self.record_last_import_snapshot(
          tunnel_name,
          "runtime://defguard_wireguard_rs.read_interface_data.admin",
          &snapshot,
        );
        Ok(())
      }
      WireGuardConnectionMode::User => {
        self.record_command(&format!("wg showconf {}", tunnel_name));
        match self.runner.run("wg", &["showconf", tunnel_name], None) {
          Ok(output) if !output.stdout.trim().is_empty() => {
            self.record_last_import_snapshot(tunnel_name, "runtime://wg_showconf", &output.stdout);
            Ok(())
          }
          Ok(_) | Err(_) => {
            self.record_command(&format!("wg show {} dump", tunnel_name));
            let output = self
              .runner
              .run("wg", &["show", tunnel_name, "dump"], None)
              .map_err(|source| map_wg_show_dump_error(source, tunnel_name))?;
            self.record_last_import_snapshot(tunnel_name, "runtime://wg_show_dump", &output.stdout);
            Ok(())
          }
        }
      }
    }
  }

  pub fn ensure_installed(&self) -> Result<(), WireGuardError> {
    if !cfg!(target_os = "windows") {
      return Err(WireGuardError::UnsupportedPlatform {
        detail: "Phase 1 WireGuard integration is supported on Windows only.".to_string(),
      });
    }

    let wireguard_program = self.resolve_wireguard_program()?;

    let wireguard_probe = self
      .runner
      .run(&wireguard_program, &["/help"], None);

    if !command_probe_indicates_presence(&wireguard_probe) {
      let source = wireguard_probe
        .err()
        .unwrap_or(CommandExecError::NotFound {
          program: wireguard_program.clone(),
          detail: "WireGuard command probe failed.".to_string(),
        });
      return Err(map_install_error(source, &wireguard_program));
    }

    Ok(())
  }

  pub fn check_readiness(&self) -> WireGuardReadiness {
    if !cfg!(target_os = "windows") {
      return WireGuardReadiness {
        installed: false,
        executable_found: false,
        service_available: false,
        version: None,
        message: "WireGuard for Windows is required on this platform.".to_string(),
        action_guidance: "Use a Windows host and install WireGuard for Windows before onboarding."
          .to_string(),
      };
    }

    let wireguard_program = self.resolve_wireguard_program().ok();

    let wireguard_probe = wireguard_program
      .as_ref()
      .map(|program| self.runner.run(program, &["/help"], None));
    let service_check = self
      .runner
      .run("sc", &["query", "WireGuardManager"], None);

    let executable_found = wireguard_probe
      .as_ref()
      .is_some_and(command_probe_indicates_presence);
    let service_available = service_check.is_ok();
    let installed = executable_found;

    let (message, action_guidance) = if installed && service_available {
      (
        "WireGuard runtime is installed and ready.".to_string(),
        "Continue onboarding.".to_string(),
      )
    } else if !installed {
      (
        "WireGuard for Windows is missing or incomplete.".to_string(),
        "Install or repair WireGuard for Windows, ensure 'wireguard.exe' is available, then click Re-check."
          .to_string(),
      )
    } else {
      (
        "WireGuard tools are available, but the manager service check failed.".to_string(),
        "Repair/reinstall WireGuard for Windows and verify the 'WireGuardManager' service exists, then click Re-check."
          .to_string(),
      )
    };

    WireGuardReadiness {
      installed,
      executable_found,
      service_available,
      version: None,
      message,
      action_guidance,
    }
  }

  pub fn resolution_diagnostics(&self) -> WireGuardResolutionDiagnostics {
    WireGuardResolutionDiagnostics {
      process_path: std::env::var_os("PATH").map(|value| value.to_string_lossy().to_string()),
      wireguard_resolved_path: resolve_wireguard_command_path("wireguard")
        .map(|path| path.to_string_lossy().to_string()),
    }
  }

  fn resolve_wireguard_program(&self) -> Result<String, WireGuardError> {
    self.resolve_program("wireguard")
  }

  fn resolve_program(&self, command_name: &str) -> Result<String, WireGuardError> {
    if let Some(path) = resolve_wireguard_command_path(command_name) {
      return Ok(path.to_string_lossy().to_string());
    }

    Err(WireGuardError::NotInstalled {
      detail: format!(
        "Could not locate the required WireGuard installation containing '{command_name}.exe'. Repair WireGuard for Windows, then click Re-check."
      ),
    })
  }

  pub fn generate_client_keypair(&self) -> Result<ClientKeyPair, WireGuardError> {
    self.ensure_installed()?;
    self.record_command("defguard_wireguard_rs::key::Key::generate");

    let private_key = WireguardKey::generate().to_string();
    let public_key = self.derive_public_key(&private_key)?;

    Ok(ClientKeyPair {
      private_key,
      public_key,
    })
  }

  pub fn derive_public_key(&self, private_key: &str) -> Result<String, WireGuardError> {
    self.record_command("defguard_wireguard_rs::key::Key::public_key");

    let private_key = WireguardKey::try_from(private_key).map_err(|_| WireGuardError::InvalidOutput {
      detail: "WireGuard private key is invalid; expected base64 or hexadecimal 32-byte key material."
        .to_string(),
    })?;

    let public_key = private_key.public_key().to_string();
    if public_key.trim().is_empty() {
      return Err(WireGuardError::InvalidOutput {
        detail: "WireGuard key derivation produced an empty public key output.".to_string(),
      });
    }

    Ok(public_key)
  }

  pub fn import_tunnel(&self, tunnel_name: &str, config_text: &str) -> Result<(), WireGuardError> {
    self.import_tunnel_for_mode(WireGuardConnectionMode::User, tunnel_name, config_text)
  }

  pub fn import_tunnel_for_mode(
    &self,
    mode: WireGuardConnectionMode,
    tunnel_name: &str,
    config_text: &str,
  ) -> Result<(), WireGuardError> {
    match mode {
      WireGuardConnectionMode::Admin if self.should_use_legacy_for_admin_mode() => {
        self.import_tunnel_legacy(tunnel_name, config_text)
      }
      WireGuardConnectionMode::Admin => match self.import_tunnel_defguard(tunnel_name, config_text) {
        Ok(()) => Ok(()),
        Err(error) if is_wireguard_runtime_access_denied_error(&error) => {
          self.mark_force_legacy_for_admin_mode();
          self.import_tunnel_legacy(tunnel_name, config_text)
        }
        Err(error) => Err(error),
      },
      WireGuardConnectionMode::User => self.import_tunnel_legacy(tunnel_name, config_text),
    }
  }

  pub fn activate_tunnel(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.activate_tunnel_for_mode(WireGuardConnectionMode::User, tunnel_name)
  }

  pub fn activate_tunnel_for_mode(
    &self,
    mode: WireGuardConnectionMode,
    tunnel_name: &str,
  ) -> Result<(), WireGuardError> {
    match mode {
      WireGuardConnectionMode::Admin if self.should_use_legacy_for_admin_mode() => {
        self.activate_tunnel_legacy(tunnel_name)
      }
      WireGuardConnectionMode::Admin => match self.activate_tunnel_defguard(tunnel_name) {
        Ok(()) => Ok(()),
        Err(error) if is_wireguard_runtime_access_denied_error(&error) => {
          self.mark_force_legacy_for_admin_mode();
          self.activate_tunnel_legacy(tunnel_name)
        }
        Err(error) => Err(error),
      },
      WireGuardConnectionMode::User => self.activate_tunnel_legacy(tunnel_name),
    }
  }

  fn wait_for_running_tunnel(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    let mut last_state = TunnelActivationProbeState::Unknown;
    let mut tried_elevated_query = false;

    for _ in 0..ACTIVATE_POLL_ATTEMPTS {
      match self.probe_tunnel_service_state(tunnel_name) {
        Ok(TunnelActivationProbeState::Running) => return Ok(()),
        Ok(state) => {
          last_state = state;
          thread::sleep(ACTIVATE_POLL_INTERVAL);
        }
        Err(error) => {
          if !tried_elevated_query && is_wireguard_access_denied_error(&error) {
            tried_elevated_query = true;
            if let Ok(state) = probe_tunnel_service_state_via_uac(tunnel_name) {
              if state == TunnelActivationProbeState::Running {
                return Ok(());
              }
              last_state = state;
            }
          }
          thread::sleep(ACTIVATE_POLL_INTERVAL);
        }
      }
    }

    Err(WireGuardError::InvalidOutput {
      detail: format!(
        "WireGuard tunnel '{tunnel_name}' did not reach Running after activation; last observed service state was {}. Check whether the tunnel is still starting, disabled, or blocked by a bad tunnel configuration.",
        describe_tunnel_activation_probe_state(last_state)
      ),
    })
  }

  fn probe_tunnel_service_state(
    &self,
    tunnel_name: &str,
  ) -> Result<TunnelActivationProbeState, WireGuardError> {
    let service_name = wireguard_service_name(tunnel_name);
    self.record_command(&format!("sc query {}", service_name));
    let service_out = self
      .runner
      .run("sc", &["query", &service_name], None)
      .map_err(|source| map_sc_tunnel_error(source, &service_name, "query"))?;

    for line in service_out.stdout.lines() {
      let trimmed = line.trim();
      if !trimmed.to_ascii_uppercase().starts_with("STATE") {
        continue;
      }

      if trimmed.contains("RUNNING") {
        return Ok(TunnelActivationProbeState::Running);
      }

      if trimmed.contains("START_PENDING") {
        return Ok(TunnelActivationProbeState::Starting);
      }

      if trimmed.contains("STOPPED") {
        return Ok(TunnelActivationProbeState::Stopped);
      }

      return Ok(TunnelActivationProbeState::Unknown);
    }

    Ok(TunnelActivationProbeState::Unknown)
  }

  

  pub fn deactivate_tunnel(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.deactivate_tunnel_for_mode(WireGuardConnectionMode::User, tunnel_name)
  }

  pub fn deactivate_tunnel_for_mode(
    &self,
    mode: WireGuardConnectionMode,
    tunnel_name: &str,
  ) -> Result<(), WireGuardError> {
    match mode {
      WireGuardConnectionMode::Admin if self.should_use_legacy_for_admin_mode() => {
        self.deactivate_tunnel_legacy(tunnel_name)
      }
      WireGuardConnectionMode::Admin => match self.deactivate_tunnel_defguard(tunnel_name) {
        Ok(()) => Ok(()),
        Err(error) if is_wireguard_runtime_access_denied_error(&error) => {
          self.mark_force_legacy_for_admin_mode();
          self.deactivate_tunnel_legacy(tunnel_name)
        }
        Err(error) => Err(error),
      },
      WireGuardConnectionMode::User => self.deactivate_tunnel_legacy(tunnel_name),
    }
  }

  pub fn unregister_tunnel(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.unregister_tunnel_for_mode(WireGuardConnectionMode::User, tunnel_name)
  }

  pub fn unregister_tunnel_for_mode(
    &self,
    mode: WireGuardConnectionMode,
    tunnel_name: &str,
  ) -> Result<(), WireGuardError> {
    match mode {
      WireGuardConnectionMode::Admin if self.should_use_legacy_for_admin_mode() => {
        self.unregister_tunnel_legacy(tunnel_name)
      }
      WireGuardConnectionMode::Admin => match self.unregister_tunnel_defguard(tunnel_name) {
        Ok(()) => Ok(()),
        Err(error) if is_wireguard_runtime_access_denied_error(&error) => {
          self.mark_force_legacy_for_admin_mode();
          self.unregister_tunnel_legacy(tunnel_name)
        }
        Err(error) => Err(error),
      },
      WireGuardConnectionMode::User => self.unregister_tunnel_legacy(tunnel_name),
    }
  }

  fn import_tunnel_legacy(&self, tunnel_name: &str, config_text: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    let wireguard_program = self.resolve_wireguard_program()?;
    if self.tunnel_service_exists(tunnel_name)? {
      self.unregister_legacy_tunnel_service_with_program(tunnel_name, &wireguard_program)?;
    }

    let mut config_dir = std::env::temp_dir();
    let stamp = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_nanos())
      .unwrap_or(0);
    config_dir.push(format!("goatstare-wireguard-import-{stamp}"));
    fs::create_dir_all(&config_dir).map_err(|source| WireGuardError::Io {
      action: "create_temp_config_dir",
      detail: format!(
        "Could not create temporary WireGuard config directory '{}'.",
        config_dir.display()
      ),
      source,
    })?;

    let config_path = config_dir.join(format!("{tunnel_name}.conf"));
    fs::write(&config_path, config_text).map_err(|source| WireGuardError::Io {
      action: "write_temp_config",
      detail: format!(
        "Could not write temporary WireGuard config '{}'.",
        config_path.display()
      ),
      source,
    })?;

    self.record_last_import_snapshot(tunnel_name, &config_path.to_string_lossy(), config_text);

    let config_path_string = config_path.to_string_lossy().to_string();
    let install_args = ["/installtunnelservice", config_path_string.as_str()];

    self.record_command(&format!(
      "{} /installtunnelservice {}",
      wireguard_program, config_path_string
    ));
    let result = if cfg!(target_os = "windows") && !cfg!(test) {
      self.record_command(&format!(
        "{} /installtunnelservice {} (via UAC)",
        wireguard_program, config_path_string
      ));
      run_wireguard_via_uac(&wireguard_program, &install_args)
    } else {
      match self.runner.run(&wireguard_program, &install_args, None) {
        Ok(output) => Ok(output),
        Err(source) => {
          let should_retry_elevated = matches!(
            &source,
            CommandExecError::NonZeroExit { detail, .. }
              if is_windows_access_denied_detail(detail)
          );

          if should_retry_elevated {
            self.record_command(&format!(
              "{} /installtunnelservice {} (retry via UAC)",
              wireguard_program, config_path_string
            ));
            run_wireguard_via_uac(&wireguard_program, &install_args)
          } else {
            Err(source)
          }
        }
      }
    }
    .map_err(|source| WireGuardError::Command {
      command: format!("wireguard /installtunnelservice {config_path_string}"),
      detail:
        "Could not import WireGuard tunnel service. If prompted, approve administrator elevation."
          .to_string(),
      source,
    });

    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_dir_all(&config_dir);
    result.map(|_| ())
  }

  fn activate_tunnel_legacy(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    if let Ok(state) = self.probe_tunnel_service_state(tunnel_name) {
      if matches!(state, TunnelActivationProbeState::Running | TunnelActivationProbeState::Starting) {
        return self.wait_for_running_tunnel(tunnel_name);
      }
    }

    let service_name = wireguard_service_name(tunnel_name);
    self.record_command(&format!("sc start {}", service_name));
    let start_result = run_sc_with_uac_fallback(self.runner.as_ref(), "start", &service_name);

    match start_result {
      Ok(_) => self.wait_for_running_tunnel(tunnel_name),
      Err(error) => {
        if self.wait_for_running_tunnel(tunnel_name).is_ok() {
          return Ok(());
        }

        Err(error)
      }
    }
  }

  fn deactivate_tunnel_legacy(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    let service_name = wireguard_service_name(tunnel_name);
    self.record_command(&format!("sc stop {}", service_name));
    let stop_result = self.runner.run("sc", &["stop", &service_name], None);

    match stop_result {
      Ok(_) => Ok(()),
      Err(CommandExecError::NonZeroExit { code: Some(1060), .. }) => Ok(()),
      Err(CommandExecError::NonZeroExit { code: Some(1062), .. }) => Ok(()),
      Err(CommandExecError::NonZeroExit { detail, .. }) if is_windows_service_missing_detail(&detail) => Ok(()),
      Err(source) => Err(map_sc_tunnel_error(source, &service_name, "stop")),
    }
  }

  fn unregister_tunnel_legacy(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.deactivate_tunnel_legacy(tunnel_name)?;
    self.unregister_legacy_tunnel_service(tunnel_name)
  }

  fn tunnel_service_exists(&self, tunnel_name: &str) -> Result<bool, WireGuardError> {
    let service_name = wireguard_service_name(tunnel_name);
    self.record_command(&format!("sc query {}", service_name));
    let query_result = self
      .runner
      .run("sc", &["query", &service_name], None)
      .map_err(|source| map_sc_tunnel_error(source, &service_name, "query"));

    match query_result {
      Ok(_) => Ok(true),
      Err(WireGuardError::TunnelNotFound { .. }) => Ok(false),
      Err(WireGuardError::Command {
        source: CommandExecError::NonZeroExit { detail, .. },
        ..
      }) if is_windows_access_denied_detail(&detail) => Ok(true),
      Err(error) => Err(error),
    }
  }

  pub fn read_tunnel_status(&self, tunnel_name: &str) -> Result<TunnelRuntimeStatus, WireGuardError> {
    self.read_tunnel_status_for_mode(WireGuardConnectionMode::User, tunnel_name)
  }

  pub fn read_tunnel_status_for_mode(
    &self,
    mode: WireGuardConnectionMode,
    tunnel_name: &str,
  ) -> Result<TunnelRuntimeStatus, WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    match mode {
      WireGuardConnectionMode::Admin if self.should_use_legacy_for_admin_mode() => {
        self.read_tunnel_status_legacy(tunnel_name)
      }
      WireGuardConnectionMode::Admin => match self.read_tunnel_status_defguard(tunnel_name) {
        Ok(status) => Ok(status),
        Err(error) if is_wireguard_runtime_access_denied_error(&error) => {
          self.mark_force_legacy_for_admin_mode();
          self.read_tunnel_status_legacy(tunnel_name)
        }
        Err(error) => Err(error),
      },
      WireGuardConnectionMode::User => self.read_tunnel_status_legacy(tunnel_name),
    }
  }

  fn read_tunnel_status_legacy(&self, tunnel_name: &str) -> Result<TunnelRuntimeStatus, WireGuardError> {
    validate_tunnel_name(tunnel_name)?;

    let service_name = wireguard_service_name(tunnel_name);
    self.record_command(&format!("sc query {}", service_name));
    let service_out = self
      .runner
      .run("sc", &["query", &service_name], None)
      .map_err(|source| map_sc_tunnel_error(source, &service_name, "query"))?;

    let state = parse_sc_state(&service_out.stdout)?;

    if state != TunnelServiceState::Running {
      return Ok(TunnelRuntimeStatus {
        tunnel_name: tunnel_name.to_string(),
        state,
        last_handshake_age_seconds: None,
        rx_bytes: None,
        tx_bytes: None,
      });
    }

    self.record_command(&format!("wg show {} dump", tunnel_name));
    let dump = self
      .runner
      .run("wg", &["show", tunnel_name, "dump"], None)
      .map_err(|source| map_wg_show_dump_error(source, tunnel_name))?;

    let now_unix_seconds = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_secs())
      .unwrap_or(0);
    let peer_metrics = parse_wg_dump_metrics(now_unix_seconds, &dump.stdout)?;

    Ok(TunnelRuntimeStatus {
      tunnel_name: tunnel_name.to_string(),
      state,
      last_handshake_age_seconds: peer_metrics.last_handshake_age_seconds,
      rx_bytes: peer_metrics.rx_bytes,
      tx_bytes: peer_metrics.tx_bytes,
    })
  }

  fn read_tunnel_status_defguard(&self, tunnel_name: &str) -> Result<TunnelRuntimeStatus, WireGuardError> {
    validate_tunnel_name(tunnel_name)?;

    let adapter = open_existing_adapter(tunnel_name)?;
    self.record_command(&format!("wireguard_nt::Adapter::is_up {}", tunnel_name));
    let is_up = adapter.is_up().map_err(map_wireguard_nt_error)?;
    let state = if is_up {
      TunnelServiceState::Running
    } else {
      TunnelServiceState::Stopped
    };

    if state != TunnelServiceState::Running {
      return Ok(TunnelRuntimeStatus {
        tunnel_name: tunnel_name.to_string(),
        state,
        last_handshake_age_seconds: None,
        rx_bytes: None,
        tx_bytes: None,
      });
    }

    self.record_command(&format!(
      "defguard_wireguard_rs::WGApi::read_interface_data {} (admin)",
      tunnel_name
    ));
    let host = self.read_runtime_host_data(tunnel_name)?;
    let peer_metrics = peer_metrics_from_host(&host);

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

  fn import_tunnel_defguard(&self, tunnel_name: &str, config_text: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;
    ensure_defguard_runtime_dll_path()?;

    let parsed = parse_interface_config_from_wg_text(tunnel_name, config_text)?;
    self.record_last_import_snapshot(
      tunnel_name,
      "runtime://defguard_wireguard_rs.configure_interface",
      config_text,
    );
    let invocation = panic::catch_unwind(AssertUnwindSafe(|| {
      self.record_command("defguard_wireguard_rs::WGApi::create_interface");

      let mut api = WireguardApi::<WireguardKernel>::new(tunnel_name).map_err(map_defguard_error)?;
      api.create_interface().map_err(map_defguard_error)?;

      self.record_command("defguard_wireguard_rs::WGApi::configure_interface");
      api.configure_interface(&parsed.interface)
        .map_err(map_defguard_error)?;

      if !parsed.dns_servers.is_empty() {
        self.record_command("defguard_wireguard_rs::WGApi::configure_dns");
        api.configure_dns(&parsed.dns_servers, &[])
          .map_err(map_defguard_error)?;
      }

      Ok(())
    }));

    match invocation {
      Ok(result) => result,
      Err(_) => Err(WireGuardError::Runtime {
        detail: "WireGuard runtime crashed while importing the tunnel interface. Verify WireGuard for Windows runtime files are installed and retry."
          .to_string(),
      }),
    }
  }

  fn activate_tunnel_defguard(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    let adapter = open_existing_adapter(tunnel_name)?;
    self.record_command(&format!("wireguard_nt::Adapter::up {}", tunnel_name));
    adapter.up().map_err(map_wireguard_nt_error)?;

    for _ in 0..ACTIVATE_POLL_ATTEMPTS {
      if adapter.is_up().map_err(map_wireguard_nt_error)? {
        return Ok(());
      }
      thread::sleep(ACTIVATE_POLL_INTERVAL);
    }

    Err(WireGuardError::Runtime {
      detail: format!(
        "WireGuard adapter '{}' did not report Up after activation.",
        tunnel_name
      ),
    })
  }

  fn deactivate_tunnel_defguard(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    let adapter = match open_existing_adapter(tunnel_name) {
      Ok(adapter) => adapter,
      Err(WireGuardError::TunnelNotFound { .. }) => return Ok(()),
      Err(error) => return Err(error),
    };

    self.record_command(&format!("wireguard_nt::Adapter::down {}", tunnel_name));
    adapter.down().map_err(map_wireguard_nt_error)?;
    Ok(())
  }

  fn unregister_tunnel_defguard(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    // wireguard-nt exposes up/down and close but no single-adapter deletion API.
    // Keep parity with previous uninstall behavior by applying a legacy cleanup attempt
    // for service-managed tunnels after bringing the adapter down.
    self.deactivate_tunnel_defguard(tunnel_name)?;
    self.unregister_legacy_tunnel_service(tunnel_name)
  }

  fn unregister_legacy_tunnel_service(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    let wireguard_program = self.resolve_wireguard_program()?;
    self.unregister_legacy_tunnel_service_with_program(tunnel_name, &wireguard_program)
  }

  fn unregister_legacy_tunnel_service_with_program(
    &self,
    tunnel_name: &str,
    wireguard_program: &str,
  ) -> Result<(), WireGuardError> {

    if !self.tunnel_service_exists(tunnel_name)? {
      return Ok(());
    }

    let uninstall_args = ["/uninstalltunnelservice", tunnel_name];
    self.record_command(&format!(
      "{} /uninstalltunnelservice {}",
      wireguard_program, tunnel_name
    ));
    let result = if cfg!(target_os = "windows") && !cfg!(test) {
      self.record_command(&format!(
        "{} /uninstalltunnelservice {} (via UAC)",
        wireguard_program, tunnel_name
      ));
      run_wireguard_via_uac(&wireguard_program, &uninstall_args)
    } else {
      match self.runner.run(&wireguard_program, &uninstall_args, None) {
        Ok(output) => Ok(output),
        Err(source) => {
          let should_retry_elevated = matches!(
            &source,
            CommandExecError::NonZeroExit { detail, .. }
              if is_windows_access_denied_detail(detail)
          );

          if should_retry_elevated {
            self.record_command(&format!(
              "{} /uninstalltunnelservice {} (retry via UAC)",
              wireguard_program, tunnel_name
            ));
            run_wireguard_via_uac(&wireguard_program, &uninstall_args)
          } else {
            Err(source)
          }
        }
      }
    }
    .map_err(|source| WireGuardError::Command {
      command: format!("wireguard /uninstalltunnelservice {tunnel_name}"),
      detail:
        "Could not unregister WireGuard tunnel service. If prompted, approve administrator elevation for cleanup."
          .to_string(),
      source,
    });

    match result {
      Ok(_) => Ok(()),
      Err(WireGuardError::Command {
        source: CommandExecError::NonZeroExit { code: Some(1060), .. },
        ..
      }) => Ok(()),
      Err(WireGuardError::Command {
        source: CommandExecError::NonZeroExit { detail, .. },
        ..
      }) if is_windows_service_missing_detail(&detail) => Ok(()),
      Err(error) => Err(error),
    }
  }
}

impl WireGuardManager {
  fn read_runtime_host_data(&self, tunnel_name: &str) -> Result<WireguardHost, WireGuardError> {
    let api = WireguardApi::<WireguardKernel>::new(tunnel_name).map_err(map_defguard_error)?;
    api.read_interface_data().map_err(map_defguard_error)
  }
}

#[derive(Debug, Clone, Default)]
struct WireGuardDebugTrace {
  commands: VecDeque<String>,
  last_import_tunnel_name: Option<String>,
  last_import_config_redacted: Option<String>,
  last_import_config_temp_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireGuardDebugSnapshot {
  pub commands: Vec<String>,
  pub last_import_tunnel_name: Option<String>,
  pub last_import_config_redacted: Option<String>,
  pub last_import_config_temp_path: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TunnelActivationProbeState {
  Running,
  Starting,
  Stopped,
  Unknown,
}

fn describe_tunnel_activation_probe_state(state: TunnelActivationProbeState) -> &'static str {
  match state {
    TunnelActivationProbeState::Running => "running",
    TunnelActivationProbeState::Starting => "start_pending",
    TunnelActivationProbeState::Stopped => "stopped",
    TunnelActivationProbeState::Unknown => "unknown",
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireGuardReadiness {
  pub installed: bool,
  pub executable_found: bool,
  pub service_available: bool,
  pub version: Option<String>,
  pub message: String,
  pub action_guidance: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireGuardResolutionDiagnostics {
  pub process_path: Option<String>,
  pub wireguard_resolved_path: Option<String>,
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
    let resolved_program = resolve_wireguard_command_path(program)
      .map(|path| path.to_string_lossy().to_string())
      .unwrap_or_else(|| program.to_string());

    let mut command = Command::new(&resolved_program);
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
          program: resolved_program.to_string(),
          detail: format!("Command '{}' was not found in PATH.", resolved_program),
        },
        _ => CommandExecError::Spawn {
          program: resolved_program.to_string(),
          detail: format!("Could not start command '{}'.", resolved_program),
          source,
        },
      })?;

    if let Some(input) = stdin {
      if let Some(mut pipe) = child.stdin.take() {
        pipe
          .write_all(input.as_bytes())
          .map_err(|source| CommandExecError::Io {
            program: resolved_program.to_string(),
            detail: "Could not write stdin to command process.".to_string(),
            source,
          })?;
      }
    }

    let output = child.wait_with_output().map_err(|source| CommandExecError::Io {
      program: resolved_program.to_string(),
      detail: "Could not collect command output.".to_string(),
      source,
    })?;

    let stdout = String::from_utf8(output.stdout).map_err(|source| CommandExecError::Decode {
      program: resolved_program.to_string(),
      detail: "Command stdout was not valid UTF-8.".to_string(),
      source,
    })?;
    let stderr = String::from_utf8(output.stderr).map_err(|source| CommandExecError::Decode {
      program: resolved_program.to_string(),
      detail: "Command stderr was not valid UTF-8.".to_string(),
      source,
    })?;

    if !output.status.success() {
      return Err(CommandExecError::NonZeroExit {
        program: resolved_program.to_string(),
        code: output.status.code(),
        detail: format_nonzero_detail(&stdout, &stderr),
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
  #[error("WireGuard runtime API failed: {detail}")]
  Runtime { detail: String },
}

#[derive(Debug, Clone)]
struct ParsedInterfaceConfig {
  interface: WireguardInterfaceConfiguration,
  dns_servers: Vec<IpAddr>,
}

fn parse_interface_config_from_wg_text(
  tunnel_name: &str,
  config_text: &str,
) -> Result<ParsedInterfaceConfig, WireGuardError> {
  let mut section = String::new();
  let mut private_key: Option<String> = None;
  let mut addresses: Vec<WireguardIpAddrMask> = Vec::new();
  let mut dns_servers: Vec<IpAddr> = Vec::new();
  let mut peer_public_key: Option<String> = None;
  let mut peer_endpoint: Option<SocketAddr> = None;
  let mut peer_allowed_ips: Vec<WireguardIpAddrMask> = Vec::new();
  let mut keepalive: Option<u16> = None;

  for raw_line in config_text.lines() {
    let line = raw_line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
      continue;
    }

    if line.starts_with('[') && line.ends_with(']') {
      section = line[1..line.len() - 1].trim().to_ascii_lowercase();
      continue;
    }

    let Some((key_raw, value_raw)) = line.split_once('=') else {
      continue;
    };
    let key = key_raw.trim().to_ascii_lowercase();
    let value = value_raw.trim();

    match (section.as_str(), key.as_str()) {
      ("interface", "privatekey") => private_key = Some(value.to_string()),
      ("interface", "address") => {
        for entry in value.split(',').map(str::trim).filter(|part| !part.is_empty()) {
          let ip_mask = entry
            .parse::<WireguardIpAddrMask>()
            .map_err(|_| WireGuardError::InvalidOutput {
              detail: format!("Could not parse interface address '{}'.", entry),
            })?;
          addresses.push(ip_mask);
        }
      }
      ("interface", "dns") => {
        for entry in value.split(',').map(str::trim).filter(|part| !part.is_empty()) {
          let ip = entry.parse::<IpAddr>().map_err(|_| WireGuardError::InvalidOutput {
            detail: format!("Could not parse DNS address '{}'.", entry),
          })?;
          dns_servers.push(ip);
        }
      }
      ("peer", "publickey") => peer_public_key = Some(value.to_string()),
      ("peer", "endpoint") => {
        let endpoint = value.parse::<SocketAddr>().map_err(|_| WireGuardError::InvalidOutput {
          detail: format!("Could not parse peer endpoint '{}'.", value),
        })?;
        peer_endpoint = Some(endpoint);
      }
      ("peer", "allowedips") => {
        for entry in value.split(',').map(str::trim).filter(|part| !part.is_empty()) {
          let ip_mask = entry
            .parse::<WireguardIpAddrMask>()
            .map_err(|_| WireGuardError::InvalidOutput {
              detail: format!("Could not parse peer allowed IP '{}'.", entry),
            })?;
          peer_allowed_ips.push(ip_mask);
        }
      }
      ("peer", "persistentkeepalive") => {
        let seconds = value.parse::<u16>().map_err(|_| WireGuardError::InvalidOutput {
          detail: format!("Could not parse persistent keepalive '{}'.", value),
        })?;
        keepalive = Some(seconds);
      }
      _ => {}
    }
  }

  let private_key = private_key.ok_or_else(|| WireGuardError::InvalidOutput {
    detail: "Tunnel config is missing [Interface] PrivateKey.".to_string(),
  })?;
  let public_key = peer_public_key.ok_or_else(|| WireGuardError::InvalidOutput {
    detail: "Tunnel config is missing [Peer] PublicKey.".to_string(),
  })?;
  let endpoint = peer_endpoint.ok_or_else(|| WireGuardError::InvalidOutput {
    detail: "Tunnel config is missing [Peer] Endpoint.".to_string(),
  })?;

  let mut peer = WireguardPeer::new(
    WireguardKey::try_from(public_key.as_str()).map_err(|_| WireGuardError::InvalidOutput {
      detail: "Tunnel config contains an invalid peer public key value.".to_string(),
    })?,
  );
  peer.endpoint = Some(endpoint);
  peer.allowed_ips = peer_allowed_ips;
  peer.persistent_keepalive_interval = keepalive;

  let interface = WireguardInterfaceConfiguration {
    name: tunnel_name.to_string(),
    prvkey: private_key,
    addresses,
    port: 0,
    peers: vec![peer],
    mtu: None,
    fwmark: None,
  };

  Ok(ParsedInterfaceConfig {
    interface,
    dns_servers,
  })
}

fn wireguard_dll_name() -> &'static str {
  #[cfg(target_arch = "aarch64")]
  {
    "wireguard-arm64.dll"
  }
  #[cfg(target_arch = "x86_64")]
  {
    "wireguard-amd64.dll"
  }
}

fn wireguard_canonical_dll_name() -> &'static str {
  "wireguard.dll"
}

fn wireguard_nt_zip_arch_dir() -> &'static str {
  #[cfg(target_arch = "aarch64")]
  {
    "arm64"
  }
  #[cfg(target_arch = "x86_64")]
  {
    "amd64"
  }
}

fn candidate_wireguard_dll_paths() -> Vec<PathBuf> {
  let mut candidates = Vec::new();

  if let Ok(exe) = std::env::current_exe() {
    if let Some(parent) = exe.parent() {
      candidates.push(parent.join("resources-windows").join("binaries").join(wireguard_dll_name()));
      candidates.push(parent.join("resources-windows").join("binaries").join(wireguard_canonical_dll_name()));
      candidates.push(parent.join("resources-windows").join(wireguard_dll_name()));
      candidates.push(parent.join("resources-windows").join(wireguard_canonical_dll_name()));
      candidates.push(parent.join(wireguard_dll_name()));
      candidates.push(parent.join(wireguard_canonical_dll_name()));
    }
  }

  if let Ok(cwd) = std::env::current_dir() {
    candidates.push(cwd.join("resources-windows").join("binaries").join(wireguard_dll_name()));
    candidates.push(cwd.join("resources-windows").join("binaries").join(wireguard_canonical_dll_name()));
    candidates.push(cwd.join("resources-windows").join(wireguard_dll_name()));
    candidates.push(cwd.join("resources-windows").join(wireguard_canonical_dll_name()));
    candidates.push(cwd.join(wireguard_dll_name()));
    candidates.push(cwd.join(wireguard_canonical_dll_name()));
  }

  let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap_or_else(|| Path::new("."));
  candidates.push(repo_root.join("src-tauri").join("resources-windows").join("binaries").join(wireguard_dll_name()));
  candidates.push(repo_root.join("src-tauri").join("resources-windows").join("binaries").join(wireguard_canonical_dll_name()));
  candidates.push(repo_root.join("src-tauri").join("resources-windows").join(wireguard_dll_name()));
  candidates.push(repo_root.join("src-tauri").join("resources-windows").join(wireguard_canonical_dll_name()));

  candidates.push(PathBuf::from(r"C:\Program Files\WireGuard\").join(wireguard_dll_name()));
  candidates.push(PathBuf::from(r"C:\Program Files\WireGuard\").join(wireguard_canonical_dll_name()));
  candidates.push(PathBuf::from(r"C:\Program Files (x86)\WireGuard\").join(wireguard_dll_name()));
  candidates.push(PathBuf::from(r"C:\Program Files (x86)\WireGuard\").join(wireguard_canonical_dll_name()));

  candidates
}

fn bootstrap_wireguard_dll_from_download() -> Result<PathBuf, WireGuardError> {
  let cwd = std::env::current_dir().map_err(|source| WireGuardError::Io {
    action: "resolve_current_dir",
    detail: "Could not resolve process current directory while bootstrapping WireGuard runtime DLL."
      .to_string(),
    source,
  })?;
  let runtime_dir = cwd.join("resources-windows").join("binaries");
  let target_path = runtime_dir.join(wireguard_dll_name());

  if let Some(parent) = target_path.parent() {
    let _ = fs::create_dir_all(parent);
  }

  let temp_base = std::env::temp_dir().join("goatstare-wireguard-nt");
  let temp_dir = temp_base.join("extract");
  let _ = fs::remove_dir_all(&temp_dir);
  let _ = fs::create_dir_all(&temp_base);
  let zip_path = temp_base.join("wireguard-nt-1.1.zip");

  let powershell = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
  let script = format!(
    r#"$ErrorActionPreference = "Stop"; $zip = '{}'; $dest = '{}'; $out = '{}'; $preferredArch = '{}'; Invoke-WebRequest -Uri 'https://download.wireguard.com/wireguard-nt/wireguard-nt-1.1.zip' -OutFile $zip; Expand-Archive -Path $zip -DestinationPath $dest -Force; $preferred = Join-Path $dest ("bin\" + $preferredArch + "\wireguard.dll"); $dll = if (Test-Path $preferred) {{ $preferred }} else {{ Get-ChildItem -Path $dest -Recurse -Filter 'wireguard.dll' -File | Select-Object -First 1 -ExpandProperty FullName }}; if (-not $dll) {{ throw "wireguard.dll was not found in downloaded wireguard-nt archive." }}; Copy-Item $dll -Destination $out -Force"#,
    zip_path.display(),
    temp_dir.display(),
    target_path.display(),
    wireguard_nt_zip_arch_dir()
  );

  let output = Command::new(powershell)
    .arg("-NoProfile")
    .arg("-ExecutionPolicy")
    .arg("Bypass")
    .arg("-Command")
    .arg(&script)
    .output()
    .map_err(|source| WireGuardError::Runtime {
      detail: format!("Failed to invoke PowerShell for WireGuard runtime bootstrap: {source}"),
    })?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    return Err(WireGuardError::Runtime {
      detail: format!(
        "WireGuard runtime bootstrap failed: {}",
        stderr.trim()
      ),
    });
  }

  if target_path.exists() {
    Ok(target_path)
  } else {
    Err(WireGuardError::Runtime {
      detail: format!(
        "WireGuard runtime DLL was not found after bootstrap. Checked: {}",
        target_path.display()
      ),
    })
  }
}

fn ensure_wireguard_dll_available() -> Result<PathBuf, WireGuardError> {
  let mut seen = std::collections::HashSet::new();
  let mut candidate_paths = candidate_wireguard_dll_paths();
  candidate_paths.retain(|path| seen.insert(path.clone()));

  for path in &candidate_paths {
    if path.exists() {
      return Ok(path.clone());
    }
  }

  let cwd = std::env::current_dir().map_err(|source| WireGuardError::Io {
    action: "resolve_current_dir",
    detail: "Could not resolve process current directory while searching for WireGuard runtime DLL."
      .to_string(),
    source,
  })?;
  let runtime_dir = cwd.join("resources-windows").join("binaries");
  let target_path = runtime_dir.join(wireguard_dll_name());

  if target_path.exists() {
    return Ok(target_path);
  }

  bootstrap_wireguard_dll_from_download()
}

fn ensure_defguard_runtime_dll_path() -> Result<PathBuf, WireGuardError> {
  let source_dll = ensure_wireguard_dll_available()?;
  let cwd = std::env::current_dir().map_err(|source| WireGuardError::Io {
    action: "resolve_current_dir",
    detail: "Could not resolve process current directory while preparing WireGuard runtime DLL."
      .to_string(),
    source,
  })?;

  let defguard_path = cwd
    .join("resources-windows")
    .join("binaries")
    .join(wireguard_dll_name());

  if defguard_path.exists() {
    return Ok(defguard_path);
  }

  if let Some(parent) = defguard_path.parent() {
    fs::create_dir_all(parent).map_err(|source| WireGuardError::Io {
      action: "create_runtime_dir",
      detail: format!(
        "Could not create DefGuard runtime directory at '{}'.",
        parent.display()
      ),
      source,
    })?;
  }

  if source_dll != defguard_path {
    fs::copy(&source_dll, &defguard_path).map_err(|source| WireGuardError::Io {
      action: "copy_runtime_dll",
      detail: format!(
        "Could not copy WireGuard runtime DLL from '{}' to '{}'.",
        source_dll.display(),
        defguard_path.display()
      ),
      source,
    })?;
  }

  Ok(defguard_path)
}

fn load_wireguard_nt_runtime() -> Result<WireguardNtRuntime, WireGuardError> {
  let dll_path = ensure_wireguard_dll_available()?;
  // Loading a signed system WireGuard runtime DLL from application resources.
  unsafe { wireguard_nt::load_from_path(dll_path.to_string_lossy().as_ref()) }.map_err(|source| {
    WireGuardError::Runtime {
      detail: format!(
        "Could not load WireGuard-NT runtime DLL at '{}': {}",
        dll_path.display(), source
      ),
    }
  })
}

fn open_existing_adapter(tunnel_name: &str) -> Result<WireguardNtAdapter, WireGuardError> {
  let runtime = load_wireguard_nt_runtime()?;
  WireguardNtAdapter::open(&runtime, tunnel_name).map_err(|source| match source {
    WireguardNtError::Driver(io_error) if is_adapter_not_found_error(&io_error) => {
      WireGuardError::TunnelNotFound {
        tunnel_name: tunnel_name.to_string(),
        detail: "WireGuard adapter is not present locally.".to_string(),
      }
    }
    other => map_wireguard_nt_error(other),
  })
}

fn map_defguard_error(source: defguard_wireguard_rs::error::WireguardInterfaceError) -> WireGuardError {
  WireGuardError::Runtime {
    detail: format!("defguard_wireguard_rs runtime error: {source}"),
  }
}

fn map_wireguard_nt_error(source: WireguardNtError) -> WireGuardError {
  WireGuardError::Runtime {
    detail: format!("wireguard-nt runtime error: {source}"),
  }
}

fn is_adapter_not_found_error(io_error: &std::io::Error) -> bool {
  io_error.raw_os_error() == Some(2)
    || io_error
      .to_string()
      .to_ascii_lowercase()
      .contains("cannot find")
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

fn peer_metrics_from_host(host: &WireguardHost) -> PeerMetrics {
  let mut newest_handshake_age_seconds: Option<u64> = None;
  let mut rx_total: u64 = 0;
  let mut tx_total: u64 = 0;
  let mut peer_count: u64 = 0;

  for peer in host.peers.values() {
    peer_count += 1;
    rx_total = rx_total.saturating_add(peer.rx_bytes);
    tx_total = tx_total.saturating_add(peer.tx_bytes);

    if let Some(last_handshake) = peer.last_handshake {
      if let Ok(age) = SystemTime::now().duration_since(last_handshake) {
        let age_seconds = age.as_secs();
        newest_handshake_age_seconds = Some(match newest_handshake_age_seconds {
          Some(current) => current.min(age_seconds),
          None => age_seconds,
        });
      }
    }
  }

  PeerMetrics {
    last_handshake_age_seconds: newest_handshake_age_seconds,
    rx_bytes: if peer_count == 0 { None } else { Some(rx_total) },
    tx_bytes: if peer_count == 0 { None } else { Some(tx_total) },
  }
}

fn format_runtime_snapshot(host: &WireguardHost) -> String {
  format!("{host:#?}")
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

fn parse_tunnel_activation_probe_state(raw_output: &str) -> TunnelActivationProbeState {
  for line in raw_output.lines() {
    let trimmed = line.trim();
    if !trimmed.to_ascii_uppercase().starts_with("STATE") {
      continue;
    }

    if trimmed.contains("RUNNING") {
      return TunnelActivationProbeState::Running;
    }

    if trimmed.contains("START_PENDING") {
      return TunnelActivationProbeState::Starting;
    }

    if trimmed.contains("STOPPED") {
      return TunnelActivationProbeState::Stopped;
    }

    return TunnelActivationProbeState::Unknown;
  }

  TunnelActivationProbeState::Unknown
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

fn is_windows_service_missing_detail(detail: &str) -> bool {
  let lowered = detail.to_ascii_lowercase();
  lowered.contains("1060")
    || lowered.contains("does not exist")
    || lowered.contains("not found")
    || lowered.contains("unable to find")
    || lowered.contains("no such service")
}

fn is_windows_access_denied_detail(detail: &str) -> bool {
  let lowered = detail.to_ascii_lowercase();
  lowered.contains("access is denied")
    || lowered.contains("requires elevation")
    || lowered.contains("administrator")
    || lowered.contains("0x80070005")
}

fn is_wireguard_access_denied_error(error: &WireGuardError) -> bool {
  match error {
    WireGuardError::Command {
      source: CommandExecError::NonZeroExit { detail, .. },
      ..
    } => is_windows_access_denied_detail(detail),
    _ => false,
  }
}

fn is_wireguard_runtime_access_denied_error(error: &WireGuardError) -> bool {
  match error {
    WireGuardError::Runtime { detail } => {
      let lowered = detail.to_ascii_lowercase();
      lowered.contains("access is denied") || lowered.contains("os error 5")
    }
    _ => false,
  }
}

fn run_sc_with_uac_fallback(
  runner: &dyn CommandRunner,
  operation: &str,
  service_name: &str,
) -> Result<CommandOutput, WireGuardError> {
  let args = [operation, service_name];
  match runner.run("sc", &args, None) {
    Ok(output) => Ok(output),
    Err(source) => {
      if should_retry_sc_with_uac(&source) {
        run_wireguard_via_uac("sc", &args)
          .map_err(|elevated_source| map_sc_tunnel_error(elevated_source, service_name, operation))
      } else {
        Err(map_sc_tunnel_error(source, service_name, operation))
      }
    }
  }
}

fn should_retry_sc_with_uac(source: &CommandExecError) -> bool {
  match source {
    CommandExecError::NonZeroExit { detail, .. } => is_windows_access_denied_detail(detail),
    _ => false,
  }
}

fn probe_tunnel_service_state_via_uac(
  tunnel_name: &str,
) -> Result<TunnelActivationProbeState, WireGuardError> {
  let service_name = wireguard_service_name(tunnel_name);
  let output = run_wireguard_via_uac("sc", &["query", &service_name]).map_err(|source| {
    map_sc_tunnel_error(source, &service_name, "query")
  })?;

  Ok(parse_tunnel_activation_probe_state(&output.stdout))
}

fn run_wireguard_via_uac(program: &str, args: &[&str]) -> Result<CommandOutput, CommandExecError> {
  if !cfg!(target_os = "windows") {
    return Err(CommandExecError::Spawn {
      program: program.to_string(),
      detail: "UAC elevation fallback is only supported on Windows.".to_string(),
      source: std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "UAC fallback attempted on a non-Windows host",
      ),
    });
  }

  let escaped_program = program.replace('\'', "''");
  let escaped_args = args
    .iter()
    .map(|arg| format!("'{}'", arg.replace('\'', "''")))
    .collect::<Vec<_>>()
    .join(", ");

  let script = format!(
    "$p = Start-Process -FilePath '{}' -Verb RunAs -Wait -PassThru -ArgumentList @({}); if ($null -eq $p) {{ exit 1223 }}; exit $p.ExitCode",
    escaped_program, escaped_args
  );

  ProcessCommandRunner.run(
    "powershell",
    &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script],
    None,
  )
}

fn resolve_wireguard_command_path(program: &str) -> Option<PathBuf> {
  if !cfg!(target_os = "windows") {
    return None;
  }

  if program != "wg" && program != "wireguard" {
    return None;
  }

  if let Some(in_path) = find_executable_in_path(program) {
    return Some(in_path);
  }

  None
}

fn find_executable_in_path(program: &str) -> Option<PathBuf> {
  let path_os = std::env::var_os("PATH")?;
  for segment in std::env::split_paths(&path_os) {
    let candidate_exe = segment.join(format!("{program}.exe"));
    if candidate_exe.is_file() {
      return Some(candidate_exe);
    }

    let candidate_plain = segment.join(program);
    if candidate_plain.is_file() {
      return Some(candidate_plain);
    }
  }

  None
}
fn parse_wg_version_output(raw: &str) -> Option<String> {
  let trimmed = raw.trim();
  if trimmed.is_empty() {
    return None;
  }

  let first_line = trimmed.lines().next()?.trim();
  if first_line.is_empty() {
    return None;
  }

  Some(first_line.to_string())
}

fn detect_windows_elevated_process(runner: &dyn CommandRunner) -> bool {
  if !cfg!(target_os = "windows") {
    return false;
  }

  let output = match runner.run("whoami", &["/groups"], None) {
    Ok(output) => output,
    Err(_) => return false,
  };

  let group_output = output.stdout.to_ascii_lowercase();

  group_output.contains("s-1-16-12288")
    || group_output.contains("s-1-16-16384")
    || group_output.contains("s-1-16-20480")
}

fn command_probe_indicates_presence(result: &Result<CommandOutput, CommandExecError>) -> bool {
  match result {
    Ok(_) => true,
    Err(CommandExecError::NonZeroExit { .. }) => true,
    Err(CommandExecError::Decode { .. }) => true,
    Err(CommandExecError::NotFound { .. }) => false,
    Err(CommandExecError::Spawn { .. }) => false,
    Err(CommandExecError::Io { .. }) => false,
  }
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

  if let CommandExecError::NonZeroExit {
    ref detail,
    ..
  } = source
  {
    if is_windows_service_missing_detail(detail) {
      return WireGuardError::TunnelNotFound {
        tunnel_name: service_name.strip_prefix("WireGuardTunnel$").unwrap_or(service_name).to_string(),
        detail: "WireGuard tunnel service does not exist on this machine. Re-import the tunnel configuration."
          .to_string(),
      };
    }
  }

  let detail = match operation {
    "start" => {
      "Could not start the WireGuard tunnel service. If the tunnel was just imported, wait a few seconds and retry once. If it still fails, the tunnel configuration may be invalid, the service may require elevation, or the Windows WireGuard installation may be unhealthy.".to_string()
    }
    "stop" => {
      "Could not stop the WireGuard tunnel service. If it is already stopped, retrying is safe. Otherwise confirm the service is still present and that WireGuard has permission to manage it.".to_string()
    }
    "query" => {
      "Could not read the WireGuard tunnel service state. This usually means the tunnel service is still transitioning, missing, or Windows denied access to the service control manager.".to_string()
    }
    _ => "Could not complete WireGuard tunnel service operation.".to_string(),
  };

  WireGuardError::Command {
    command: format!("sc {operation} {service_name}"),
    detail,
    source,
  }
}

fn map_wg_show_dump_error(source: CommandExecError, tunnel_name: &str) -> WireGuardError {
  if let CommandExecError::NonZeroExit {
    code: Some(1060),
    ..
  } = source
  {
    return WireGuardError::TunnelNotFound {
      tunnel_name: tunnel_name.to_string(),
      detail: "WireGuard tunnel service does not exist on this machine. Re-import the tunnel configuration."
        .to_string(),
    };
  }

  if let CommandExecError::NonZeroExit { ref detail, .. } = source {
    if is_windows_service_missing_detail(detail) {
      return WireGuardError::TunnelNotFound {
        tunnel_name: tunnel_name.to_string(),
        detail: "WireGuard tunnel service does not exist on this machine. Re-import the tunnel configuration."
          .to_string(),
      };
    }
  }

  WireGuardError::Command {
    command: format!("wg show {tunnel_name} dump"),
    detail: "Could not read WireGuard runtime peer metrics for the active tunnel.".to_string(),
    source,
  }
}

fn format_nonzero_detail(stdout: &str, stderr: &str) -> String {
  let stderr_trimmed = stderr.trim();
  let stdout_trimmed = stdout.trim();

  let base = if !stderr_trimmed.is_empty() {
    stderr_trimmed
  } else if !stdout_trimmed.is_empty() {
    stdout_trimmed
  } else {
    return "Process exited with a non-zero code and no stdout/stderr output.".to_string();
  };

  let mut safe = OsString::from(base).to_string_lossy().to_string();
  if safe.len() > 500 {
    safe.truncate(500);
    safe.push_str("...[truncated]");
  }
  safe
}

fn chrono_like_timestamp() -> String {
  let epoch = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|value| value.as_secs())
    .unwrap_or(0);
  format!("epoch:{epoch}")
}

#[cfg(test)]
mod tests {
  use super::{
    ClientConfigParams, CommandExecError, CommandOutput, CommandRunner,
    DEFAULT_CLIENT_ALLOWED_IPS, DEFAULT_CLIENT_KEEPALIVE_SECONDS, TunnelServiceState,
    WireGuardError, WireGuardManager, build_client_config, is_adapter_not_found_error,
    is_windows_access_denied_detail,
    is_windows_service_missing_detail, parse_sc_state,
    parse_interface_config_from_wg_text, parse_wg_dump_metrics, parse_wg_version_output,
  };
  use std::io;
  use std::sync::{Arc, Mutex};

  #[derive(Clone, Default)]
  struct MockImportRunner {
    calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    service_exists: Arc<Mutex<bool>>,
  }

  impl MockImportRunner {
    fn with_service_exists(service_exists: bool) -> Self {
      Self {
        calls: Arc::new(Mutex::new(Vec::new())),
        service_exists: Arc::new(Mutex::new(service_exists)),
      }
    }

    fn calls(&self) -> Vec<(String, Vec<String>)> {
      self.calls.lock().expect("lock must work").clone()
    }
  }

  impl CommandRunner for MockImportRunner {
    fn run(
      &self,
      program: &str,
      args: &[&str],
      _stdin: Option<&str>,
    ) -> Result<CommandOutput, CommandExecError> {
      let program_name = std::path::Path::new(program)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(program);

      self.calls.lock().expect("lock must work").push((
        program.to_string(),
        args.iter().map(|value| value.to_string()).collect(),
      ));

      if program_name == "wg" && args == ["--version"] {
        return Ok(CommandOutput {
          stdout: "wireguard-tools v1".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "sc" && args.len() == 2 && args[0] == "query" {
        let service_exists = *self.service_exists.lock().expect("lock must work");
        if service_exists {
          return Ok(CommandOutput {
            stdout: "STATE              : 1  STOPPED".to_string(),
            stderr: String::new(),
          });
        }

        return Err(CommandExecError::NonZeroExit {
          program: "sc".to_string(),
          code: Some(1060),
          detail: "The specified service does not exist as an installed service.".to_string(),
        });
      }

      if program_name == "wireguard" && args.len() == 2 && args[0] == "/uninstalltunnelservice" {
        let mut service_exists = self.service_exists.lock().expect("lock must work");
        if !*service_exists {
          return Err(CommandExecError::NonZeroExit {
            program: "wireguard".to_string(),
            code: Some(1060),
            detail: "service missing".to_string(),
          });
        }

        *service_exists = false;
        return Ok(CommandOutput {
          stdout: format!("uninstalled {}", args[1]),
          stderr: String::new(),
        });
      }

      if program_name == "wireguard" && args.len() == 2 && args[0] == "/installtunnelservice" {
        let file_name = std::path::Path::new(args[1])
          .file_name()
          .and_then(|value| value.to_str())
          .unwrap_or("");
        if !file_name.ends_with(".conf") || !file_name.starts_with("wg-node-1") {
          return Err(CommandExecError::NonZeroExit {
            program: "wireguard".to_string(),
            code: Some(1),
            detail: format!("unexpected config path {file_name}"),
          });
        }

        *self.service_exists.lock().expect("lock must work") = true;
        return Ok(CommandOutput {
          stdout: "installed".to_string(),
          stderr: String::new(),
        });
      }

      Err(CommandExecError::NonZeroExit {
        program: program.to_string(),
        code: Some(1),
        detail: "unsupported mock command".to_string(),
      })
    }
  }

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

  #[test]
  fn parse_wg_version_output_extracts_first_line() {
    let version = parse_wg_version_output("wireguard-tools v1.0.20210914\nextra");
    assert_eq!(version, Some("wireguard-tools v1.0.20210914".to_string()));
  }

  #[test]
  fn import_tunnel_replaces_stale_service_and_uses_tunnel_name_basename() {
    let runner = MockImportRunner::with_service_exists(true);
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    let result = wireguard.import_tunnel(
      "wg-node-1",
      "[Interface]\nPrivateKey = priv\nAddress = 10.66.0.2/32\n\n[Peer]\nPublicKey = pub\nAllowedIPs = 0.0.0.0/0\nEndpoint = 203.0.113.10:51820\nPersistentKeepalive = 25\n",
    );

    assert!(result.is_ok());

    let calls = runner.calls();
    assert!(calls.iter().any(|(_, args)| args == &["/uninstalltunnelservice".to_string(), "wg-node-1".to_string()]));

    let install_call = calls
      .iter()
      .find(|(_, args)| args.first().is_some_and(|arg| arg == "/installtunnelservice"))
      .expect("install command should be issued");

    assert!(install_call.1[1].ends_with("wg-node-1.conf"));
  }

  #[test]
  fn import_tunnel_succeeds_when_service_is_not_preinstalled() {
    let runner = MockImportRunner::with_service_exists(false);
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    let result = wireguard.import_tunnel(
      "wg-node-1",
      "[Interface]\nPrivateKey = priv\nAddress = 10.66.0.2/32\n\n[Peer]\nPublicKey = pub\nAllowedIPs = 0.0.0.0/0\nEndpoint = 203.0.113.10:51820\nPersistentKeepalive = 25\n",
    );

    assert!(result.is_ok());

    let calls = runner.calls();
    assert!(!calls.iter().any(|(_, args)| args == &[
      "/uninstalltunnelservice".to_string(),
      "wg-node-1".to_string(),
    ]));
    assert!(calls.iter().any(|(_, args)| args.first().is_some_and(|arg| arg == "/installtunnelservice")));
  }

  #[derive(Clone, Default)]
  struct MockMissingServiceTextRunner;

  impl CommandRunner for MockMissingServiceTextRunner {
    fn run(
      &self,
      program: &str,
      args: &[&str],
      _stdin: Option<&str>,
    ) -> Result<CommandOutput, CommandExecError> {
      let program_name = std::path::Path::new(program)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(program);

      if program_name == "wg" && args == ["--version"] {
        return Ok(CommandOutput {
          stdout: "wireguard-tools v1".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "sc" && args.len() == 2 && args[0] == "query" {
        return Err(CommandExecError::NonZeroExit {
          program: "sc".to_string(),
          code: Some(1060),
          detail: "The specified service does not exist as an installed service.".to_string(),
        });
      }

      if program_name == "wireguard" && args.len() == 2 && args[0] == "/uninstalltunnelservice" {
        return Err(CommandExecError::NonZeroExit {
          program: "wireguard".to_string(),
          code: Some(1),
          detail: "The specified service does not exist as an installed service. (1060)"
            .to_string(),
        });
      }

      if program_name == "wireguard" && args.len() == 2 && args[0] == "/installtunnelservice" {
        return Ok(CommandOutput {
          stdout: "installed".to_string(),
          stderr: String::new(),
        });
      }

      Err(CommandExecError::NonZeroExit {
        program: program.to_string(),
        code: Some(1),
        detail: "unsupported mock command".to_string(),
      })
    }
  }

  #[test]
  fn import_tunnel_succeeds_when_uninstall_reports_missing_service_in_stderr_detail() {
    let runner = Arc::new(MockMissingServiceTextRunner);
    let wireguard = WireGuardManager::with_runner(runner);

    let result = wireguard.import_tunnel(
      "wg-node-1",
      "[Interface]\nPrivateKey = priv\nAddress = 10.66.0.2/32\n\n[Peer]\nPublicKey = pub\nAllowedIPs = 0.0.0.0/0\nEndpoint = 203.0.113.10:51820\nPersistentKeepalive = 25\n",
    );

    assert!(result.is_ok());
  }

  #[test]
  fn windows_error_detail_detectors_match_expected_variants() {
    assert!(is_windows_service_missing_detail(
      "The specified service does not exist as an installed service. (1060)"
    ));
    assert!(is_windows_access_denied_detail("[Error:  Access is denied.]"));
    assert!(is_windows_access_denied_detail("The requested operation requires elevation."));
    assert!(!is_windows_access_denied_detail("service missing"));
  }

    #[test]
    fn retries_sc_with_uac_when_access_denied() {
      let source = CommandExecError::NonZeroExit {
        program: "sc".to_string(),
        code: Some(5),
        detail: "Access is denied.".to_string(),
      };

      assert!(super::should_retry_sc_with_uac(&source));
    }

    #[test]
    fn does_not_retry_sc_with_uac_for_non_permission_failures() {
      let source = CommandExecError::NonZeroExit {
        program: "sc".to_string(),
        code: Some(1060),
        detail: "The specified service does not exist as an installed service.".to_string(),
      };

      assert!(!super::should_retry_sc_with_uac(&source));
    }

  #[test]
  fn map_sc_tunnel_error_detects_missing_service_from_detail_text() {
    let error = super::map_sc_tunnel_error(
      CommandExecError::NonZeroExit {
        program: "sc".to_string(),
        code: Some(1),
        detail: "[SC] StartService FAILED 1060: The specified service does not exist as an installed service."
          .to_string(),
      },
      "WireGuardTunnel$Primary-Node-wg",
      "start",
    );

    match error {
      WireGuardError::TunnelNotFound { tunnel_name, .. } => {
        assert_eq!(tunnel_name, "Primary-Node-wg");
      }
      other => panic!("expected TunnelNotFound, got {other}"),
    }
  }

  #[test]
  fn map_sc_tunnel_error_uses_start_specific_guidance() {
    let error = super::map_sc_tunnel_error(
      CommandExecError::NonZeroExit {
        program: "sc".to_string(),
        code: Some(1053),
        detail: "The service did not respond to the start or control request in a timely fashion."
          .to_string(),
      },
      "WireGuardTunnel$Primary-Node-wg",
      "start",
    );

    match error {
      WireGuardError::Command { command, detail, .. } => {
        assert!(command.contains("sc start"));
        assert!(detail.contains("Could not start the WireGuard tunnel service"));
        assert!(detail.contains("wait a few seconds and retry once"));
      }
      other => panic!("expected Command error, got {other}"),
    }
  }

  #[derive(Clone, Default)]
  struct MockStoppedServiceRunner;

  impl CommandRunner for MockStoppedServiceRunner {
    fn run(
      &self,
      program: &str,
      args: &[&str],
      _stdin: Option<&str>,
    ) -> Result<CommandOutput, CommandExecError> {
      let program_name = std::path::Path::new(program)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(program);

      if program_name == "wg" && args == ["-v"] {
        return Ok(CommandOutput {
          stdout: "wireguard-tools v1".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "sc" && args.len() == 2 && args[0] == "query" {
        return Ok(CommandOutput {
          stdout: "STATE              : 1  STOPPED".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "wg" && args.len() >= 2 && args[0] == "show" {
        return Err(CommandExecError::NonZeroExit {
          program: "wg".to_string(),
          code: Some(1),
          detail: "wg show should not be called when service is stopped".to_string(),
        });
      }

      Err(CommandExecError::NonZeroExit {
        program: program.to_string(),
        code: Some(1),
        detail: "unsupported mock command".to_string(),
      })
    }
  }

  #[test]
  fn read_tunnel_status_skips_wg_show_when_service_is_stopped() {
    let wireguard = WireGuardManager::with_runner(Arc::new(MockStoppedServiceRunner));
    let status = wireguard
      .read_tunnel_status("Primary-Node-wg")
      .expect("status read should succeed for stopped service");

    assert_eq!(status.state, TunnelServiceState::Stopped);
    assert_eq!(status.last_handshake_age_seconds, None);
    assert_eq!(status.rx_bytes, None);
    assert_eq!(status.tx_bytes, None);
  }

  #[derive(Clone, Default)]
  struct MockCaptureFallbackRunner;

  impl CommandRunner for MockCaptureFallbackRunner {
    fn run(
      &self,
      program: &str,
      args: &[&str],
      _stdin: Option<&str>,
    ) -> Result<CommandOutput, CommandExecError> {
      let program_name = std::path::Path::new(program)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(program);

      if program_name == "wg" && args == ["-v"] {
        return Ok(CommandOutput {
          stdout: "wireguard-tools v1".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "wg" && args == ["showconf", "Primary-Node-wg"] {
        return Err(CommandExecError::NonZeroExit {
          program: "wg".to_string(),
          code: Some(1),
          detail: "showconf unavailable for tunnel".to_string(),
        });
      }

      if program_name == "wg" && args == ["show", "Primary-Node-wg", "dump"] {
        return Ok(CommandOutput {
          stdout: "interface\tpub\t51820\toff\npeerpub\t(none)\t127.0.0.1:51820\t0.0.0.0/0\t1700000050\t10\t20\t25\n".to_string(),
          stderr: String::new(),
        });
      }

      Err(CommandExecError::NonZeroExit {
        program: program.to_string(),
        code: Some(1),
        detail: "unsupported mock command".to_string(),
      })
    }
  }

  #[test]
  fn capture_runtime_trace_falls_back_to_wg_dump_when_showconf_fails() {
    let wireguard = WireGuardManager::with_runner(Arc::new(MockCaptureFallbackRunner));

    wireguard
      .capture_runtime_connect_trace_snapshot("Primary-Node-wg")
      .expect("runtime capture should fall back to wg dump");

    let snapshot = wireguard.debug_snapshot();
    assert_eq!(
      snapshot.last_import_tunnel_name.as_deref(),
      Some("Primary-Node-wg")
    );
    assert_eq!(
      snapshot.last_import_config_temp_path.as_deref(),
      Some("runtime://wg_show_dump")
    );
    assert!(snapshot
      .last_import_config_redacted
      .as_deref()
      .is_some_and(|value| value.contains("peerpub")));
    assert!(snapshot
      .commands
      .iter()
      .any(|entry| entry.contains("showconf Primary-Node-wg")));
    assert!(snapshot
      .commands
      .iter()
      .any(|entry| entry.contains("show Primary-Node-wg dump")));
  }

  #[derive(Clone, Default)]
  struct MockStartFailureButRunningRunner;

  impl CommandRunner for MockStartFailureButRunningRunner {
    fn run(
      &self,
      program: &str,
      args: &[&str],
      _stdin: Option<&str>,
    ) -> Result<CommandOutput, CommandExecError> {
      let program_name = std::path::Path::new(program)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(program);

      if program_name == "wg" && args == ["-v"] {
        return Ok(CommandOutput {
          stdout: "wireguard-tools v1".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "sc" && args == ["start", "WireGuardTunnel$Primary-Node-wg"] {
        return Err(CommandExecError::NonZeroExit {
          program: "sc".to_string(),
          code: Some(1),
          detail: "The service did not respond to the start request in a timely fashion.".to_string(),
        });
      }

      if program_name == "sc" && args == ["query", "WireGuardTunnel$Primary-Node-wg"] {
        return Ok(CommandOutput {
          stdout: "STATE              : 4  RUNNING".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "wg" && args == ["show", "Primary-Node-wg", "dump"] {
        return Ok(CommandOutput {
          stdout: "interface\tpublic\t51820\toff\npeerpub\t(none)\t203.0.113.10:51820\t0.0.0.0/0\t1700000000\t128\t256\t25\n".to_string(),
          stderr: String::new(),
        });
      }

      Err(CommandExecError::NonZeroExit {
        program: program.to_string(),
        code: Some(1),
        detail: "unsupported mock command".to_string(),
      })
    }
  }

  #[test]
  fn activate_tunnel_treats_start_error_as_success_when_service_is_running() {
    let wireguard = WireGuardManager::with_runner(Arc::new(MockStartFailureButRunningRunner));

    let result = wireguard.activate_tunnel("Primary-Node-wg");

    assert!(result.is_ok());
  }

  #[derive(Clone, Default)]
  struct MockDelayedStartRunner {
    query_count: Arc<Mutex<usize>>,
  }

  impl CommandRunner for MockDelayedStartRunner {
    fn run(
      &self,
      program: &str,
      args: &[&str],
      _stdin: Option<&str>,
    ) -> Result<CommandOutput, CommandExecError> {
      let program_name = std::path::Path::new(program)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(program);

      if program_name == "wg" && args == ["-v"] {
        return Ok(CommandOutput {
          stdout: "wireguard-tools v1".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "sc" && args == ["query", "WireGuardTunnel$Primary-Node-wg"] {
        let mut query_count = self.query_count.lock().expect("lock must work");
        *query_count += 1;
        if *query_count < 2 {
          return Ok(CommandOutput {
            stdout: "STATE              : 2  START_PENDING".to_string(),
            stderr: String::new(),
          });
        }

        return Ok(CommandOutput {
          stdout: "STATE              : 4  RUNNING".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "sc" && args == ["start", "WireGuardTunnel$Primary-Node-wg"] {
        panic!("activate_tunnel should not call sc start while service is already starting");
      }

      if program_name == "wg" && args == ["show", "Primary-Node-wg", "dump"] {
        return Ok(CommandOutput {
          stdout: "interface\tpublic\t51820\toff\npeerpub\t(none)\t203.0.113.10:51820\t0.0.0.0/0\t1700000000\t128\t256\t25\n".to_string(),
          stderr: String::new(),
        });
      }

      Err(CommandExecError::NonZeroExit {
        program: program.to_string(),
        code: Some(1),
        detail: "unsupported mock command".to_string(),
      })
    }
  }

  #[test]
  fn activate_tunnel_waits_for_service_to_report_running() {
    let wireguard = WireGuardManager::with_runner(Arc::new(MockDelayedStartRunner::default()));

    let result = wireguard.activate_tunnel("Primary-Node-wg");

    assert!(result.is_ok());
  }

  #[test]
  fn derive_public_key_rejects_invalid_private_key_input() {
    let wireguard = WireGuardManager::new();
    let result = wireguard.derive_public_key("not-a-key");

    assert!(matches!(result, Err(WireGuardError::InvalidOutput { .. })));
  }

  #[test]
  fn generate_client_keypair_returns_non_empty_keys() {
    let runner = MockImportRunner::with_service_exists(false);
    let wireguard = WireGuardManager::with_runner(Arc::new(runner));

    let keypair = wireguard
      .generate_client_keypair()
      .expect("key generation should succeed when wireguard runtime is present");

    assert!(!keypair.private_key.trim().is_empty());
    assert!(!keypair.public_key.trim().is_empty());
  }

  #[test]
  fn runtime_parser_extracts_interface_and_peer_fields() {
    let manager = WireGuardManager::with_runner(Arc::new(MockImportRunner::with_service_exists(false)));
    let keypair = manager.generate_client_keypair().expect("key generation should succeed");
    let peer_public = manager
      .derive_public_key(&keypair.private_key)
      .expect("public key derivation should succeed");
    let config = format!(
      "[Interface]\nPrivateKey = {}\nAddress = 10.66.0.2/32, 2001:db8::2/128\nDNS = 1.1.1.1\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = 203.0.113.10:51820\nPersistentKeepalive = 25\n",
      keypair.private_key, peer_public
    );
    let parsed = parse_interface_config_from_wg_text(
      "wg-node-1",
      &config,
    )
    .expect("parser should succeed");

    assert_eq!(parsed.interface.name, "wg-node-1");
    assert_eq!(parsed.interface.addresses.len(), 2);
    assert_eq!(parsed.interface.peers.len(), 1);
    assert_eq!(parsed.interface.peers[0].allowed_ips.len(), 2);
    assert_eq!(parsed.interface.peers[0].persistent_keepalive_interval, Some(25));
    assert_eq!(parsed.dns_servers.len(), 1);
  }

  #[test]
  fn runtime_parser_rejects_missing_peer_endpoint() {
    let manager = WireGuardManager::with_runner(Arc::new(MockImportRunner::with_service_exists(false)));
    let keypair = manager.generate_client_keypair().expect("key generation should succeed");
    let peer_public = manager
      .derive_public_key(&keypair.private_key)
      .expect("public key derivation should succeed");
    let config = format!(
      "[Interface]\nPrivateKey = {}\nAddress = 10.66.0.2/32\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 0.0.0.0/0\n",
      keypair.private_key, peer_public
    );
    let result = parse_interface_config_from_wg_text(
      "wg-node-1",
      &config,
    );

    assert!(matches!(result, Err(WireGuardError::InvalidOutput { .. })));
  }

  #[test]
  fn runtime_parser_rejects_invalid_dns_value() {
    let manager = WireGuardManager::with_runner(Arc::new(MockImportRunner::with_service_exists(false)));
    let keypair = manager.generate_client_keypair().expect("key generation should succeed");
    let peer_public = manager
      .derive_public_key(&keypair.private_key)
      .expect("public key derivation should succeed");
    let config = format!(
      "[Interface]\nPrivateKey = {}\nAddress = 10.66.0.2/32\nDNS = not-an-ip\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 0.0.0.0/0\nEndpoint = 203.0.113.10:51820\n",
      keypair.private_key, peer_public
    );
    let result = parse_interface_config_from_wg_text(
      "wg-node-1",
      &config,
    );

    assert!(matches!(result, Err(WireGuardError::InvalidOutput { .. })));
  }

  #[test]
  fn legacy_unregister_cleanup_skips_when_service_missing() {
    let runner = MockImportRunner::with_service_exists(false);
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    let result = wireguard.unregister_legacy_tunnel_service_with_program("wg-node-1", "wireguard");
    assert!(result.is_ok());

    let calls = runner.calls();
    assert!(!calls.iter().any(|(_, args)| {
      args.first().is_some_and(|arg| arg == "/uninstalltunnelservice")
    }));
  }

  #[test]
  fn legacy_unregister_cleanup_uninstalls_when_service_exists() {
    let runner = MockImportRunner::with_service_exists(true);
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    let result = wireguard.unregister_legacy_tunnel_service_with_program("wg-node-1", "wireguard");
    assert!(result.is_ok());

    let calls = runner.calls();
    assert!(calls.iter().any(|(_, args)| {
      args == &["/uninstalltunnelservice".to_string(), "wg-node-1".to_string()]
    }));
  }

  #[test]
  fn adapter_not_found_detector_matches_expected_io_patterns() {
    let explicit_not_found = io::Error::new(io::ErrorKind::NotFound, "The system cannot find the file specified.");
    let text_hint = io::Error::other("cannot find requested adapter");
    let unrelated = io::Error::other("permission denied");

    assert!(is_adapter_not_found_error(&explicit_not_found));
    assert!(is_adapter_not_found_error(&text_hint));
    assert!(!is_adapter_not_found_error(&unrelated));
  }
}
