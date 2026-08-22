use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// `defguard_wireguard_rs` is retained for key generation only. Its interface runtime, and the
// `wireguard-nt` driver bindings alongside it, were removed: the adapter they created was dropped
// the moment the creating function returned, `read_interface_data` could never succeed because the
// API was constructed without an adapter, and the DLL was loaded from a CWD-relative path behind a
// `LazyLock` that poisons the process on failure. Tunnels are managed through the WireGuard for
// Windows service instead, which is the only mechanism that has ever worked here.
use defguard_wireguard_rs::key::Key as WireguardKey;
use serde::Serialize;
use thiserror::Error;


pub const DEFAULT_CLIENT_ALLOWED_IPS: &[&str] = &["0.0.0.0/0", "::/0"];
pub const DEFAULT_CLIENT_KEEPALIVE_SECONDS: u16 = 25;
const ACTIVATE_POLL_ATTEMPTS: usize = 60;
const ACTIVATE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const TRACE_HISTORY_LIMIT: usize = 120;

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

  /// Substitutes the process runner so tests can drive `wireguard.exe` and `sc` without either
  /// being installed, and without elevation.
  #[cfg(test)]
  pub fn with_runner(runner: Arc<dyn CommandRunner>) -> Self {
    Self {
      runner,
      debug_trace: Arc::new(Mutex::new(WireGuardDebugTrace::default())),
    }
  }

  pub fn debug_snapshot(&self) -> WireGuardDebugSnapshot {
    let guard = self
      .debug_trace
      .lock()
      .expect("wireguard debug trace lock should work");

    WireGuardDebugSnapshot {
      commands: guard.commands.iter().cloned().collect(),
      last_import_tunnel_name: guard.last_import_tunnel_name.clone(),
      last_import_config: guard.last_import_config.clone(),
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
      .push_back(format!("{} | {}", epoch_seconds_label(), command));
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
    // Stored verbatim. WireGuard key material is not a secret in this app (AGENTS.md §6), and the
    // redacted form blanked the one line most worth reading when a tunnel came up wrong.
    guard.last_import_config = Some(config_text.to_string());
  }

  pub fn record_connect_trace_snapshot(&self, tunnel_name: &str, config_text: &str) {
    self.record_last_import_snapshot(
      tunnel_name,
      "in-memory://connect_tunnel_config",
      config_text,
    );
  }

  /// Capture the live interface state into the diagnostics trace.
  ///
  /// Reads through `wg`, falling back from `showconf` to `show … dump`. There used to be a second
  /// branch here that read the same data through the in-process WireGuard runtime, but that path
  /// could never succeed — it created a `WGApi` without an adapter, so it returned
  /// `AdapterNotFound` every time.
  pub fn capture_runtime_connect_trace_snapshot(
    &self,
    tunnel_name: &str,
  ) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

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

  pub fn ensure_installed(&self) -> Result<(), WireGuardError> {
    if !cfg!(target_os = "windows") {
      return Err(WireGuardError::UnsupportedPlatform {
        detail: "WireGuard integration is supported on Windows only.".to_string(),
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
      // Best-effort: `wg` may be absent from PATH even when `wireguard.exe` resolves, and an
      // unknown version is not a readiness failure.
      version: self.read_wireguard_version(),
      message,
      action_guidance,
    }
  }

  /// The installed WireGuard version, from `wg --version`.
  ///
  /// Reported for display only. `wg` needs no elevation for `--version`, unlike the interface
  /// reads that talk to the tunnel service pipe.
  fn read_wireguard_version(&self) -> Option<String> {
    let output = self.runner.run("wg", &["--version"], None).ok()?;
    let line = output.stdout.lines().next()?.trim();

    if line.is_empty() {
      return None;
    }

    Some(line.to_string())
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

  // Key generation lives in `crypto::keypair`, which every production path uses. A second
  // generator here needed a WireGuard install just to make a keypair — the reason `crypto` exists
  // is that generation is pure Rust and must not depend on `wg.exe` being on PATH.

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
    self.import_tunnel_service(tunnel_name, config_text)
  }

  pub fn activate_tunnel(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.activate_tunnel_service(tunnel_name)
  }

  /// Service state plus adapter byte counters — everything the phase verdict needs, and nothing
  /// that costs more than a single `sc query`.
  ///
  /// Deliberately skips `ensure_installed`: that probe spawns `wireguard.exe /help` to produce a
  /// good "not installed" error before a *write*, but a status read that finds no service already
  /// has the right answer. This runs once a second during a connect, so a second spawn per call
  /// would be doubling the cost of the thing added to make polling cheap.
  ///
  /// Deliberately never calls `wg show`: it requires administrator (AGENTS.md §4), so the verdict
  /// this feeds is built not to need it. `adapter_byte_counters` reads the same interface the
  /// tunnel uses through Win32 `GetIfTable2`, in-process and unelevated.
  pub fn read_tunnel_phase_inputs(
    &self,
    tunnel_name: &str,
  ) -> Result<TunnelPhaseInputs, WireGuardError> {
    validate_tunnel_name(tunnel_name)?;

    let state = self.probe_tunnel_service_state(tunnel_name)?;

    // Counters only mean anything once the interface exists; asking earlier just costs a lookup.
    let (rx_bytes, tx_bytes) = if state == TunnelServiceState::Running {
      match adapter_byte_counters(tunnel_name) {
        Some((rx, tx)) => (Some(rx), Some(tx)),
        None => (None, None),
      }
    } else {
      (None, None)
    };

    Ok(TunnelPhaseInputs {
      state,
      rx_bytes,
      tx_bytes,
    })
  }

  fn wait_for_running_tunnel(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    let mut last_state = TunnelServiceState::Unknown;

    for _ in 0..ACTIVATE_POLL_ATTEMPTS {
      match self.probe_tunnel_service_state(tunnel_name) {
        Ok(TunnelServiceState::Running) => return Ok(()),
        Ok(state) => {
          last_state = state;
          thread::sleep(ACTIVATE_POLL_INTERVAL);
        }
        Err(_) => {
          // No elevated re-query here. This loop used to escalate to a UAC prompt on an
          // access-denied `sc query`, which put a consent dialog inside a polling loop. Reading
          // service state does not require elevation, so a failure here is a real error worth
          // reporting through the timeout below rather than prompting the user 60 times.
          thread::sleep(ACTIVATE_POLL_INTERVAL);
        }
      }
    }

    Err(WireGuardError::InvalidOutput {
      detail: format!(
        "WireGuard tunnel '{tunnel_name}' did not reach Running after activation; last observed service state was {}. Check whether the tunnel is still starting, disabled, or blocked by a bad tunnel configuration.",
        last_state.as_str()
      ),
    })
  }

  fn probe_tunnel_service_state(
    &self,
    tunnel_name: &str,
  ) -> Result<TunnelServiceState, WireGuardError> {
    let service_name = wireguard_service_name(tunnel_name);
    self.record_command(&format!("sc query {}", service_name));
    let service_out = self
      .runner
      .run("sc", &["query", &service_name], None)
      .map_err(|source| map_sc_tunnel_error(source, &service_name, "query"))?;

    // An `sc query` whose output carries no STATE line is an absence of evidence, not a stopped
    // service. The activation poll treats that as "keep waiting"; the phase verdict treats it as
    // `Connecting`. Neither may read it as success.
    parse_sc_state(&service_out.stdout).or(Ok(TunnelServiceState::Unknown))
  }

  /// Disconnect goes through `unregister_tunnel` alone: `/uninstalltunnelservice` stops the
  /// service as part of removing it, so a separate deactivate step only bought a second UAC
  /// prompt (AGENTS.md §4). The stop-without-remove pair it used to call is gone.
  pub fn unregister_tunnel(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.unregister_tunnel_service(tunnel_name)
  }

  /// Run a command directly, escalating to a UAC prompt only if it is refused for permissions.
  ///
  /// This is the single elevation policy for the whole module. Two rules matter:
  ///
  /// - **Try direct first.** An elevated app, or a machine where the user already has the rights,
  ///   completes with no prompt at all. A previous version elevated unconditionally on Windows,
  ///   so every connect and disconnect fired a consent dialog even when it was unnecessary.
  /// - **Escalate only on access-denied.** Any other failure is a real error and is returned as
  ///   such, rather than being retried behind a prompt that cannot fix it.
  fn run_elevating_on_access_denied(
    &self,
    program: &str,
    args: &[&str],
  ) -> Result<CommandOutput, CommandExecError> {
    match self.runner.run(program, args, None) {
      Ok(output) => Ok(output),
      Err(source) => {
        let refused_for_permissions = matches!(
          &source,
          CommandExecError::NonZeroExit { detail, .. } if is_windows_access_denied_detail(detail)
        );

        if !refused_for_permissions {
          return Err(source);
        }

        self.record_command(&format!("{} {} (retry via UAC)", program, args.join(" ")));
        run_wireguard_via_uac(program, args)
      }
    }
  }

  fn import_tunnel_service(&self, tunnel_name: &str, config_text: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    let wireguard_program = self.resolve_wireguard_program()?;
    if self.tunnel_service_exists(tunnel_name)? {
      self.unregister_legacy_tunnel_service_with_program(tunnel_name, &wireguard_program)?;
    }

    let config_dir = tunnel_config_dir()?;
    let config_path = config_dir.join(format!("{tunnel_name}.conf"));
    fs::write(&config_path, config_text).map_err(|source| WireGuardError::Io {
      action: "write_tunnel_config",
      detail: format!(
        "Could not write WireGuard config '{}'.",
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

    let result = self
      .run_elevating_on_access_denied(&wireguard_program, &install_args)
      .map_err(|source| WireGuardError::Command {
        command: format!("wireguard /installtunnelservice {config_path_string}"),
        detail:
          "Could not import WireGuard tunnel service. If prompted, approve administrator elevation."
            .to_string(),
        source,
      });

    // The config file is deliberately left in place.
    //
    // `/installtunnelservice` records this path in the service's own command line — it does not
    // copy the file anywhere. Deleting it after import, which is what this used to do, produced a
    // tunnel service pointing at a file that no longer existed: the import appeared to succeed and
    // every subsequent start failed. Configs live under a stable app-owned directory now, and are
    // removed when the tunnel is unregistered.
    if result.is_err() {
      // Nothing registered, so leave no private key on disk.
      let _ = fs::remove_file(&config_path);
    }

    result.map(|_| ())
  }

  fn activate_tunnel_service(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;

    if let Ok(state) = self.probe_tunnel_service_state(tunnel_name) {
      if matches!(state, TunnelServiceState::Running | TunnelServiceState::Starting) {
        return self.wait_for_running_tunnel(tunnel_name);
      }
    }

    let service_name = wireguard_service_name(tunnel_name);
    self.record_command(&format!("sc start {}", service_name));
    let start_result = self
      .run_elevating_on_access_denied("sc", &["start", &service_name])
      .map_err(|source| map_sc_tunnel_error(source, &service_name, "start"));

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

  /// Remove the tunnel service. `wireguard.exe /uninstalltunnelservice` stops it first.
  ///
  /// This used to run `sc stop` beforehand, which was redundant and expensive: each elevated call
  /// is a separate consent prompt, so unregistering cost two where one would do. Combined with
  /// the caller that also stopped the tunnel before unregistering, disconnect fired several
  /// prompts in a row.
  fn unregister_tunnel_service(&self, tunnel_name: &str) -> Result<(), WireGuardError> {
    self.ensure_installed()?;
    validate_tunnel_name(tunnel_name)?;
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
    self.ensure_installed()?;
    self.read_tunnel_status_service(tunnel_name)
  }

  /// Read service state and peer metrics.
  ///
  /// `sc query` needs no elevation and decides whether the tunnel is up. `wg show` does need
  /// administrator, so peer metrics degrade through the tiers described below rather than failing.
  fn read_tunnel_status_service(&self, tunnel_name: &str) -> Result<TunnelRuntimeStatus, WireGuardError> {
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
        metrics_source: None,
      });
    }

    // Peer metrics are supplementary, and reading them is a privileged operation.
    //
    // `wg show` talks to the tunnel service's pipe, which is restricted to administrators, so an
    // unelevated app gets "Unable to access interface: Permission denied". Treating that as fatal
    // failed the whole connect on a tunnel whose service had installed and started correctly —
    // the app reported an error for a working tunnel.
    //
    // Whether the tunnel is up is decided by `sc query`, which needs no elevation. Handshake age
    // and byte counters are enrichment: absent when they cannot be read, never a reason to fail.
    self.record_command(&format!("wg show {} dump", tunnel_name));
    let peer_metrics = match self.runner.run("wg", &["show", tunnel_name, "dump"], None) {
      Ok(dump) => {
        let now_unix_seconds = SystemTime::now()
          .duration_since(UNIX_EPOCH)
          .map(|value| value.as_secs())
          .unwrap_or(0);

        parse_wg_dump_metrics(now_unix_seconds, &dump.stdout).unwrap_or_else(|_| {
          self.record_command(&format!("wg show {} dump -> unparseable output", tunnel_name));
          PeerMetrics::unavailable()
        })
      }
      Err(source) => {
        // Recorded rather than raised, so the reason is visible in diagnostics.
        self.record_command(&format!(
          "wg show {} dump -> metrics unavailable ({})",
          tunnel_name,
          map_wg_show_dump_error(source, tunnel_name)
        ));

        // Fall back to the adapter's own counters, which need no elevation. Throughput survives;
        // handshake age does not, because only the tunnel service knows it.
        match adapter_byte_counters(tunnel_name) {
          Some((rx_bytes, tx_bytes)) => {
            self.record_command(&format!(
              "adapter counters {} -> rx={} tx={} (handshake age needs elevation)",
              tunnel_name, rx_bytes, tx_bytes
            ));
            PeerMetrics::from_adapter_counters(rx_bytes, tx_bytes)
          }
          None => PeerMetrics::unavailable(),
        }
      }
    };

    Ok(TunnelRuntimeStatus {
      tunnel_name: tunnel_name.to_string(),
      state,
      last_handshake_age_seconds: peer_metrics.last_handshake_age_seconds,
      rx_bytes: peer_metrics.rx_bytes,
      tx_bytes: peer_metrics.tx_bytes,
      metrics_source: peer_metrics.metrics_source,
    })
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
    let result = self
      .run_elevating_on_access_denied(wireguard_program, &uninstall_args)
      .map_err(|source| WireGuardError::Command {
      command: format!("wireguard /uninstalltunnelservice {tunnel_name}"),
      detail:
        "Could not unregister WireGuard tunnel service. If prompted, approve administrator elevation for cleanup."
          .to_string(),
      source,
    });

    let outcome = match result {
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
    };

    // The service is gone (or was never there), so its config should not outlive it — it holds
    // the client private key.
    if outcome.is_ok() {
      remove_tunnel_config(tunnel_name);
    }

    outcome
  }
}

#[derive(Debug, Clone, Default)]
struct WireGuardDebugTrace {
  commands: VecDeque<String>,
  last_import_tunnel_name: Option<String>,
  last_import_config: Option<String>,
  last_import_config_temp_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireGuardDebugSnapshot {
  pub commands: Vec<String>,
  pub last_import_tunnel_name: Option<String>,
  pub last_import_config: Option<String>,
  pub last_import_config_temp_path: Option<String>,
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

/// What `sc query` says about a tunnel's service.
///
/// `Starting` (`START_PENDING`) exists because the activation poll always knew about it while the
/// status read discarded it — two parsers of the same output, one of which threw away the single
/// state that distinguishes "coming up" from "wedged". They are now one parser (`parse_sc_state`).
///
/// Serialized lowercase: this crosses the IPC boundary inside `TunnelPhaseSnapshot`, and the
/// derive without a rename would emit `"Running"` where every other enum on that boundary is
/// lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelServiceState {
  Running,
  /// `START_PENDING` — the service was asked to start and has not finished.
  Starting,
  Stopped,
  Unknown,
}

impl TunnelServiceState {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Running => "running",
      Self::Starting => "start_pending",
      Self::Stopped => "stopped",
      Self::Unknown => "unknown",
    }
  }
}

/// Which measurement produced the byte counters in a status read.
///
/// The two tiers count different things: `wg show` reports per-peer WireGuard payload counters,
/// while the adapter reports whole-interface lifetime octets. A consumer differencing samples
/// over time must reset its baseline when this value changes, or the switch registers as a
/// phantom throughput spike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelMetricsSource {
  /// `wg show <name> dump` — requires administrator.
  Peer,
  /// Win32 `GetIfTable2` interface counters — no elevation needed.
  Adapter,
}

impl TunnelMetricsSource {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Peer => "peer",
      Self::Adapter => "adapter",
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TunnelRuntimeStatus {
  pub tunnel_name: String,
  pub state: TunnelServiceState,
  pub last_handshake_age_seconds: Option<u64>,
  pub rx_bytes: Option<u64>,
  pub tx_bytes: Option<u64>,
  /// `None` when no counters could be read at all.
  pub metrics_source: Option<TunnelMetricsSource>,
}

/// The cheap half of a status read: what `read_tunnel_phase_inputs` can learn for one `sc query`.
///
/// Separate from `TunnelRuntimeStatus` because that type carries `last_handshake_age_seconds`,
/// which cannot be filled without `wg show` and therefore without administrator. Returning it here
/// would invite a caller to read a field that is structurally always `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TunnelPhaseInputs {
  pub state: TunnelServiceState,
  /// Decrypted inbound octets. `Some(0)` means measured and idle; `None` means unreadable.
  pub rx_bytes: Option<u64>,
  pub tx_bytes: Option<u64>,
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
  metrics_source: Option<TunnelMetricsSource>,
}

impl PeerMetrics {
  /// Metrics could not be read. Distinct from "read successfully, no peer activity yet" only in
  /// the diagnostics trace, which records why.
  fn unavailable() -> Self {
    Self {
      last_handshake_age_seconds: None,
      rx_bytes: None,
      tx_bytes: None,
      metrics_source: None,
    }
  }

  /// Throughput recovered from the network adapter when `wg show` is unavailable.
  ///
  /// Handshake age is deliberately absent: it is WireGuard protocol state held by the tunnel
  /// service, and the adapter knows nothing about it.
  fn from_adapter_counters(rx_bytes: u64, tx_bytes: u64) -> Self {
    Self {
      last_handshake_age_seconds: None,
      rx_bytes: Some(rx_bytes),
      tx_bytes: Some(tx_bytes),
      metrics_source: Some(TunnelMetricsSource::Adapter),
    }
  }
}

/// Byte counters for a network adapter, read without elevation.
///
/// `wg show` requires administrator because it talks to the tunnel service's restricted pipe. An
/// adapter's own counters are ordinary interface statistics that any user can read — the numbers
/// Task Manager shows — so throughput stays visible while the app runs unelevated.
///
/// WireGuard for Windows names the adapter after the tunnel, so the tunnel name is the alias to
/// look for.
#[cfg(target_os = "windows")]
fn adapter_byte_counters(adapter_alias: &str) -> Option<(u64, u64)> {
  use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2,
  };

  let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();

  // SAFETY: GetIfTable2 allocates the table and reports success before it is read. The rows form a
  // contiguous array of NumEntries entries, and the allocation is released on every path below.
  unsafe {
    if GetIfTable2(&mut table) != 0 || table.is_null() {
      return None;
    }

    let entry_count = (*table).NumEntries as usize;
    let rows = std::ptr::addr_of!((*table).Table) as *const MIB_IF_ROW2;

    let mut counters = None;
    for index in 0..entry_count {
      let row = &*rows.add(index);
      if wide_string_to_lossy(&row.Alias).eq_ignore_ascii_case(adapter_alias) {
        counters = Some((row.InOctets, row.OutOctets));
        break;
      }
    }

    FreeMibTable(table as *const core::ffi::c_void);
    counters
  }
}

#[cfg(not(target_os = "windows"))]
fn adapter_byte_counters(_adapter_alias: &str) -> Option<(u64, u64)> {
  None
}

/// Decode a fixed-size, NUL-padded UTF-16 field.
#[cfg(target_os = "windows")]
fn wide_string_to_lossy(raw: &[u16]) -> String {
  let end = raw.iter().position(|unit| *unit == 0).unwrap_or(raw.len());
  String::from_utf16_lossy(&raw[..end])
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
  pub stdout: String,
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

    Ok(CommandOutput { stdout })
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
    metrics_source: if peer_count == 0 { None } else { Some(TunnelMetricsSource::Peer) },
  })
}

/// The single parser for `sc query` output.
///
/// There were two: the activation poll recognised `START_PENDING` and the status read collapsed it
/// to `Unknown`, so the same output meant different things depending on which caller looked. That
/// is the divergence AGENTS.md §5 exists to prevent.
fn parse_sc_state(raw_output: &str) -> Result<TunnelServiceState, WireGuardError> {
  for line in raw_output.lines() {
    let trimmed = line.trim();
    if !trimmed.to_ascii_uppercase().starts_with("STATE") {
      continue;
    }

    if trimmed.contains("RUNNING") {
      return Ok(TunnelServiceState::Running);
    }
    if trimmed.contains("START_PENDING") {
      return Ok(TunnelServiceState::Starting);
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

/// Directory holding the tunnel configs this app has registered.
///
/// It must be stable for the lifetime of the tunnel service, because
/// `/installtunnelservice` stores the path in the service command line and reads it on every
/// start. `%TEMP%` was used originally and the file was deleted straight after import, which left
/// services that could never start.
///
/// Under `%LOCALAPPDATA%`, so no elevation is needed to write it, and the per-user ACL keeps the
/// private key out of other users' reach while still readable by the SYSTEM-hosted tunnel service.
/// Where tunnel `.conf` files live, without creating anything.
///
/// Separate from [`tunnel_config_dir`] so that showing the user the path — on the Activity page —
/// does not have the side effect of creating a directory they may never need.
pub fn tunnel_config_dir_path() -> PathBuf {
  let base = std::env::var_os("LOCALAPPDATA")
    .map(PathBuf::from)
    .unwrap_or_else(std::env::temp_dir);

  base.join("GoatStare").join("tunnels")
}

fn tunnel_config_dir() -> Result<PathBuf, WireGuardError> {
  let dir = tunnel_config_dir_path();

  fs::create_dir_all(&dir).map_err(|source| WireGuardError::Io {
    action: "create_tunnel_config_dir",
    detail: format!(
      "Could not create the WireGuard config directory '{}'.",
      dir.display()
    ),
    source,
  })?;

  Ok(dir)
}

/// Remove a tunnel's config once its service is gone, so no private key outlives the tunnel.
fn remove_tunnel_config(tunnel_name: &str) {
  if let Ok(dir) = tunnel_config_dir() {
    let _ = fs::remove_file(dir.join(format!("{tunnel_name}.conf")));
  }
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

/// Unix epoch seconds, labelled and safe to embed in a filename.
///
/// This is not a formatted date despite the name it used to carry; it is a raw second count. It
/// also used a colon separator, which is illegal in NTFS filenames — and the value is used to
/// name the exported diagnostics bundle, so every export failed with `ERROR_INVALID_NAME`.
pub(crate) fn epoch_seconds_label() -> String {
  let epoch = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|value| value.as_secs())
    .unwrap_or(0);
  format!("epoch-{epoch}")
}

#[cfg(test)]
mod tests {
  use super::{
    tunnel_config_dir,
    ClientConfigParams, CommandExecError, CommandOutput, CommandRunner,
    DEFAULT_CLIENT_ALLOWED_IPS, DEFAULT_CLIENT_KEEPALIVE_SECONDS, TunnelServiceState,
    WireGuardError, WireGuardManager, build_client_config,
    is_windows_access_denied_detail,
    is_windows_service_missing_detail, parse_sc_state,
    parse_wg_dump_metrics,
  };
  use std::sync::{Arc, Mutex};

  #[derive(Clone, Default)]
  struct MockImportRunner {
    calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    service_exists: Arc<Mutex<bool>>,
  }

  /// Runner that fails every command with a fixed error, for exercising failure policy.
  #[derive(Clone)]
  struct MockFailingRunner {
    calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    error: Arc<CommandExecError>,
  }

  impl MockFailingRunner {
    fn new(error: CommandExecError) -> Self {
      Self {
        calls: Arc::new(Mutex::new(Vec::new())),
        error: Arc::new(error),
      }
    }

    fn calls(&self) -> Vec<(String, Vec<String>)> {
      self.calls.lock().expect("lock must work").clone()
    }
  }

  impl CommandRunner for MockFailingRunner {
    fn run(
      &self,
      program: &str,
      args: &[&str],
      _stdin: Option<&str>,
    ) -> Result<CommandOutput, CommandExecError> {
      self.calls.lock().expect("lock must work").push((
        program.to_string(),
        args.iter().map(|value| value.to_string()).collect(),
      ));

      // `CommandExecError` is not `Clone` (its spawn variant wraps an `io::Error`), so the stored
      // failure is rebuilt per call. Only the non-zero-exit variant is needed here.
      match self.error.as_ref() {
        CommandExecError::NonZeroExit { program, code, detail } => {
          Err(CommandExecError::NonZeroExit {
            program: program.clone(),
            code: *code,
            detail: detail.clone(),
          })
        }
        other => Err(CommandExecError::NonZeroExit {
          program: program.to_string(),
          code: Some(1),
          detail: other.to_string(),
        }),
      }
    }
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
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
        });
      }

      if program_name == "sc" && args.len() == 2 && args[0] == "query" {
        let service_exists = *self.service_exists.lock().expect("lock must work");
        if service_exists {
          return Ok(CommandOutput {
            stdout: "STATE              : 1  STOPPED".to_string(),
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
        });
      }

      if program_name == "wireguard" && args.len() == 2 && args[0] == "/installtunnelservice" {
        let file_name = std::path::Path::new(args[1])
          .file_name()
          .and_then(|value| value.to_str())
          .unwrap_or("");
        // The point is that the config is named after the tunnel, not after any one test's tunnel.
        if !file_name.ends_with(".conf") || !file_name.starts_with("wg-node-") {
          return Err(CommandExecError::NonZeroExit {
            program: "wireguard".to_string(),
            code: Some(1),
            detail: format!("unexpected config path {file_name}"),
          });
        }

        *self.service_exists.lock().expect("lock must work") = true;
        return Ok(CommandOutput {
          stdout: "installed".to_string(),
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

  /// The settings must reach the emitted config, not just the database.
  #[test]
  fn client_config_carries_the_configured_dns_and_keepalive() {
    let config = build_client_config(&ClientConfigParams {
      private_key: "client-private".to_string(),
      client_address_cidr: "10.66.0.2/32".to_string(),
      dns_server: Some("9.9.9.9".to_string()),
      server_public_key: "server-public".to_string(),
      server_endpoint_ip: "203.0.113.10".to_string(),
      server_listen_port: 51820,
      allowed_ips: vec!["0.0.0.0/0".to_string()],
      persistent_keepalive_seconds: 15,
    })
    .expect("config should render");

    assert!(config.contains("DNS = 9.9.9.9"), "config was: {config}");
    assert!(config.contains("PersistentKeepalive = 15"), "config was: {config}");
  }

  /// Clearing DNS must omit the line entirely — an empty `DNS = ` is not valid config.
  #[test]
  fn client_config_omits_the_dns_line_when_no_server_is_configured() {
    let config = build_client_config(&ClientConfigParams {
      private_key: "client-private".to_string(),
      client_address_cidr: "10.66.0.2/32".to_string(),
      dns_server: None,
      server_public_key: "server-public".to_string(),
      server_endpoint_ip: "203.0.113.10".to_string(),
      server_listen_port: 51820,
      allowed_ips: vec!["0.0.0.0/0".to_string()],
      persistent_keepalive_seconds: 25,
    })
    .expect("config should render");

    assert!(!config.contains("DNS"), "config was: {config}");
  }

  #[test]
  fn parse_sc_state_detects_running_and_stopped() {
    let running = "STATE              : 4  RUNNING";
    let stopped = "STATE              : 1  STOPPED";

    assert_eq!(parse_sc_state(running).expect("running parse"), TunnelServiceState::Running);
    assert_eq!(parse_sc_state(stopped).expect("stopped parse"), TunnelServiceState::Stopped);
  }

  /// `START_PENDING` used to be visible to the activation poll and discarded by the status read,
  /// which is how the same `sc query` output meant two different things.
  #[test]
  fn parse_sc_state_detects_start_pending() {
    let starting = "STATE              : 2  START_PENDING";

    assert_eq!(
      parse_sc_state(starting).expect("start_pending parse"),
      TunnelServiceState::Starting
    );
  }

  #[test]
  fn parse_sc_state_reports_an_unrecognised_state_rather_than_guessing() {
    let pausing = "STATE              : 6  PAUSE_PENDING";

    assert_eq!(parse_sc_state(pausing).expect("parse"), TunnelServiceState::Unknown);
  }

  /// The connect-progress poll runs once a second, so its cost is part of its contract.
  ///
  /// Without this, `read_tunnel_phase_inputs` could quietly regain an `ensure_installed` probe or a
  /// `wg show` call and nobody would notice until the UI froze again.
  #[test]
  fn reading_phase_inputs_costs_exactly_one_sc_query() {
    let runner = Arc::new(MockImportRunner::with_service_exists(true));
    let manager = WireGuardManager::with_runner(runner.clone());

    manager
      .read_tunnel_phase_inputs("wg-node-phase")
      .expect("phase inputs should read");

    let calls = runner.calls();
    let programs: Vec<&str> = calls.iter().map(|(program, _)| program.as_str()).collect();

    assert_eq!(
      programs,
      vec!["sc"],
      "the 1 Hz poll must spawn exactly one process; got {calls:?}"
    );
    assert_eq!(calls[0].1, vec!["query", "WireGuardTunnel$wg-node-phase"]);
  }

  /// Reading service state needs no elevation, and this runs on a timer — prompting for UAC from
  /// inside a polling loop is the failure AGENTS.md §4 calls out by name.
  #[test]
  fn reading_phase_inputs_never_elevates_when_the_service_is_missing() {
    let runner = Arc::new(MockImportRunner::with_service_exists(false));
    let manager = WireGuardManager::with_runner(runner.clone());

    let result = manager.read_tunnel_phase_inputs("wg-node-phase");

    assert!(
      matches!(result, Err(WireGuardError::TunnelNotFound { .. })),
      "a missing service must surface as TunnelNotFound, got {result:?}"
    );
    assert!(
      !runner
        .calls()
        .iter()
        .any(|(program, _)| program.eq_ignore_ascii_case("powershell")),
      "status reads must never escalate to a UAC prompt"
    );
  }

  #[test]
  fn phase_inputs_report_no_counters_while_the_service_is_stopped() {
    let runner = Arc::new(MockImportRunner::with_service_exists(true));
    let manager = WireGuardManager::with_runner(runner);

    let inputs = manager
      .read_tunnel_phase_inputs("wg-node-phase")
      .expect("phase inputs should read");

    // The mock reports STOPPED, so there is no adapter to read counters from.
    assert_eq!(inputs.state, TunnelServiceState::Stopped);
    assert_eq!(inputs.rx_bytes, None);
    assert_eq!(inputs.tx_bytes, None);
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

  /// Had no `#[test]` attribute, so it had never run once. The compiler reported it as dead code,
  /// which is only visible with the rest of the dead-code noise cleared away.
  #[test]
  fn import_tunnel_replaces_stale_service_and_uses_tunnel_name_basename() {
    let runner = MockImportRunner::with_service_exists(true);
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    let result = wireguard.import_tunnel(
      "wg-node-replace",
      "[Interface]\nPrivateKey = priv\nAddress = 10.66.0.2/32\n\n[Peer]\nPublicKey = pub\nAllowedIPs = 0.0.0.0/0\nEndpoint = 203.0.113.10:51820\nPersistentKeepalive = 25\n",
    );

    assert!(result.is_ok());

    let calls = runner.calls();
    assert!(calls.iter().any(|(_, args)| args == &["/uninstalltunnelservice".to_string(), "wg-node-replace".to_string()]));

    let install_call = calls
      .iter()
      .find(|(_, args)| args.first().is_some_and(|arg| arg == "/installtunnelservice"))
      .expect("install command should be issued");

    assert!(install_call.1[1].ends_with("wg-node-replace.conf"));
  }

  #[test]
  fn import_tunnel_succeeds_when_service_is_not_preinstalled() {
    let runner = MockImportRunner::with_service_exists(false);
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    let result = wireguard.import_tunnel(
      "wg-node-fresh",
      "[Interface]\nPrivateKey = priv\nAddress = 10.66.0.2/32\n\n[Peer]\nPublicKey = pub\nAllowedIPs = 0.0.0.0/0\nEndpoint = 203.0.113.10:51820\nPersistentKeepalive = 25\n",
    );

    assert!(result.is_ok());

    let calls = runner.calls();
    assert!(!calls.iter().any(|(_, args)| args == &[
      "/uninstalltunnelservice".to_string(),
      "wg-node-fresh".to_string(),
    ]));
    assert!(calls.iter().any(|(_, args)| args.first().is_some_and(|arg| arg == "/installtunnelservice")));
  }

  /// The config a tunnel service points at must outlive the import.
  ///
  /// `/installtunnelservice` records the path in the service command line and reads it on every
  /// start; it does not copy the file. Deleting it after import — which this used to do — created
  /// services that imported cleanly and then failed every start, because the file was gone.
  /// The config snapshot is shown to the user on the Kill Switch page, and it is recorded whole.
  ///
  /// WireGuard key material is not a secret in this app (AGENTS.md §6), so the snapshot keeps the
  /// entire config. Every field has to survive: `Address`, `DNS`, `AllowedIPs` and
  /// `PersistentKeepalive` are how a user confirms a settings change reached the running tunnel
  /// or that the kill switch has a `/0` route, and the key is which peer is actually loaded.
  #[test]
  fn the_config_snapshot_keeps_the_whole_config_including_the_private_key() {
    let wireguard = WireGuardManager::with_runner(Arc::new(MockImportRunner::with_service_exists(false)));

    wireguard.record_connect_trace_snapshot(
      "wg-node-snapshot",
      "[Interface]\nPrivateKey = uP7xK2mQvR4tY6wA9sD1fG3hJ5kL8nB0cV2xZ4aQ6eE=\nAddress = 10.66.0.2/32\nDNS = 9.9.9.9\n\n[Peer]\nPublicKey = serverpubkey\nAllowedIPs = 0.0.0.0/0\nEndpoint = 203.0.113.10:51820\nPersistentKeepalive = 25\n",
    );

    let config = wireguard
      .debug_snapshot()
      .last_import_config
      .expect("the connect trace should record a config");

    assert!(
      config.contains("uP7xK2mQvR4tY6wA9sD1fG3hJ5kL8nB0cV2xZ4aQ6eE="),
      "the key identifies which peer is loaded and must stay readable: {config}"
    );
    assert!(!config.contains("[REDACTED]"), "nothing in a client config is redacted: {config}");

    for setting in [
      "Address = 10.66.0.2/32",
      "DNS = 9.9.9.9",
      "AllowedIPs = 0.0.0.0/0",
      "Endpoint = 203.0.113.10:51820",
      "PersistentKeepalive = 25",
    ] {
      assert!(config.contains(setting), "'{setting}' must stay checkable: {config}");
    }
  }

  //
  // Each filesystem-touching test uses its own tunnel name. They all shared `wg-node-1`, and so
  // shared one path under `%LOCALAPPDATA%\GoatStare\tunnels\` — one test's unregister could delete
  // the file another had just imported, failing at random under the default parallel runner.
  #[test]
  fn imported_config_survives_import_and_is_removed_on_unregister() {
    let runner = MockImportRunner::with_service_exists(false);
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    wireguard
      .import_tunnel(
        "wg-node-lifetime",
        "[Interface]\nPrivateKey = priv\nAddress = 10.66.0.2/32\n\n[Peer]\nPublicKey = pub\nAllowedIPs = 0.0.0.0/0\nEndpoint = 203.0.113.10:51820\nPersistentKeepalive = 25\n",
      )
      .expect("import should succeed");

    // The path handed to /installtunnelservice must still exist afterwards.
    let installed_path = runner
      .calls()
      .into_iter()
      .find(|(_, args)| args.first().is_some_and(|arg| arg == "/installtunnelservice"))
      .and_then(|(_, args)| args.get(1).cloned())
      .expect("install call should record a config path");

    assert!(
      std::path::Path::new(&installed_path).exists(),
      "the registered config must not be deleted after import: {installed_path}"
    );

    wireguard
      .unregister_legacy_tunnel_service_with_program("wg-node-lifetime", "wireguard")
      .expect("unregister should succeed");

    assert!(
      !std::path::Path::new(&installed_path).exists(),
      "the config holds a private key and must not outlive the tunnel: {installed_path}"
    );
  }

  /// Unregistering must issue exactly one privileged command.
  ///
  /// `/uninstalltunnelservice` stops the service as part of removing it, so the `sc stop` that
  /// used to run first was redundant. Each elevated call is its own consent prompt, and with the
  /// caller also stopping the tunnel beforehand, disconnect fired a run of them.
  #[test]
  fn unregister_issues_a_single_privileged_command() {
    let runner = MockImportRunner::with_service_exists(true);
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    wireguard
      .unregister_tunnel("wg-node-single-command")
      .expect("unregister should succeed");

    let calls = runner.calls();

    let stops = calls
      .iter()
      .filter(|(program, args)| program.contains("sc") && args.first().is_some_and(|arg| arg == "stop"))
      .count();
    assert_eq!(stops, 0, "uninstall already stops the service: {calls:?}");

    let uninstalls = calls
      .iter()
      .filter(|(_, args)| args.first().is_some_and(|arg| arg == "/uninstalltunnelservice"))
      .count();
    assert_eq!(uninstalls, 1, "exactly one uninstall should run: {calls:?}");
  }

  /// A failed import must not leave the private key on disk.
  #[test]
  fn config_is_removed_when_import_fails() {
    let runner = MockFailingRunner::new(CommandExecError::NonZeroExit {
      program: "wireguard".to_string(),
      code: Some(1),
      detail: "install refused".to_string(),
    });
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    let _ = wireguard.import_tunnel("wg-fail-1", "[Interface]\nPrivateKey = priv\n");

    let leftover = tunnel_config_dir()
      .expect("config dir should resolve")
      .join("wg-fail-1.conf");

    assert!(
      !leftover.exists(),
      "a failed import must not leave key material behind: {}",
      leftover.display()
    );
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
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
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
      "wg-node-missing-service-text",
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
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
        });
      }

      if program_name == "sc" && args.len() == 2 && args[0] == "query" {
        return Ok(CommandOutput {
          stdout: "STATE              : 1  STOPPED".to_string(),
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

  /// An adapter that does not exist yields no counters, rather than zeroes.
  ///
  /// Zeroes would read as "connected, no traffic", which is a different and misleading claim.
  #[test]
  fn adapter_counters_are_absent_for_an_unknown_adapter() {
    assert_eq!(
      super::adapter_byte_counters("goatstare-no-such-adapter-a1b2c3"),
      None
    );
  }

  #[cfg(target_os = "windows")]
  #[test]
  fn wide_string_decoding_stops_at_the_nul_terminator() {
    // The Win32 field is a fixed-size buffer padded with NULs, not a sized string.
    let mut raw = [0u16; 8];
    for (index, unit) in "wg0".encode_utf16().enumerate() {
      raw[index] = unit;
    }

    assert_eq!(super::wide_string_to_lossy(&raw), "wg0");
    assert_eq!(super::wide_string_to_lossy(&[0u16; 4]), "");
  }

  /// A running tunnel whose metrics cannot be read is still running.
  ///
  /// `wg show` talks to the tunnel service's pipe, which is restricted to administrators, so an
  /// unelevated app gets "Permission denied". Raising that failed connect on a tunnel that had
  /// installed and started correctly — the app reported an error for a working tunnel.
  #[test]
  fn running_tunnel_reports_connected_when_peer_metrics_are_not_readable() {
    /// Service is running, but `wg show` is refused, as it is for a non-elevated caller.
    #[derive(Clone, Default)]
    struct MetricsDeniedRunner;

    impl CommandRunner for MetricsDeniedRunner {
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
          return Ok(CommandOutput { stdout: "wireguard-tools v1".to_string() });
        }

        if program_name == "wireguard" && args == ["/help"] {
          return Ok(CommandOutput { stdout: "WireGuard for Windows".to_string() });
        }

        if program_name == "sc" && args.first().is_some_and(|arg| *arg == "query") {
          return Ok(CommandOutput {
            stdout: "STATE              : 4  RUNNING".to_string(),
          });
        }

        if program_name == "wg" && args.first().is_some_and(|arg| *arg == "show") {
          return Err(CommandExecError::NonZeroExit {
            program: "wg".to_string(),
            code: Some(1),
            detail: "Unable to access interface Primary-Node-wg: Permission denied".to_string(),
          });
        }

        Err(CommandExecError::NonZeroExit {
          program: program_name.to_string(),
          code: Some(1),
          detail: "unsupported mock command".to_string(),
        })
      }
    }

    let wireguard = WireGuardManager::with_runner(Arc::new(MetricsDeniedRunner));

    let status = wireguard
      .read_tunnel_status("Primary-Node-wg")
      .expect("unreadable metrics must not fail the status read");

    assert_eq!(status.state, TunnelServiceState::Running);
    assert_eq!(status.last_handshake_age_seconds, None);
    assert_eq!(status.rx_bytes, None);
    assert_eq!(status.tx_bytes, None);

    // The reason must be recoverable from diagnostics rather than silently dropped.
    let snapshot = wireguard.debug_snapshot();
    assert!(
      snapshot.commands.iter().any(|entry| entry.contains("metrics unavailable")),
      "the trace should record why metrics were missing: {:?}",
      snapshot.commands
    );
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
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
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
      .last_import_config
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
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
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
        });
      }

      if program_name == "wg" && args == ["show", "Primary-Node-wg", "dump"] {
        return Ok(CommandOutput {
          stdout: "interface\tpublic\t51820\toff\npeerpub\t(none)\t203.0.113.10:51820\t0.0.0.0/0\t1700000000\t128\t256\t25\n".to_string(),
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
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
        });
      }

      if program_name == "sc" && args == ["query", "WireGuardTunnel$Primary-Node-wg"] {
        let mut query_count = self.query_count.lock().expect("lock must work");
        *query_count += 1;
        if *query_count < 2 {
          return Ok(CommandOutput {
            stdout: "STATE              : 2  START_PENDING".to_string(),
          });
        }

        return Ok(CommandOutput {
          stdout: "STATE              : 4  RUNNING".to_string(),
        });
      }

      if program_name == "sc" && args == ["start", "WireGuardTunnel$Primary-Node-wg"] {
        panic!("activate_tunnel should not call sc start while service is already starting");
      }

      if program_name == "wg" && args == ["show", "Primary-Node-wg", "dump"] {
        return Ok(CommandOutput {
          stdout: "interface\tpublic\t51820\toff\npeerpub\t(none)\t203.0.113.10:51820\t0.0.0.0/0\t1700000000\t128\t256\t25\n".to_string(),
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

  // Key generation is covered in `crypto::keypair`, which owns it. The duplicate generator that
  // lived here needed a WireGuard install just to make a keypair, which is the coupling the crypto
  // module exists to avoid.

  #[test]
  fn legacy_unregister_cleanup_skips_when_service_missing() {
    let runner = MockImportRunner::with_service_exists(false);
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    let result =
      wireguard.unregister_legacy_tunnel_service_with_program("wg-node-legacy-absent", "wireguard");
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

    let result =
      wireguard.unregister_legacy_tunnel_service_with_program("wg-node-legacy-present", "wireguard");
    assert!(result.is_ok());

    let calls = runner.calls();
    assert!(calls.iter().any(|(_, args)| {
      args == &["/uninstalltunnelservice".to_string(), "wg-node-legacy-present".to_string()]
    }));
  }

  /// A command that succeeds outright must not trigger a UAC prompt.
  ///
  /// Elevation used to be unconditional on Windows, so every connect and disconnect fired a
  /// consent dialog even when the direct call would have worked.
  #[test]
  fn does_not_elevate_when_the_direct_call_succeeds() {
    let runner = MockImportRunner::with_service_exists(false);
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    let result =
      wireguard.run_elevating_on_access_denied("wireguard", &["/installtunnelservice", "wg-node-1.conf"]);
    assert!(result.is_ok(), "direct call should succeed: {result:?}");

    let calls = runner.calls();
    assert!(
      !calls.iter().any(|(program, _)| program == "powershell"),
      "a successful direct call must not escalate to a UAC prompt"
    );
  }

  /// Failures that elevation cannot fix must surface as-is rather than behind a prompt.
  #[test]
  fn does_not_elevate_for_failures_that_are_not_permission_related() {
    let runner = MockFailingRunner::new(CommandExecError::NonZeroExit {
      program: "sc".to_string(),
      code: Some(1060),
      detail: "The specified service does not exist as an installed service.".to_string(),
    });
    let wireguard = WireGuardManager::with_runner(Arc::new(runner.clone()));

    let result = wireguard.run_elevating_on_access_denied("sc", &["stop", "WireGuardTunnel$wg-node-1"]);
    assert!(result.is_err());

    let calls = runner.calls();
    assert!(
      !calls.iter().any(|(program, _)| program == "powershell"),
      "a non-permission failure must not escalate to a UAC prompt"
    );
    assert_eq!(calls.len(), 1, "the command should be attempted exactly once");
  }
}
