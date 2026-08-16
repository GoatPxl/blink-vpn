use thiserror::Error;
use uuid::Uuid;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::backend::bootstrap::{
  Bootstrap, BootstrapError, BootstrapParams, DEFAULT_CLIENT_CIDR, DEFAULT_INTERFACE_NAME,
  DEFAULT_SERVER_INTERFACE_CIDR, DEFAULT_WIREGUARD_PORT,
};
use crate::backend::crypto::keypair::{generate_client_keypair, generate_server_keypair, KeygenError};
use crate::backend::db::Database;
use crate::backend::db::{
  DbError, NewActivityEvent, NodeLifecycleStatus, TunnelRecord, TunnelStatus,
};
use crate::backend::logging;
use crate::backend::provider::{
  BootstrapWaitRequest, NodeInitRequest, ProvisionedDroplet, ProviderDropletStatus, ProviderFlowError,
  ProviderPlanPricing, ProviderRegion, ProviderRegistry, DEFAULT_REGION, DEFAULT_SIZE,
};
use crate::backend::secure_storage::SecureStorage;
use crate::backend::secure_storage::SecretStoreError;
use crate::backend::wireguard::{
  ClientConfigParams, DEFAULT_CLIENT_ALLOWED_IPS, DEFAULT_CLIENT_KEEPALIVE_SECONDS,
  TunnelMetricsSource, TunnelRuntimeStatus, WireGuardError,
  WireGuardManager,
};

#[derive(Clone)]
pub struct Orchestrator {
  ip_probe: Arc<dyn ExternalIpProbe>,
  external_ip: Arc<Mutex<Option<ExternalIpSnapshot>>>,
  tunnel_watch: Arc<Mutex<TunnelWatch>>,
}

/// Tracks whether a tunnel stopping was the app's doing.
///
/// Both a crash and a Disconnect click land on `TunnelStatus::Disconnected` with nothing to tell
/// them apart, so the app could not warn about a drop that left traffic unprotected. Intent has to
/// be recorded at the point it is known.
///
/// In-memory on purpose: a drop is only actionable inside the session that saw it, and startup
/// reconciliation already reports a stale connected row as a repaired issue.
#[derive(Debug, Default)]
struct TunnelWatch {
  /// Last runtime state observed per tunnel, so a transition can be spotted.
  last_connected: HashMap<String, bool>,
  /// Tunnels the app is deliberately stopping; a stop here is not a drop.
  expected_stop: HashSet<String>,
  /// The most recent unexpected stop, until the user reconnects.
  drop: Option<UnexpectedDrop>,
}

/// The last measured egress addresses, and the tunnel state they were measured under.
#[derive(Debug, Clone)]
struct ExternalIpSnapshot {
  ipv4: Option<String>,
  ipv6: Option<String>,
  observed_at: Instant,
  /// Re-probing is forced when this stops matching reality, so connecting updates the card
  /// immediately instead of after the TTL.
  measured_while_connected: bool,
}

const DEFAULT_TUNNEL_BASENAME: &str = "primary";
/// `app_settings` key holding the node the app connects to by default.
const CURRENT_NODE_SETTING_KEY: &str = "current_node_id";
/// `app_settings` key holding the DNS server written into generated tunnel configs.
const DNS_SERVER_SETTING_KEY: &str = "wireguard_dns_server";
/// `app_settings` key holding the `PersistentKeepalive` interval, in seconds.
const KEEPALIVE_SETTING_KEY: &str = "wireguard_keepalive_seconds";
/// Cloudflare's resolver: what every tunnel used when this was hardcoded in the frontend.
const DEFAULT_DNS_SERVER: &str = "1.1.1.1";
const EXTERNAL_IP_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Health polls every few seconds; probing that often would hammer a third-party service for a
/// value that rarely changes. A connection state change bypasses this entirely.
const EXTERNAL_IP_CACHE_TTL: Duration = Duration::from_secs(60);
/// How many activity events to keep. Its predecessor table grew without bound while only
/// provisioning wrote to it; every connect and disconnect appends now, so it needs a ceiling.
const ACTIVITY_HISTORY_LIMIT: usize = 1000;

#[derive(Debug, Clone, Serialize)]
pub struct CreateVpnNodeRequest {
  pub node_id: String,
  pub provider_id: String,
  pub node_name: String,
  pub region: String,
  pub plan_key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateVpnNodeResult {
  pub node_id: String,
  pub status: NodeLifecycleStatus,
  pub droplet_id: u64,
  pub server_endpoint_ip: String,
  pub server_public_key: String,
  pub listen_port: u16,
  pub correlation_id: String,
  pub idempotent_reuse: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateClientTunnelRequest {
  pub tunnel_id: String,
  pub node_id: String,
  pub tunnel_name: String,
  pub client_address_cidr: String,
  pub dns_server: Option<String>,
  pub allowed_ips: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateClientTunnelResult {
  pub tunnel_id: String,
  pub tunnel_name: String,
  pub client_public_key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForceCleanupResult {
  pub node_id: String,
  pub correlation_id: String,
  pub local_tunnels_removed: usize,
  pub remote_droplet_delete_attempted: bool,
  pub remote_droplet_delete_succeeded: bool,
  pub node_record_removed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectTunnelRequest {
  pub node_id: String,
  pub tunnel_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisconnectTunnelRequest {
  pub node_id: String,
  pub tunnel_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ReconcileIssueKind {
  StaleTunnelStatusRepaired,
  MissingDroplet,
  MissingNodeRuntimeData,
  StartupDependencyUnavailable,
  ProviderCheckFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReconcileIssue {
  pub kind: ReconcileIssueKind,
  pub node_id: Option<String>,
  pub tunnel_id: Option<String>,
  pub repaired: bool,
  pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReconcileReport {
  pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum UserFacingErrorKind {
  InvalidState,
  InvalidInput,
  MissingResource,
  Provider,
  WireGuard,
  Persistence,
  Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserFacingError {
  pub kind: UserFacingErrorKind,
  pub message: String,
  pub next_action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthSummary {
  pub node_id: String,
  pub tunnel_id: Option<String>,
  pub endpoint_ip: Option<String>,
  pub handshake_age_seconds: Option<u64>,
  pub rx_bytes: Option<u64>,
  pub tx_bytes: Option<u64>,
  /// The IPv4 address the internet currently sees for this machine, measured in every connection
  /// state. `None` means the probe failed — never the server's own address, because substituting
  /// it would make an unverified state indistinguishable from a verified one.
  pub external_ipv4: Option<String>,
  /// The IPv6 address the internet currently sees, if the machine has IPv6 egress at all.
  ///
  /// This deployment's tunnel is IPv4-only — the bootstrap configures `iptables` MASQUERADE and
  /// IPv4 forwarding, and the client interface has no IPv6 address — so any value present here
  /// while connected is traffic leaving outside the tunnel.
  pub external_ipv6: Option<String>,
  pub tunnel_status: Option<TunnelStatus>,
  /// Which tier produced `rx_bytes`/`tx_bytes`; see `TunnelMetricsSource`.
  pub metrics_source: Option<TunnelMetricsSource>,
  /// When the current session began, or `None` if not connected or the start time is unknown.
  pub connected_at: Option<String>,
  /// Whether untunneled traffic is currently blocked, and why.
  pub kill_switch: KillSwitchState,
  /// Set when the tunnel stopped without the app asking it to; `None` otherwise.
  pub unexpected_drop: Option<UnexpectedDrop>,
}

/// Why leak protection is or is not in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KillSwitchReason {
  /// WireGuard's WFP filters are blocking untunneled traffic.
  Active,
  /// No tunnel service is running, so no filters exist.
  TunnelNotRunning,
  /// The tunnel carries only a scoped route, so WireGuard restricts nothing.
  RoutesScoped,
}

/// Whether untunneled traffic is currently blocked, and by what.
///
/// This app installs no firewall rules of its own. WireGuard for Windows enables WFP filters that
/// block traffic outside the tunnel when a config has exactly one peer, an `AllowedIPs` containing
/// a `/0` route, and no `Table = off`. This reports whether those preconditions actually hold for
/// the running tunnel — not whether the app would like them to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KillSwitchState {
  pub active: bool,
  pub blocks_ipv4: bool,
  pub blocks_ipv6: bool,
  pub reason: KillSwitchReason,
  /// The routes the running config actually carries, so the UI can show its working.
  pub routes: Vec<String>,
}

/// Client tunnel parameters the user can change, resolved from `app_settings`.
///
/// These are rendered into the `.conf` on every connect, so a change takes effect the next time a
/// tunnel starts — a running tunnel keeps whatever it was started with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TunnelSettings {
  /// `None` means no `DNS` line at all, which leaves the system resolver in place.
  pub dns_server: Option<String>,
  /// `0` disables keepalive.
  pub keepalive_seconds: u16,
}

impl Default for TunnelSettings {
  fn default() -> Self {
    Self {
      dns_server: Some(DEFAULT_DNS_SERVER.to_string()),
      keepalive_seconds: DEFAULT_CLIENT_KEEPALIVE_SECONDS,
    }
  }
}

/// A tunnel that stopped without the app asking it to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnexpectedDrop {
  pub tunnel_id: String,
  pub node_id: String,
  /// Unix epoch milliseconds. Named for its unit because the rest of this payload carries ISO-8601
  /// strings from SQLite, and a bare `observed_at` would invite parsing it as one.
  pub observed_at_epoch_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsBundle {
  pub generated_at: String,
  pub providers: Vec<crate::backend::db::ProviderRecord>,
  pub nodes: Vec<crate::backend::db::VpnNodeRecord>,
  pub tunnels: Vec<crate::backend::db::TunnelRecord>,
  pub activity: Vec<crate::backend::db::ActivityEvent>,
  pub redacted_notes: Vec<String>,
  pub wireguard_debug: crate::backend::wireguard::WireGuardDebugSnapshot,
}

/// Writes the activity trail for one run of one flow.
///
/// Holding the run's identity in one place keeps every call site down to stage, outcome and detail,
/// and guarantees the whole run shares a `correlation_id` so the UI can collapse it into a single
/// entry.
struct ActivityRecorder<'a> {
  db: &'a Database,
  node_id: Option<&'a str>,
  provider_slug: Option<&'a str>,
  category: &'a str,
  correlation_id: Option<&'a str>,
}

impl<'a> ActivityRecorder<'a> {
  fn new(db: &'a Database, category: &'a str, correlation_id: Option<&'a str>) -> Self {
    Self {
      db,
      node_id: None,
      provider_slug: None,
      category,
      correlation_id,
    }
  }

  fn for_node(mut self, node_id: &'a str) -> Self {
    self.node_id = Some(node_id);
    self
  }

  fn with_provider(mut self, provider_slug: Option<&'a str>) -> Self {
    self.provider_slug = provider_slug;
    self
  }

  /// Deliberately infallible to the caller: the history is a diagnostic aid, and failing a connect
  /// because the diary could not be written would be the wrong trade.
  fn record(&self, stage: &str, outcome: &str, detail: &str) {
    let _ = self.db.append_activity_event(&NewActivityEvent {
      node_id: self.node_id,
      provider_slug: self.provider_slug,
      category: self.category,
      stage,
      outcome,
      correlation_id: self.correlation_id,
      // Redacted here, at the write boundary, so nothing downstream has to remember to.
      detail: &logging::redact_sensitive_text(detail),
    });
  }

  fn info(&self, stage: &str, detail: &str) {
    self.record(stage, "info", detail);
  }

  fn success(&self, stage: &str, detail: &str) {
    self.record(stage, "success", detail);
  }

  /// Records the flattened `#[source]` chain rather than the outermost message.
  ///
  /// A provider rejecting a droplet name used to be stored as only "Node creation or bootstrap
  /// setup failed", with the actual complaint attached to the error and never printed. The whole
  /// point of keeping this history is that the recorded reason is the one you can act on.
  fn failure(&self, stage: &str, error: &dyn std::error::Error) {
    self.record(stage, "error", &logging::describe_error_chain(error));
  }
}

#[derive(Debug, Error)]
pub enum OrchestratorError {
  #[error("Provider orchestration failed: {detail}")]
  Provider {
    detail: String,
    #[source]
    source: ProviderFlowError,
  },
  #[error("Bootstrap generation failed: {detail}")]
  Bootstrap {
    detail: String,
    #[source]
    source: BootstrapError,
  },
  #[error("Database operation failed: {detail}")]
  Database {
    detail: String,
    #[source]
    source: DbError,
  },
  #[error("Node create flow cannot proceed for node '{node_id}': {detail}")]
  InvalidState { node_id: String, detail: String },
  /// A value the user supplied cannot be used. Distinct from `InvalidState`, which is about the
  /// app's own state rather than something the user typed.
  #[error("{detail}")]
  InvalidInput { detail: String },
  #[error("WireGuard operation failed: {detail}")]
  WireGuard {
    detail: String,
    #[source]
    source: WireGuardError,
  },
  #[error("Secure storage operation failed: {detail}")]
  SecretStorage {
    detail: String,
    #[source]
    source: SecretStoreError,
  },
  #[error("WireGuard keypair generation failed: {detail}")]
  Keygen {
    detail: String,
    #[source]
    source: KeygenError,
  },
}

impl Orchestrator {
  pub fn new() -> Self {
    Self::with_ip_probe(Arc::new(HttpExternalIpProbe))
  }

  pub fn with_ip_probe(ip_probe: Arc<dyn ExternalIpProbe>) -> Self {
    Self {
      ip_probe,
      external_ip: Arc::new(Mutex::new(None)),
      tunnel_watch: Arc::new(Mutex::new(TunnelWatch::default())),
    }
  }

  /// Records that the app is about to stop this tunnel, so the next poll does not call it a drop.
  fn expect_tunnel_stop(&self, tunnel_id: &str) {
    if let Ok(mut watch) = self.tunnel_watch.lock() {
      watch.expected_stop.insert(tunnel_id.to_string());
      watch.drop = None;
    }
  }

  /// Records a successful connect, clearing any drop the user has now recovered from.
  fn note_tunnel_connected(&self, tunnel_id: &str) {
    if let Ok(mut watch) = self.tunnel_watch.lock() {
      watch.expected_stop.remove(tunnel_id);
      watch.last_connected.insert(tunnel_id.to_string(), true);
      watch.drop = None;
    }
  }

  /// Folds one observation into the watch and reports an unexpected stop if this was one.
  ///
  /// A drop is a `connected -> not connected` transition with no matching intent. Anything else —
  /// a first observation, a steady state, a stop the app asked for — is not.
  fn observe_tunnel_runtime(
    &self,
    db: &Database,
    tunnel_id: &str,
    node_id: &str,
    connected: bool,
  ) -> Option<UnexpectedDrop> {
    let mut watch = self.tunnel_watch.lock().ok()?;

    let was_connected = watch.last_connected.insert(tunnel_id.to_string(), connected);

    if connected {
      watch.expected_stop.remove(tunnel_id);
      if watch.drop.as_ref().is_some_and(|drop| drop.tunnel_id == tunnel_id) {
        watch.drop = None;
      }
      return None;
    }

    if was_connected == Some(true) {
      if watch.expected_stop.remove(tunnel_id) {
        // The app asked for this stop, so it is not a drop.
        return None;
      }

      // Written here, at the transition, rather than wherever the drop is later read: `watch.drop`
      // keeps reporting the same drop on every poll, so recording on read would append a row every
      // few seconds until the user reconnected.
      ActivityRecorder::new(db, "connect", None)
        .for_node(node_id)
        .record(
          "connect.dropped",
          "error",
          &format!(
            "tunnel_id={tunnel_id} stopped without being asked to; traffic is unprotected until reconnect"
          ),
        );

      watch.drop = Some(UnexpectedDrop {
        tunnel_id: tunnel_id.to_string(),
        node_id: node_id.to_string(),
        observed_at_epoch_ms: epoch_millis(),
      });
    }

    watch.drop.clone()
  }

  /// The addresses the internet currently sees, cached.
  ///
  /// Probed regardless of connection state: the card's whole purpose is showing the address
  /// change when the tunnel comes up, which needs a reading from both sides of the transition.
  /// Both families are probed together so one family's timeout does not delay the other.
  fn external_ips(&self, connected: bool, force: bool) -> (Option<String>, Option<String>) {
    if !force {
      if let Ok(cache) = self.external_ip.lock() {
        if let Some(snapshot) = cache.as_ref() {
          if snapshot.measured_while_connected == connected
            && snapshot.observed_at.elapsed() < EXTERNAL_IP_CACHE_TTL
          {
            return (snapshot.ipv4.clone(), snapshot.ipv6.clone());
          }
        }
      }
    }

    let v4_probe = Arc::clone(&self.ip_probe);
    let v6_probe = Arc::clone(&self.ip_probe);
    let v4_handle = std::thread::spawn(move || v4_probe.probe(IpFamily::V4));
    let ipv6 = v6_probe.probe(IpFamily::V6);
    let ipv4 = v4_handle.join().unwrap_or(None);

    if let Ok(mut cache) = self.external_ip.lock() {
      *cache = Some(ExternalIpSnapshot {
        ipv4: ipv4.clone(),
        ipv6: ipv6.clone(),
        observed_at: Instant::now(),
        measured_while_connected: connected,
      });
    }

    (ipv4, ipv6)
  }

  pub fn list_nodes(
    &self,
    providers: &ProviderRegistry,
    db: &Database,
    secure_storage: &SecureStorage,
  ) -> Result<Vec<crate::backend::db::VpnNodeRecord>, OrchestratorError> {
    let mut nodes = db.list_vpn_nodes().map_err(|source| OrchestratorError::Database {
      detail: "Failed listing vpn nodes.".to_string(),
      source,
    })?;

    self.attach_live_provider_status(providers, db, secure_storage, &mut nodes);
    self.attach_region_geography(providers, db, &mut nodes);
    Ok(nodes)
  }

  /// Name each node's location from its own adapter's table.
  ///
  /// No token and no network call, unlike the provider-status pass above: a node row stores only a
  /// region slug, and the fleet list is meant to stay readable when a provider key goes bad.
  fn attach_region_geography(
    &self,
    providers: &ProviderRegistry,
    db: &Database,
    nodes: &mut [crate::backend::db::VpnNodeRecord],
  ) {
    let mut resolved: HashMap<(String, String), Option<ProviderRegion>> = HashMap::new();

    for node in nodes {
      let key = (node.provider_id.clone(), node.region.clone());
      let region = resolved
        .entry(key)
        .or_insert_with(|| providers.describe_provider_region(db, &node.provider_id, &node.region));

      node.region_geo = region.clone();
    }
  }

  fn attach_live_provider_status(
    &self,
    providers: &ProviderRegistry,
    db: &Database,
    secure_storage: &SecureStorage,
    nodes: &mut [crate::backend::db::VpnNodeRecord],
  ) {
    for node in nodes {
      let external_ref = match node.external_ref.as_deref() {
        Some(value) => value,
        None => {
          node.provider_status = None;
          continue;
        }
      };

      let droplet_id = match external_ref.parse::<u64>() {
        Ok(value) => value,
        Err(_) => {
          node.provider_status = Some(ProviderDropletStatus::Unknown.as_str().to_string());
          continue;
        }
      };

      let live_status = match providers.provider_droplet_status(
        db,
        secure_storage,
        &node.provider_id,
        droplet_id,
      ) {
        Ok(status) => status,
        Err(_) => ProviderDropletStatus::Unknown,
      };

      node.provider_status = Some(live_status.as_str().to_string());
    }
  }

  /// The node the app connects to by default.
  ///
  /// Validated against the node list on every read: destroying a node leaves the stored id
  /// dangling, and returning it would point the connect button at a server that no longer exists.
  /// With nothing set, a lone node is unambiguously the current one; two or more is a real choice
  /// the user has not made yet, so the answer is `None` rather than a guess.
  pub fn get_current_node_id(&self, db: &Database) -> Result<Option<String>, OrchestratorError> {
    let nodes = db.list_vpn_nodes().map_err(|source| OrchestratorError::Database {
      detail: "Failed listing vpn nodes while resolving the current node.".to_string(),
      source,
    })?;

    let stored = db
      .get_setting(CURRENT_NODE_SETTING_KEY)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed reading the current node setting.".to_string(),
        source,
      })?;

    if let Some(node_id) = stored {
      if nodes.iter().any(|node| node.id == node_id) {
        return Ok(Some(node_id));
      }

      // Stale pointer to a destroyed node. Clear it so the fallback below can apply.
      let _ = db.set_setting(CURRENT_NODE_SETTING_KEY, None);
    }

    if nodes.len() == 1 {
      return Ok(nodes.into_iter().next().map(|node| node.id));
    }

    Ok(None)
  }

  /// Sets, or with `None` clears, the default node. Setting rejects an unknown id rather than
  /// storing a pointer to nothing.
  pub fn set_current_node_id(
    &self,
    db: &Database,
    node_id: Option<&str>,
  ) -> Result<(), OrchestratorError> {
    if let Some(node_id) = node_id {
      let exists = db
        .get_vpn_node(node_id)
        .map_err(|source| OrchestratorError::Database {
          detail: "Failed loading vpn node while setting the current node.".to_string(),
          source,
        })?
        .is_some();

      if !exists {
        return Err(OrchestratorError::InvalidState {
          node_id: node_id.to_string(),
          detail: "Cannot make an unknown node current. Refresh the node list and try again."
            .to_string(),
        });
      }
    }

    db.set_setting(CURRENT_NODE_SETTING_KEY, node_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed persisting the current node setting.".to_string(),
        source,
      })
  }

  /// The tunnel parameters the user has chosen, falling back to the shipped defaults.
  ///
  /// A stored value that no longer parses is treated as unset rather than propagated: it would
  /// otherwise render an invalid `.conf` and fail every connect with no way back through the UI.
  pub fn get_tunnel_settings(&self, db: &Database) -> Result<TunnelSettings, OrchestratorError> {
    let defaults = TunnelSettings::default();

    let dns_server = match db.get_setting(DNS_SERVER_SETTING_KEY) {
      // An explicitly stored empty string means "no DNS line", which is distinct from unset.
      Ok(Some(value)) if value.trim().is_empty() => None,
      Ok(Some(value)) if value.trim().parse::<IpAddr>().is_ok() => Some(value.trim().to_string()),
      Ok(Some(_)) | Err(_) => defaults.dns_server.clone(),
      Ok(None) => defaults.dns_server.clone(),
    };

    let keepalive_seconds = match db.get_setting(KEEPALIVE_SETTING_KEY) {
      Ok(Some(value)) => value.trim().parse::<u16>().unwrap_or(defaults.keepalive_seconds),
      _ => defaults.keepalive_seconds,
    };

    Ok(TunnelSettings { dns_server, keepalive_seconds })
  }

  /// Validates before storing, so an unusable value can never reach config generation.
  pub fn set_tunnel_settings(
    &self,
    db: &Database,
    settings: &TunnelSettings,
  ) -> Result<(), OrchestratorError> {
    let dns_value = match settings.dns_server.as_ref().map(|value| value.trim().to_string()) {
      Some(value) if value.is_empty() => String::new(),
      Some(value) => {
        if value.parse::<IpAddr>().is_err() {
          return Err(OrchestratorError::InvalidInput {
            detail: format!("'{value}' is not a valid IP address for a DNS server."),
          });
        }
        value
      }
      None => String::new(),
    };

    db.set_setting(DNS_SERVER_SETTING_KEY, Some(&dns_value))
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed persisting the DNS server setting.".to_string(),
        source,
      })?;

    db.set_setting(KEEPALIVE_SETTING_KEY, Some(&settings.keepalive_seconds.to_string()))
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed persisting the keepalive setting.".to_string(),
        source,
      })
  }

  pub fn list_tunnels_for_node(&self, db: &Database, node_id: &str) -> Result<Vec<crate::backend::db::TunnelRecord>, OrchestratorError> {
    db.list_tunnels_for_node(node_id).map_err(|source| OrchestratorError::Database {
      detail: "Failed listing tunnels for node.".to_string(),
      source,
    })
  }

  pub fn provision_droplet_for_provider(
    &self,
    providers: &ProviderRegistry,
    bootstrap: &Bootstrap,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    droplet_name: &str,
  ) -> Result<ProvisionedDroplet, OrchestratorError> {
    let server_keypair = generate_server_keypair().map_err(|source| OrchestratorError::Keygen {
      detail: "Failed to generate WireGuard server keypair for droplet provisioning.".to_string(),
      source,
    })?;

    // This lower-level helper provisions a bare droplet without an associated node/tunnel record,
    // so there is nowhere to persist a client private key. Generate a throwaway peer to keep the
    // payload well-formed; real connectable nodes come from `create_vpn_node_to_ready`.
    let client_keypair = generate_client_keypair().map_err(|source| OrchestratorError::Keygen {
      detail: "Failed to generate WireGuard client keypair for droplet provisioning.".to_string(),
      source,
    })?;

    let artifact = bootstrap
      .render_cloud_init(&BootstrapParams {
        server_endpoint_ip: "0.0.0.0".to_string(),
        listen_port: DEFAULT_WIREGUARD_PORT,
        server_interface_cidr: DEFAULT_SERVER_INTERFACE_CIDR.to_string(),
        client_cidr: DEFAULT_CLIENT_CIDR.to_string(),
        interface_name: DEFAULT_INTERFACE_NAME.to_string(),
        client_public_key: client_keypair.public_key.clone(),
      }, &server_keypair.private_key, &server_keypair.public_key)
      .map_err(|source| OrchestratorError::Bootstrap {
        detail: "Could not render bootstrap cloud-init for droplet create.".to_string(),
        source,
      })?;

    let token = providers
      .resolve_provider_token_for_orchestrator(db, secure_storage, provider_id)
      .map_err(|source| OrchestratorError::Provider {
        detail: "Failed to resolve provider token before node initialization.".to_string(),
        source,
      })?;

    let node_provider = providers
      .node_provider_for_provider_id(db, secure_storage, provider_id)
      .map_err(|source| OrchestratorError::Provider {
        detail: "No node provider implementation is currently available for init flow.".to_string(),
        source,
      })?;

    let request = NodeInitRequest {
      node_name: droplet_name.to_string(),
      region: DEFAULT_REGION.to_string(),
      size_slug: DEFAULT_SIZE.to_string(),
      init_payload: artifact.init_payload,
    };

    let init_result = node_provider
      .init(&token, &request)
      .map_err(|source| OrchestratorError::Provider {
        detail: "Provisioning flow failed while creating or activating droplet.".to_string(),
        source: ProviderFlowError::Adapter {
          detail: "Provider adapter init call failed.".to_string(),
          source,
        },
      })?;

    Ok(init_result.provisioned)
  }

  pub fn destroy_droplet_for_provider(
    &self,
    providers: &ProviderRegistry,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    droplet_id: u64,
  ) -> Result<(), OrchestratorError> {
    providers
      .delete_provider_droplet(db, secure_storage, provider_id, droplet_id)
      .map_err(|source| OrchestratorError::Provider {
        detail: "Destroy flow failed while deleting droplet.".to_string(),
        source,
      })
  }

  pub fn destroy_vpn_node(
    &self,
    providers: &ProviderRegistry,
    wireguard: &WireGuardManager,
    db: &Database,
    secure_storage: &SecureStorage,
    node_id: &str,
  ) -> Result<(), OrchestratorError> {
    let _ = self.cleanup_vpn_node(providers, wireguard, db, secure_storage, node_id, "destroy");
    Ok(())
  }

  pub fn force_cleanup_vpn_node(
    &self,
    providers: &ProviderRegistry,
    wireguard: &WireGuardManager,
    db: &Database,
    secure_storage: &SecureStorage,
    node_id: &str,
  ) -> Result<ForceCleanupResult, OrchestratorError> {
    self.cleanup_vpn_node(providers, wireguard, db, secure_storage, node_id, "cleanup")
  }

  /// Shared teardown for both Destroy and Force Cleanup.
  ///
  /// `category` only labels the activity trail — the two entry points do the same work, and the
  /// user needs to be able to tell which one they asked for when reading the history back.
  fn cleanup_vpn_node(
    &self,
    providers: &ProviderRegistry,
    wireguard: &WireGuardManager,
    db: &Database,
    secure_storage: &SecureStorage,
    node_id: &str,
    category: &str,
  ) -> Result<ForceCleanupResult, OrchestratorError> {
    let correlation_id = Uuid::new_v4().to_string();
    let activity = ActivityRecorder::new(db, category, Some(&correlation_id)).for_node(node_id);
    activity.info("cleanup.start", "teardown requested");
    log::warn!(
      "event=vpn_node.force_cleanup.start correlation_id={} node_id={}",
      correlation_id,
      node_id
    );

    let node = db.get_vpn_node(node_id).map_err(|source| OrchestratorError::Database {
      detail: "Failed loading vpn node before force cleanup flow.".to_string(),
      source,
    })
    .inspect_err(|error| activity.failure("cleanup.error", error))?;

    let tunnels = db.list_tunnels_for_node(node_id).map_err(|source| OrchestratorError::Database {
      detail: "Failed listing tunnels before force cleanup flow.".to_string(),
      source,
    })
    .inspect_err(|error| activity.failure("cleanup.error", error))?;

    for tunnel in &tunnels {
      let _ = wireguard.unregister_tunnel(&tunnel.name);
    }

    let local_tunnels_removed = tunnels.len();
    for tunnel in tunnels {
      if let Some(private_key_ref) = tunnel.client_private_key_ref.as_deref() {
        let _ = secure_storage.delete_client_private_key(private_key_ref);
      }
      let _ = db.delete_tunnel(&tunnel.id);
    }

    let mut remote_droplet_delete_attempted = false;
    let mut remote_droplet_delete_succeeded = false;
    if let Some(node) = node.as_ref() {
      if let Some(external_ref) = node.external_ref.as_deref() {
        if let Ok(droplet_id) = external_ref.parse::<u64>() {
          remote_droplet_delete_attempted = true;
          remote_droplet_delete_succeeded = providers
            .delete_provider_droplet(db, secure_storage, &node.provider_id, droplet_id)
            .is_ok();
        }
      }
    }

    let node_record_removed = node
      .as_ref()
      .map(|record| db.delete_vpn_node(&record.id).is_ok())
      .unwrap_or(true);

    // A destroyed node cannot stay the default. Leaving the pointer would aim the connect button
    // at a server that no longer exists.
    if node_record_removed {
      if let Ok(Some(current)) = db.get_setting(CURRENT_NODE_SETTING_KEY) {
        if current == node_id {
          let _ = db.set_setting(CURRENT_NODE_SETTING_KEY, None);
        }
      }
    }

    log::warn!(
      "event=vpn_node.force_cleanup.completed correlation_id={} node_id={} local_tunnels_removed={} remote_droplet_delete_attempted={} remote_droplet_delete_succeeded={} node_record_removed={}",
      correlation_id,
      node_id,
      local_tunnels_removed,
      remote_droplet_delete_attempted,
      remote_droplet_delete_succeeded,
      node_record_removed
    );

    let detail = format!(
      "local_tunnels_removed={local_tunnels_removed} remote_droplet_delete_attempted={remote_droplet_delete_attempted} remote_droplet_delete_succeeded={remote_droplet_delete_succeeded} node_record_removed={node_record_removed}"
    );
    // A remote delete that was tried and failed leaves a droplet the user is still paying for while
    // the local row disappears. That is the one outcome of this flow that must not read as success.
    if remote_droplet_delete_attempted && !remote_droplet_delete_succeeded {
      activity.record(
        "cleanup.completed",
        "error",
        &format!("{detail} — the remote server was NOT deleted and may still be billing"),
      );
    } else {
      activity.success("cleanup.completed", &detail);
    }

    Ok(ForceCleanupResult {
      node_id: node_id.to_string(),
      correlation_id,
      local_tunnels_removed,
      remote_droplet_delete_attempted,
      remote_droplet_delete_succeeded,
      node_record_removed,
    })
  }

  pub fn create_vpn_node_to_ready(
    &self,
    providers: &ProviderRegistry,
    bootstrap: &Bootstrap,
    db: &Database,
    secure_storage: &SecureStorage,
    request: &CreateVpnNodeRequest,
  ) -> Result<CreateVpnNodeResult, OrchestratorError> {
    let correlation_id = Uuid::new_v4().to_string();
    log::info!(
      "event=vpn_node.create.start correlation_id={} node_id={} provider_id={}",
      correlation_id,
      request.node_id,
      request.provider_id
    );

    let provider_slug = db
      .get_provider(&request.provider_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed loading provider metadata before create flow.".to_string(),
        source,
      })?
      .map(|provider| provider.slug)
      .unwrap_or_else(|| "digitalocean".to_string());

    // The stage labels used to be prefixed "cloud-init" or "docker-init", chosen by comparing the
    // slug here — a provider branch outside an adapter (AGENTS.md 3). `provider_slug` is recorded
    // on every event and already carries that distinction.
    let activity = ActivityRecorder::new(db, "provision", Some(&correlation_id))
      .for_node(&request.node_id)
      .with_provider(Some(&provider_slug));
    let record_init = |stage: &str, outcome: &str, output: String| {
      activity.record(stage, outcome, &output);
    };
    record_init(
      "create.start",
      "info",
      format!(
        "init flow started; provider_id={} correlation_id={}",
        request.provider_id, correlation_id
      ),
    );

    providers
      .resolve_provider_token_for_orchestrator(db, secure_storage, &request.provider_id)
      .map_err(|source| OrchestratorError::Provider {
        detail: "Provider token reference is missing or unreadable for node create flow."
          .to_string(),
        source,
      })?;

    providers
      .validate_provider_token(db, secure_storage, &request.provider_id)
      .map_err(|source| OrchestratorError::Provider {
        detail: if provider_slug == "docker" {
          "Docker provider validation failed before provisioning node. Verify Docker Desktop is running and daemon is reachable, then retry.".to_string()
        } else {
          "Provider token failed validation before provisioning node.".to_string()
        },
        source,
      })?;

    if let Some(existing) = db
      .get_vpn_node(&request.node_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed checking for existing vpn node before create.".to_string(),
        source,
      })?
    {
      match existing.lifecycle_status {
        NodeLifecycleStatus::Ready => {
          if let (Some(external_ref), Some(public_ip), Some(server_public_key)) = (
            existing.external_ref.clone(),
            existing.public_ipv4.clone(),
            existing.server_public_key.clone(),
          ) {
            let droplet_id = external_ref.parse::<u64>().unwrap_or(0);
            return Ok(CreateVpnNodeResult {
              node_id: existing.id,
              status: NodeLifecycleStatus::Ready,
              droplet_id,
              server_endpoint_ip: public_ip,
              server_public_key,
              listen_port: DEFAULT_WIREGUARD_PORT,
              correlation_id,
              idempotent_reuse: true,
            });
          }

          return Err(OrchestratorError::InvalidState {
            node_id: request.node_id.clone(),
            detail: "Node is marked Ready but missing droplet reference, endpoint IP, or server key."
              .to_string(),
          });
        }
        NodeLifecycleStatus::Provisioning | NodeLifecycleStatus::Bootstrapping => {
          return Err(OrchestratorError::InvalidState {
            node_id: request.node_id.clone(),
            detail:
              "Node create flow is already in progress; wait for completion or resolve error state."
                .to_string(),
          });
        }
        NodeLifecycleStatus::Connected => {
          return Err(OrchestratorError::InvalidState {
            node_id: request.node_id.clone(),
            detail: "Node is already connected; create flow is not applicable.".to_string(),
          });
        }
        NodeLifecycleStatus::Error | NodeLifecycleStatus::Deleting => {
          return Err(OrchestratorError::InvalidState {
            node_id: request.node_id.clone(),
            detail:
              "Node is in Error/Deleting state; cleanup or reconciliation is required before retry."
                .to_string(),
          });
        }
        NodeLifecycleStatus::New => {}
      }
    } else {
      db
        .create_vpn_node(
          &request.node_id,
          &request.provider_id,
          &request.node_name,
          &request.region,
        )
        .map_err(|source| OrchestratorError::Database {
          detail: "Failed inserting new vpn node record.".to_string(),
          source,
        })?;
    }

    db
      .transition_vpn_node_status(&request.node_id, NodeLifecycleStatus::Provisioning, None)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed transitioning node to Provisioning state.".to_string(),
        source,
      })?;

    let server_keypair = generate_server_keypair().map_err(|source| OrchestratorError::Keygen {
      detail: "Failed to generate WireGuard server keypair for node provisioning.".to_string(),
      source,
    })?;

    // The client keypair must exist before the payload is rendered: its public key ships inside
    // the server's [Peer] block. Because the app never contacts the server after create, there is
    // no later opportunity to register a peer.
    let client_keypair = generate_client_keypair().map_err(|source| OrchestratorError::Keygen {
      detail: "Failed to generate WireGuard client keypair for node provisioning.".to_string(),
      source,
    })?;

    let client_private_key_ref = secure_storage
      .store_client_private_key(&request.node_id, &client_keypair.private_key)
      .map_err(|source| OrchestratorError::SecretStorage {
        detail: "Could not persist client private key before provisioning.".to_string(),
        source,
      })?;

    db
      .update_vpn_node_client_key(
        &request.node_id,
        Some(&client_private_key_ref),
        Some(&client_keypair.public_key),
      )
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed recording client key reference on node.".to_string(),
        source,
      })?;

    let bootstrap_params = BootstrapParams {
      server_endpoint_ip: "0.0.0.0".to_string(),
      listen_port: DEFAULT_WIREGUARD_PORT,
      server_interface_cidr: DEFAULT_SERVER_INTERFACE_CIDR.to_string(),
      client_cidr: DEFAULT_CLIENT_CIDR.to_string(),
      interface_name: DEFAULT_INTERFACE_NAME.to_string(),
      client_public_key: client_keypair.public_key.clone(),
    };

    // Both renderers receive the same key material and produce the same server state; only the
    // delivery mechanism differs (cloud-init write_files vs. a mounted entrypoint script).
    let bootstrap_artifact = if provider_slug == "docker" {
      bootstrap.render_docker_init(
        &bootstrap_params,
        &server_keypair.private_key,
        &server_keypair.public_key,
      )
    } else {
      bootstrap.render_cloud_init(
        &bootstrap_params,
        &server_keypair.private_key,
        &server_keypair.public_key,
      )
    }
    .map_err(|source| OrchestratorError::Bootstrap {
      detail: if provider_slug == "docker" {
        "Failed rendering Docker init script for node bootstrap.".to_string()
      } else {
        "Failed rendering cloud-init user-data for node bootstrap.".to_string()
      },
      source,
    })?;

    record_init(
      "bootstrap.render",
      "success",
      format!(
        "readiness marker path={} output contract preview endpoint={} listen_port={}",
        bootstrap_artifact.readiness_marker_path,
        bootstrap_artifact.output_contract_preview.server_endpoint_ip,
        bootstrap_artifact.output_contract_preview.listen_port
      ),
    );

    let mut created_droplet: Option<ProvisionedDroplet> = None;

    let pricing = providers
      .list_provider_plan_pricing(db, secure_storage, &request.provider_id, &request.region)
      .map_err(|source| OrchestratorError::Provider {
        detail: "Could not retrieve live provider pricing before provisioning.".to_string(),
        source,
      })?;

    let selected_pricing = select_plan_pricing(&pricing, &request.plan_key).ok_or_else(|| {
      OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: format!(
          "Selected plan '{}' is unavailable for region '{}' in current provider pricing.",
          request.plan_key, request.region
        ),
      }
    })?;

    db
      .update_vpn_node_pricing_snapshot(
        &request.node_id,
        Some(&selected_pricing.plan_key),
        Some(&selected_pricing.size_slug),
        Some(selected_pricing.monthly_usd),
        Some(selected_pricing.hourly_usd),
        Some(&selected_pricing.source_timestamp),
      )
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed persisting selected pricing snapshot for node.".to_string(),
        source,
      })?;

    let flow_result: Result<CreateVpnNodeResult, OrchestratorError> = (|| {
      let init_request = NodeInitRequest {
        node_name: request.node_name.clone(),
        region: request.region.clone(),
        size_slug: selected_pricing.size_slug.clone(),
        init_payload: bootstrap_artifact.init_payload.clone(),
      };

      let init_result = providers
        .init_provider_node(db, secure_storage, &request.provider_id, &init_request)
        .map_err(|source| OrchestratorError::Provider {
          detail: "Node creation or bootstrap setup failed in create-node flow.".to_string(),
          source,
        })?;

      let droplet = init_result.provisioned;

      record_init(
        "provider.create",
        "success",
        format!(
          "droplet created with external_ref={} endpoint_ip={}",
          droplet.droplet_id, droplet.public_ipv4
        ),
      );

      created_droplet = Some(droplet.clone());

      db
        .update_vpn_node_runtime_details(
          &request.node_id,
          Some(&droplet.droplet_id.to_string()),
          Some(&droplet.public_ipv4),
          None,
          Some(droplet.endpoint_port),
        )
        .map_err(|source| OrchestratorError::Database {
          detail: "Failed storing droplet reference/IP for node record.".to_string(),
          source,
        })?;

      db
        .transition_vpn_node_status(&request.node_id, NodeLifecycleStatus::Bootstrapping, None)
        .map_err(|source| OrchestratorError::Database {
          detail: "Failed transitioning node to Bootstrapping state.".to_string(),
          source,
        })?;

      let contract = providers
        .wait_for_provider_bootstrap_ready(
          db,
          secure_storage,
          &request.provider_id,
          &BootstrapWaitRequest {
            droplet_id: droplet.droplet_id,
            endpoint_ip: droplet.public_ipv4.clone(),
            server_keypair,
          },
        )
        .map_err(|source| OrchestratorError::Provider {
          detail: "Bootstrap readiness marker wait failed.".to_string(),
          source,
        })?;

      record_init(
        "bootstrap.wait",
        "success",
        format!(
          "ready contract endpoint={} listen_port={}",
          contract.server_endpoint_ip, contract.listen_port
        ),
      );

      db
        .update_vpn_node_runtime_details(
          &request.node_id,
          Some(&droplet.droplet_id.to_string()),
          Some(&contract.server_endpoint_ip),
          Some(&contract.server_public_key),
          // The adapter reports the port the client should dial, which for the Docker dev
          // provider is the published host port rather than the in-container listen port.
          Some(contract.listen_port),
        )
        .map_err(|source| OrchestratorError::Database {
          detail: "Failed persisting server endpoint and public key.".to_string(),
          source,
        })?;

      db
        .transition_vpn_node_status(&request.node_id, NodeLifecycleStatus::Ready, None)
        .map_err(|source| OrchestratorError::Database {
          detail: "Failed transitioning node to Ready state.".to_string(),
          source,
        })?;

      log::info!(
        "event=vpn_node.create.ready correlation_id={} node_id={} droplet_id={} endpoint_ip={}",
        correlation_id,
        request.node_id,
        droplet.droplet_id,
        contract.server_endpoint_ip
      );

      record_init(
        "create.ready",
        "success",
        format!(
          "node ready with droplet_id={} endpoint_ip={}",
          droplet.droplet_id, contract.server_endpoint_ip
        ),
      );

      Ok(CreateVpnNodeResult {
        node_id: request.node_id.clone(),
        status: NodeLifecycleStatus::Ready,
        droplet_id: droplet.droplet_id,
        server_endpoint_ip: contract.server_endpoint_ip,
        server_public_key: contract.server_public_key,
        listen_port: contract.listen_port,
        correlation_id: correlation_id.clone(),
        idempotent_reuse: false,
      })
    })();

    if let Err(error) = flow_result {
      // The whole chain, so the provider's actual complaint survives into `error_reason` and the
      // log instead of being flattened to the orchestrator's own wrapper message.
      let mut final_reason = format!(
        "create_vpn_node_to_ready failed: {}",
        logging::describe_error_chain(&error)
      );

      if let Some(droplet) = created_droplet {
        let cleanup_result = providers.delete_provider_droplet(
          db,
          secure_storage,
          &request.provider_id,
          droplet.droplet_id,
        );

        if let Err(cleanup_error) = cleanup_result {
          final_reason = format!(
            "{} | rollback cleanup failed for droplet {}: {}",
            final_reason,
            droplet.droplet_id,
            cleanup_error
          );
          record_init(
            "rollback.cleanup",
            "error",
            format!(
              "droplet cleanup failed for {}: {}",
              droplet.droplet_id, cleanup_error
            ),
          );
        } else {
          record_init(
            "rollback.cleanup",
            "success",
            format!("droplet cleanup completed for {}", droplet.droplet_id),
          );
        }
      }

      record_init("create.error", "error", final_reason.clone());

      let _ = db.transition_vpn_node_status(
        &request.node_id,
        NodeLifecycleStatus::Error,
        Some(&final_reason),
      );

      log::error!(
        "event=vpn_node.create.error correlation_id={} node_id={} reason={}",
        correlation_id,
        request.node_id,
        logging::redact_sensitive_text(&final_reason)
      );

      return Err(error);
    }

    flow_result
  }

  pub fn create_and_activate_client_tunnel(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
    secure_storage: &SecureStorage,
    request: &CreateClientTunnelRequest,
  ) -> Result<CreateClientTunnelResult, OrchestratorError> {
    let node = db
      .get_vpn_node(&request.node_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed reading vpn node before client tunnel creation.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: "VPN node does not exist for tunnel creation.".to_string(),
      })?;

    if !matches!(
      node.lifecycle_status,
      NodeLifecycleStatus::Ready | NodeLifecycleStatus::Connected
    ) {
      return Err(OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: "VPN node must be Ready before client tunnel activation.".to_string(),
      });
    }

    let canonical_tunnel_name = canonical_tunnel_name_for_node(&node.name);

    let existing = db
      .get_tunnel(&request.tunnel_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed reading tunnel before create.".to_string(),
        source,
      })?;

    if existing.is_none() {
      db
        .create_tunnel(
          &request.tunnel_id,
          &request.node_id,
          &canonical_tunnel_name,
          Some(&canonical_tunnel_name),
        )
        .map_err(|source| OrchestratorError::Database {
          detail: "Failed creating tunnel row before activation.".to_string(),
          source,
        })?;
    } else {
      let existing = existing.expect("checked is_some above");
      if existing.node_id != request.node_id {
        return Err(OrchestratorError::InvalidState {
          node_id: request.node_id.clone(),
          detail: "Tunnel id already belongs to a different vpn node.".to_string(),
        });
      }

      if existing.name != canonical_tunnel_name
        || existing.interface_name.as_deref() != Some(canonical_tunnel_name.as_str())
      {
        db
          .update_tunnel_identity(
            &request.tunnel_id,
            &canonical_tunnel_name,
            Some(&canonical_tunnel_name),
          )
          .map_err(|source| OrchestratorError::Database {
            detail: "Failed normalizing tunnel name for existing tunnel.".to_string(),
            source,
          })?;
      }
    }

    let persisted_tunnel = db
      .get_tunnel(&request.tunnel_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed reloading tunnel before secure key setup.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: "Tunnel row is missing during secure key setup.".to_string(),
      })?;

    // The server was provisioned with one specific client public key baked into its [Peer] block,
    // and the app cannot reach the server to change it. Generating a fresh keypair here would
    // therefore produce a tunnel that can never complete a handshake, so this resolves the
    // existing key or fails — it must never mint a new one.
    let client_private_key_ref = persisted_tunnel
      .client_private_key_ref
      .clone()
      .or_else(|| node.client_private_key_ref.clone())
      .ok_or_else(|| OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail:
          "This node has no client key recorded from provisioning, so its server has no matching peer. Destroy and recreate the node."
            .to_string(),
      })?;

    let private_key = secure_storage
      .get_client_private_key(&client_private_key_ref)
      .map_err(|source| OrchestratorError::SecretStorage {
        detail: "Could not load persisted client private key from secure storage.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail:
          "The client private key for this node is missing from Windows Credential Manager. Its server peer can no longer be matched; destroy and recreate the node."
            .to_string(),
      })?;

    let public_key = wireguard
      .derive_public_key(&private_key)
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not derive WireGuard public key from persisted private key.".to_string(),
        source,
      })?;

    // Guard against a key that does not match what the server was built with. Without this the
    // failure surfaces as a silent no-handshake, which is very hard to diagnose.
    if let Some(provisioned) = node.client_public_key.as_deref() {
      if provisioned != public_key {
        return Err(OrchestratorError::InvalidState {
          node_id: request.node_id.clone(),
          detail:
            "The stored client key does not match the peer baked into this node's server. Destroy and recreate the node."
              .to_string(),
        });
      }
    }

    db
      .update_tunnel_client_config(
        &request.tunnel_id,
        Some(&client_private_key_ref),
        Some(&request.client_address_cidr),
        request.dns_server.as_deref(),
        Some(&request.allowed_ips.join(",")),
      )
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed persisting tunnel secure metadata.".to_string(),
        source,
      })?;

    // No peer wiring step. The server already has this client in its [Peer] block from the
    // bootstrap payload, for every provider — there is nothing provider-specific to do here.

    db
      .update_tunnel_status(&request.tunnel_id, TunnelStatus::Ready, None)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed setting tunnel status to configured.".to_string(),
        source,
      })?;

    Ok(CreateClientTunnelResult {
      tunnel_id: request.tunnel_id.clone(),
      tunnel_name: canonical_tunnel_name,
      client_public_key: public_key,
    })
  }

  pub fn deactivate_client_tunnel(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
    secure_storage: &SecureStorage,
    tunnel_id: &str,
  ) -> Result<(), OrchestratorError> {
    let tunnel = db
      .get_tunnel(tunnel_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed loading tunnel before deactivation.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::Database {
        detail: "Tunnel deactivation requested for unknown id.".to_string(),
        source: DbError::NotFound {
          entity: "tunnel",
          id: tunnel_id.to_string(),
          detail: "No tunnel row matched the id for deactivation.".to_string(),
        },
      })?;

    self.disconnect_tunnel(
      wireguard,
      db,
      secure_storage,
      &DisconnectTunnelRequest {
        node_id: tunnel.node_id,
        tunnel_id: tunnel_id.to_string(),
      },
    )
  }

  pub fn activate_client_tunnel(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
    secure_storage: &SecureStorage,
    tunnel_id: &str,
  ) -> Result<(), OrchestratorError> {
    let tunnel = db
      .get_tunnel(tunnel_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed loading tunnel before activation.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::Database {
        detail: "Tunnel activation requested for unknown id.".to_string(),
        source: DbError::NotFound {
          entity: "tunnel",
          id: tunnel_id.to_string(),
          detail: "No tunnel row matched the id for activation.".to_string(),
        },
      })?;

    self.connect_tunnel(
      wireguard,
      db,
      secure_storage,
      &ConnectTunnelRequest {
        node_id: tunnel.node_id,
        tunnel_id: tunnel_id.to_string(),
      },
    )
  }

  pub fn connect_tunnel(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
    secure_storage: &SecureStorage,
    request: &ConnectTunnelRequest,
  ) -> Result<(), OrchestratorError> {
    // Recorded here rather than inside `connect_tunnel_inner`, which returns early from a dozen
    // places; wrapping it means every one of those exits is accounted for.
    let correlation_id = Uuid::new_v4().to_string();
    let activity = ActivityRecorder::new(db, "connect", Some(&correlation_id))
      .for_node(&request.node_id);
    activity.info(
      "connect.start",
      &format!("tunnel_id={}", request.tunnel_id),
    );

    let outcome = self.connect_tunnel_inner(wireguard, db, secure_storage, request);

    match &outcome {
      Ok(()) => activity.success(
        "connect.ready",
        &format!("tunnel_id={} is connected", request.tunnel_id),
      ),
      Err(error) => activity.failure("connect.error", error),
    }

    outcome
  }

  fn connect_tunnel_inner(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
    secure_storage: &SecureStorage,
    request: &ConnectTunnelRequest,
  ) -> Result<(), OrchestratorError> {
    let node = db
      .get_vpn_node(&request.node_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed loading vpn node before connect flow.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: "Connect requested for a vpn node that does not exist.".to_string(),
      })?;

    if !matches!(
      node.lifecycle_status,
      NodeLifecycleStatus::Ready | NodeLifecycleStatus::Connected
    ) {
      return Err(OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: "Connect flow requires node lifecycle status Ready or Connected.".to_string(),
      });
    }

    let tunnel = db
      .get_tunnel(&request.tunnel_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed loading tunnel before connect flow.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::Database {
        detail: "Connect flow requested for unknown tunnel id.".to_string(),
        source: DbError::NotFound {
          entity: "tunnel",
          id: request.tunnel_id.clone(),
          detail: "No tunnel row matched the id for connect flow.".to_string(),
        },
      })?;

    if tunnel.node_id != request.node_id {
      return Err(OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: "Connect flow tunnel id does not belong to the requested node.".to_string(),
      });
    }

    let canonical_tunnel_name = canonical_tunnel_name_for_node(&node.name);
    let previous_tunnel_name = tunnel.name.clone();
    if previous_tunnel_name != canonical_tunnel_name
      || tunnel.interface_name.as_deref() != Some(canonical_tunnel_name.as_str())
    {
      // Best-effort migration cleanup for legacy/randomized tunnel services.
      // Ignore cleanup errors so connect can still proceed.
      if previous_tunnel_name != canonical_tunnel_name {
        let _ = wireguard.unregister_tunnel(&previous_tunnel_name);
      }

      db
        .update_tunnel_identity(
          &request.tunnel_id,
          &canonical_tunnel_name,
          Some(&canonical_tunnel_name),
        )
        .map_err(|source| OrchestratorError::Database {
          detail: "Failed normalizing tunnel identity before connect flow.".to_string(),
          source,
        })?;
    }

    let provider = db
      .get_provider(&node.provider_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed loading provider before connect flow.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: "Connect flow requires an existing provider for the VPN node.".to_string(),
      })?;

    let tunnel_settings = self.get_tunnel_settings(db)?;
    let connect_config =
      load_connect_config_for_tunnel(secure_storage, &provider.slug, &node, &tunnel, &tunnel_settings)?;

    // Always capture the latest connect config in diagnostics, even when connect is idempotent.
    wireguard.record_connect_trace_snapshot(&canonical_tunnel_name, &connect_config);

    if tunnel.status == TunnelStatus::Connected && node.lifecycle_status == NodeLifecycleStatus::Connected {
      if let Ok(runtime) = wireguard.read_tunnel_status(&canonical_tunnel_name) {
        if runtime.state == crate::backend::wireguard::TunnelServiceState::Running {
          return Ok(());
        }
      }
    }

    let mut imported_in_this_connect = false;
    match wireguard.read_tunnel_status(&canonical_tunnel_name) {
      Ok(runtime) => {
        if runtime.state == crate::backend::wireguard::TunnelServiceState::Running {
          persist_tunnel_stats_snapshot(db, &request.tunnel_id, &runtime)?;
          let _ = db.update_tunnel_status(&request.tunnel_id, TunnelStatus::Connected, None);
          let _ = db.transition_vpn_node_status(&request.node_id, NodeLifecycleStatus::Connected, None);
          return Ok(());
        }
      }
      Err(WireGuardError::TunnelNotFound { .. }) => {
        wireguard
          .import_tunnel(&canonical_tunnel_name, &connect_config)
          .map_err(|source| OrchestratorError::WireGuard {
            detail: "Could not import client tunnel into WireGuard for Windows during connect flow."
              .to_string(),
            source,
          })?;

        match wireguard.read_tunnel_status(&canonical_tunnel_name) {
          Ok(_) => {}
          Err(source @ WireGuardError::TunnelNotFound { .. }) => {
            return Err(OrchestratorError::WireGuard {
              detail: "Tunnel import completed but no local WireGuard tunnel service was created."
                .to_string(),
              source,
            });
          }
          Err(source) => {
            return Err(OrchestratorError::WireGuard {
              detail: "Could not verify WireGuard tunnel service after import in connect flow."
                .to_string(),
              source,
            });
          }
        }

        imported_in_this_connect = true;
      }
      Err(source) => {
        return Err(OrchestratorError::WireGuard {
          detail: "Could not read WireGuard tunnel status before connect activation.".to_string(),
          source,
        });
      }
    }

    if let Err(source) = wireguard.activate_tunnel(&canonical_tunnel_name) {
      if matches!(source, WireGuardError::TunnelNotFound { .. }) && !imported_in_this_connect {
        wireguard
          .import_tunnel(&canonical_tunnel_name, &connect_config)
          .map_err(|import_source| OrchestratorError::WireGuard {
            detail: "Could not import client tunnel after missing-service activation response."
              .to_string(),
            source: import_source,
          })?;

        wireguard
          .activate_tunnel(&canonical_tunnel_name)
          .map_err(|activate_source| OrchestratorError::WireGuard {
            detail: "Could not activate WireGuard tunnel service after re-import.".to_string(),
            source: activate_source,
          })?;
      } else {
        if imported_in_this_connect {
          let _ = wireguard.unregister_tunnel(&canonical_tunnel_name);
          let _ = db.update_tunnel_status(&request.tunnel_id, TunnelStatus::Ready, Some("Tunnel activation failed; service was removed."));
        }

        return Err(OrchestratorError::WireGuard {
          detail: "Could not activate WireGuard tunnel service in connect flow.".to_string(),
          source,
        });
      }
    }

    let runtime = wireguard
      .read_tunnel_status(&canonical_tunnel_name)
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not verify tunnel runtime state after connect.".to_string(),
        source,
      })?;

    persist_tunnel_stats_snapshot(db, &request.tunnel_id, &runtime)?;

    let status = match runtime.state {
      crate::backend::wireguard::TunnelServiceState::Running => TunnelStatus::Connected,
      crate::backend::wireguard::TunnelServiceState::Stopped => TunnelStatus::Disconnected,
      crate::backend::wireguard::TunnelServiceState::Unknown => TunnelStatus::Connected,
    };

    db
      .update_tunnel_status(&request.tunnel_id, status, None)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed persisting tunnel status after connect flow.".to_string(),
        source,
      })?;

    if status == TunnelStatus::Connected {
      // Stamps the session start so the dashboard can show a real elapsed timer. Best-effort:
      // an uptime display is not worth failing an otherwise successful connect over.
      let _ = db.set_tunnel_connected_at(&request.tunnel_id, true);
      // Clears any drop the user has now recovered from, and gives the watch a baseline so the
      // next stop can be classified.
      self.note_tunnel_connected(&request.tunnel_id);
      // Connecting to a node is the clearest possible statement that it is the one you want.
      let _ = self.set_current_node_id(db, Some(&request.node_id));
      let _ = db.transition_vpn_node_status(&request.node_id, NodeLifecycleStatus::Connected, None);
      return Ok(());
    }

    let _ = wireguard.unregister_tunnel(&canonical_tunnel_name);
    let _ = db.update_tunnel_status(&request.tunnel_id, TunnelStatus::Ready, Some("Tunnel runtime did not report Running; service was removed."));
    let _ = db.set_tunnel_connected_at(&request.tunnel_id, false);

    Err(OrchestratorError::InvalidState {
      node_id: request.node_id.clone(),
      detail: "Tunnel did not report Running after activation. Retry connect or inspect WireGuard logs."
        .to_string(),
    })
  }

  pub fn disconnect_tunnel(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
    secure_storage: &SecureStorage,
    request: &DisconnectTunnelRequest,
  ) -> Result<(), OrchestratorError> {
    let correlation_id = Uuid::new_v4().to_string();
    let activity = ActivityRecorder::new(db, "disconnect", Some(&correlation_id))
      .for_node(&request.node_id);

    let outcome = self.disconnect_tunnel_inner(wireguard, db, secure_storage, request);

    match &outcome {
      Ok(()) => activity.success(
        "disconnect.completed",
        &format!("tunnel_id={} was stopped on request", request.tunnel_id),
      ),
      Err(error) => activity.failure("disconnect.error", error),
    }

    outcome
  }

  fn disconnect_tunnel_inner(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
    _secure_storage: &SecureStorage,
    request: &DisconnectTunnelRequest,
  ) -> Result<(), OrchestratorError> {
    let node = db
      .get_vpn_node(&request.node_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed loading vpn node before disconnect flow.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: "Disconnect requested for a vpn node that does not exist.".to_string(),
      })?;

    if !matches!(
      node.lifecycle_status,
      NodeLifecycleStatus::Connected | NodeLifecycleStatus::Ready
    ) {
      return Err(OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: "Disconnect flow requires node lifecycle status Connected or Ready.".to_string(),
      });
    }

    let tunnel = db
      .get_tunnel(&request.tunnel_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed loading tunnel before disconnect flow.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::Database {
        detail: "Disconnect flow requested for unknown tunnel id.".to_string(),
        source: DbError::NotFound {
          entity: "tunnel",
          id: request.tunnel_id.clone(),
          detail: "No tunnel row matched the id for disconnect flow.".to_string(),
        },
      })?;

    if tunnel.node_id != request.node_id {
      return Err(OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: "Disconnect flow tunnel id does not belong to the requested node.".to_string(),
      });
    }

    // Recorded before the stop, so the poll that observes it knows this was intentional and does
    // not raise a drop alert.
    self.expect_tunnel_stop(&request.tunnel_id);

    // Unregister alone: it stops the service as part of removing it. Stopping separately first
    // only bought a second elevation prompt.
    wireguard
      .unregister_tunnel(&tunnel.name)
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not unregister WireGuard tunnel service in disconnect flow.".to_string(),
        source,
      })?;

    let _ = db.update_tunnel_stats_snapshot(&request.tunnel_id, None, None, None);
    let _ = db.set_tunnel_connected_at(&request.tunnel_id, false);

    db
      .update_tunnel_status(&request.tunnel_id, TunnelStatus::Disconnected, None)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed persisting tunnel status after disconnect flow.".to_string(),
        source,
      })?;

    let _ = db.transition_vpn_node_status(&request.node_id, NodeLifecycleStatus::Ready, None);
    Ok(())
  }

  pub fn force_disconnect_node_service(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
    node_id: &str,
  ) -> Result<(), OrchestratorError> {
    let node = db
      .get_vpn_node(node_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed loading vpn node before force-disconnect flow.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::InvalidState {
        node_id: node_id.to_string(),
        detail: "Force disconnect requested for a vpn node that does not exist.".to_string(),
      })?;

    let tunnels = db
      .list_tunnels_for_node(node_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed listing tunnels before force-disconnect flow.".to_string(),
        source,
      })?;

    for tunnel in tunnels {
      // Deliberate teardown, so none of these stops should read as a drop.
      self.expect_tunnel_stop(&tunnel.id);
      // Unregister stops the service as part of removal, so one elevated call does both.
      let _ = wireguard.unregister_tunnel(&tunnel.name);
      let _ = db.update_tunnel_stats_snapshot(&tunnel.id, None, None, None);
      let _ = db.set_tunnel_connected_at(&tunnel.id, false);
      let _ = db.update_tunnel_status(&tunnel.id, TunnelStatus::Disconnected, Some("Best-effort force disconnect requested from dashboard."));
    }

    let _ = db.transition_vpn_node_status(&node.id, NodeLifecycleStatus::Ready, Some("Best-effort force disconnect requested from dashboard."));
    Ok(())
  }

  pub fn read_client_tunnel_status(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
    tunnel_id: &str,
  ) -> Result<TunnelRuntimeStatus, OrchestratorError> {
    let tunnel = db
      .get_tunnel(tunnel_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed loading tunnel before status read.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::Database {
        detail: "Tunnel status requested for unknown id.".to_string(),
        source: DbError::NotFound {
          entity: "tunnel",
          id: tunnel_id.to_string(),
          detail: "No tunnel row matched the id for status query.".to_string(),
        },
      })?;

    let status = match wireguard.read_tunnel_status(&tunnel.name) {
      Ok(runtime) => runtime,
      Err(WireGuardError::TunnelNotFound { .. }) => TunnelRuntimeStatus {
        tunnel_name: tunnel.name.clone(),
        state: crate::backend::wireguard::TunnelServiceState::Stopped,
        last_handshake_age_seconds: None,
        rx_bytes: None,
        tx_bytes: None,
        metrics_source: None,
      },
      Err(source) => {
        return Err(OrchestratorError::WireGuard {
          detail: "Could not read WireGuard runtime status.".to_string(),
          source,
        })
      }
    };

    let db_status = match status.state {
      crate::backend::wireguard::TunnelServiceState::Running => TunnelStatus::Connected,
      crate::backend::wireguard::TunnelServiceState::Stopped => TunnelStatus::Disconnected,
      crate::backend::wireguard::TunnelServiceState::Unknown => tunnel.status,
    };
    let _ = db.update_tunnel_status(tunnel_id, db_status, None);
    let _ = persist_tunnel_stats_snapshot(db, tunnel_id, &status);

    Ok(status)
  }

  pub fn get_health_summary(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
    secure_storage: &SecureStorage,
    node_id: &str,
    tunnel_id: Option<&str>,
    // Bypasses the egress-IP cache. Set when the user explicitly asks for a refresh, so the
    // button does something the ambient poll would not.
    force_ip_probe: bool,
  ) -> Result<HealthSummary, OrchestratorError> {
    let node = db
      .get_vpn_node(node_id)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed loading vpn node for health summary.".to_string(),
        source,
      })?
      .ok_or_else(|| OrchestratorError::InvalidState {
        node_id: node_id.to_string(),
        detail: "Health summary requested for unknown node.".to_string(),
      })?;

    let selected_tunnel = match tunnel_id {
      Some(tunnel_id) => db.get_tunnel(tunnel_id).map_err(|source| OrchestratorError::Database {
        detail: "Failed loading selected tunnel for health summary.".to_string(),
        source,
      })?,
      None => db
        .list_tunnels_for_node(node_id)
        .map_err(|source| OrchestratorError::Database {
          detail: "Failed listing tunnels for health summary.".to_string(),
          source,
        })?
        .into_iter()
        .next(),
    };

    if let Some(tunnel) = selected_tunnel.as_ref() {      if let Ok(provider) = db.get_provider(&node.provider_id) {
        if let Some(provider) = provider {
          let tunnel_settings = self.get_tunnel_settings(db).unwrap_or_default();
          if let Ok(config) = load_connect_config_for_tunnel(secure_storage, &provider.slug, &node, tunnel, &tunnel_settings) {
            // Populate diagnostics trace even when only health polling occurs.
            wireguard.record_connect_trace_snapshot(&tunnel.name, &config);
          } else {
            let _ = wireguard.capture_runtime_connect_trace_snapshot(&tunnel.name);
          }
        } else {
          let _ = wireguard.capture_runtime_connect_trace_snapshot(&tunnel.name);
        }
      } else {
        let _ = wireguard.capture_runtime_connect_trace_snapshot(&tunnel.name);
      }
    }

    // A tunnel with no local service is simply not connected, and health polls every few
    // seconds. Treating the absence as an error meant a recurring "WireGuard issue" toast for
    // the entire normal period after a disconnect — unregistering removes the service, so the
    // very next poll reported a failure — and for any tunnel whose connect never completed.
    let runtime = match selected_tunnel.as_ref() {
      Some(tunnel) => match wireguard.read_tunnel_status(&tunnel.name) {
        Ok(status) => Some(status),
        Err(WireGuardError::TunnelNotFound { .. }) => None,
        Err(source) => {
          return Err(OrchestratorError::WireGuard {
            detail: "Failed reading tunnel runtime for health summary.".to_string(),
            source,
          });
        }
      },
      None => None,
    };

    // Absent runtime means the service is gone, so the node is not connected regardless of what
    // the stored status says.
    let service_missing = selected_tunnel.is_some() && runtime.is_none();

    let runtime_status = runtime.as_ref().map(|value| match value.state {
      crate::backend::wireguard::TunnelServiceState::Running => TunnelStatus::Connected,
      crate::backend::wireguard::TunnelServiceState::Stopped => TunnelStatus::Disconnected,
      crate::backend::wireguard::TunnelServiceState::Unknown => TunnelStatus::Error,
    });

    let effective_status = if service_missing {
      Some(TunnelStatus::Disconnected)
    } else {
      runtime_status.or_else(|| selected_tunnel.as_ref().map(|tunnel| tunnel.status))
    };

    // Reconcile the stored row against what the service actually reports. Without this the
    // dashboard read a stale row and showed "Idle" with a Connect button while a tunnel was up.
    if let (Some(tunnel), Some(status)) = (selected_tunnel.as_ref(), effective_status) {
      self.reconcile_tunnel_status(db, tunnel, status);
    }

    // A tunnel that stopped on its own took WireGuard's WFP filters down with it, so traffic is
    // flowing unprotected. Nothing used to notice; the row was rewritten and the badge flipped.
    let unexpected_drop = selected_tunnel.as_ref().and_then(|tunnel| {
      self.observe_tunnel_runtime(
        db,
        &tunnel.id,
        &node.id,
        matches!(effective_status, Some(TunnelStatus::Connected)),
      )
    });

    // Derived from the routes the running config carries, not the stored column: those differ for
    // a locally-scoped endpoint, and the column would claim protection the tunnel does not have.
    let kill_switch = {
      let connected = matches!(effective_status, Some(TunnelStatus::Connected));
      match selected_tunnel.as_ref() {
        Some(tunnel) => {
          let routes = scope_allowed_ips_for_endpoint(
            parse_allowed_ips_csv(tunnel.allowed_ips.as_deref()),
            node.public_ipv4.as_deref().unwrap_or_default(),
            tunnel.client_address_cidr.as_deref().unwrap_or(DEFAULT_CLIENT_CIDR),
          );
          derive_kill_switch_state(&routes, connected)
        }
        None => derive_kill_switch_state(&[], false),
      }
    };

    // Measured in every state, so the card can show the address changing as the tunnel comes up.
    // Cached, because health polls far more often than an egress address changes.
    let connected = matches!(effective_status, Some(TunnelStatus::Connected));
    let (external_ipv4, external_ipv6) = self.external_ips(connected, force_ip_probe);

    Ok(HealthSummary {
      node_id: node.id,
      tunnel_id: selected_tunnel.as_ref().map(|tunnel| tunnel.id.clone()),
      endpoint_ip: node.public_ipv4.clone(),
      handshake_age_seconds: runtime.as_ref().and_then(|value| value.last_handshake_age_seconds),
      rx_bytes: runtime.as_ref().and_then(|value| value.rx_bytes),
      tx_bytes: runtime.as_ref().and_then(|value| value.tx_bytes),
      external_ipv4,
      external_ipv6,
      tunnel_status: effective_status,
      metrics_source: runtime.as_ref().and_then(|value| value.metrics_source),
      connected_at: if matches!(effective_status, Some(TunnelStatus::Connected)) {
        selected_tunnel.as_ref().and_then(|tunnel| tunnel.connected_at.clone())
      } else {
        None
      },
      kill_switch,
      unexpected_drop,
    })
  }

  /// Write a runtime-observed status back onto the stored tunnel row.
  ///
  /// Health polling runs every few seconds, so this writes only when the value actually changed —
  /// otherwise every poll would churn `updated_at`. An `Error` status is left alone because it
  /// carries a `last_error` the user still needs; a real connect or disconnect clears it.
  fn reconcile_tunnel_status(&self, db: &Database, tunnel: &TunnelRecord, observed: TunnelStatus) {
    if tunnel.status == observed || tunnel.status == TunnelStatus::Error {
      return;
    }

    if db.update_tunnel_status(&tunnel.id, observed, None).is_ok() {
      // Session timing follows the reconciled state. A tunnel found running whose start time was
      // never recorded keeps `connected_at = NULL`: the true start is unknowable here, and
      // stamping "now" would show a timer counting from an arbitrary moment.
      if observed == TunnelStatus::Disconnected {
        let _ = db.set_tunnel_connected_at(&tunnel.id, false);
      }
    }
  }

  pub fn build_diagnostics_bundle(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
  ) -> Result<DiagnosticsBundle, OrchestratorError> {
    let providers = db.list_providers().map_err(|source| OrchestratorError::Database {
      detail: "Failed loading providers for diagnostics bundle.".to_string(),
      source,
    })?;
    let nodes = db.list_vpn_nodes().map_err(|source| OrchestratorError::Database {
      detail: "Failed loading nodes for diagnostics bundle.".to_string(),
      source,
    })?;

    let mut tunnels = Vec::new();
    for node in &nodes {
      tunnels.extend(db.list_tunnels_for_node(&node.id).map_err(|source| OrchestratorError::Database {
        detail: "Failed loading tunnels for diagnostics bundle.".to_string(),
        source,
      })?);
    }

    let activity = db.list_activity_events(200).map_err(|source| OrchestratorError::Database {
      detail: "Failed loading activity history for diagnostics bundle.".to_string(),
      source,
    })?;

    let providers = providers
      .into_iter()
      .map(|mut provider| {
        provider.token_ref = None;
        provider
      })
      .collect::<Vec<_>>();

    let nodes = nodes
      .into_iter()
      .map(|mut node| {
        node.error_reason = node
          .error_reason
          .map(|value| logging::redact_sensitive_text(&value));
        node
      })
      .collect::<Vec<_>>();

    let tunnels = tunnels
      .into_iter()
      .map(|mut tunnel| {
        tunnel.last_error = tunnel
          .last_error
          .map(|value| logging::redact_sensitive_text(&value));
        tunnel
      })
      .collect::<Vec<_>>();

    // Redacted again on read as defence in depth: rows written before 0017 came from a path that
    // redacted, but the bundle leaves the process and cheap belt-and-braces is worth it here.
    let activity = activity
      .into_iter()
      .map(|mut event| {
        event.detail = logging::redact_sensitive_text(&event.detail);
        event
      })
      .collect::<Vec<_>>();

    let mut redacted_notes = Vec::new();
    for node in &nodes {
      if let Some(reason) = &node.error_reason {
        redacted_notes.push(format!(
          "node:{} error_reason:{}",
          node.id,
          logging::redact_sensitive_text(reason)
        ));
      }
    }
    for tunnel in &tunnels {
      if let Some(reason) = &tunnel.last_error {
        redacted_notes.push(format!(
          "tunnel:{} last_error:{}",
          tunnel.id,
          logging::redact_sensitive_text(reason)
        ));
      }
    }
    for event in &activity {
      if event.outcome.eq_ignore_ascii_case("error") {
        redacted_notes.push(format!(
          "activity:{}:{} stage={} detail={}",
          event.node_id.as_deref().unwrap_or("app"),
          event.category,
          event.stage,
          logging::redact_sensitive_text(&event.detail)
        ));
      }
    }

    Ok(DiagnosticsBundle {
      generated_at: epoch_millis_label(),
      providers,
      nodes,
      tunnels,
      activity,
      redacted_notes,
      wireguard_debug: wireguard.debug_snapshot(),
    })
  }
  pub fn reconcile_startup(
    &self,
    providers: &ProviderRegistry,
    wireguard: &WireGuardManager,
    db: &Database,
    secure_storage: &SecureStorage,
  ) -> Result<ReconcileReport, OrchestratorError> {
    // Startup is the natural moment to trim: it happens often enough to keep the table bounded and
    // never competes with a connect the user is waiting on.
    let _ = db.prune_activity_events(ACTIVITY_HISTORY_LIMIT);

    let mut report = ReconcileReport::default();
    let nodes = db
      .list_vpn_nodes()
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed listing vpn nodes during startup reconciliation.".to_string(),
        source,
      })?;

    for node in nodes {
      if matches!(node.lifecycle_status, NodeLifecycleStatus::Ready | NodeLifecycleStatus::Connected) {
        match node.external_ref.as_deref() {
          None => {
            let detail = "Node is Ready/Connected but has no droplet reference.".to_string();
            let _ = db.transition_vpn_node_status(&node.id, NodeLifecycleStatus::Error, Some(&detail));
            report.issues.push(ReconcileIssue {
              kind: ReconcileIssueKind::MissingNodeRuntimeData,
              node_id: Some(node.id.clone()),
              tunnel_id: None,
              repaired: false,
              detail,
            });
          }
          Some(external_ref) => {
            if let Ok(droplet_id) = external_ref.parse::<u64>() {
              match providers.provider_droplet_exists(
                db,
                secure_storage,
                &node.provider_id,
                droplet_id,
              ) {
                Ok(true) => {}
                Ok(false) => {
                  let detail = format!(
                    "Provider droplet '{}' no longer exists for node '{}'.",
                    droplet_id, node.id
                  );
                  let _ = db.transition_vpn_node_status(
                    &node.id,
                    NodeLifecycleStatus::Error,
                    Some(&detail),
                  );
                  report.issues.push(ReconcileIssue {
                    kind: ReconcileIssueKind::MissingDroplet,
                    node_id: Some(node.id.clone()),
                    tunnel_id: None,
                    repaired: false,
                    detail,
                  });
                }
                Err(err) => {
                  report.issues.push(ReconcileIssue {
                    kind: ReconcileIssueKind::ProviderCheckFailed,
                    node_id: Some(node.id.clone()),
                    tunnel_id: None,
                    repaired: false,
                    detail: format!("Provider droplet existence check failed: {err}"),
                  });
                }
              }
            } else {
              let detail = "Node droplet reference is invalid and cannot be parsed as numeric id."
                .to_string();
              let _ = db.transition_vpn_node_status(&node.id, NodeLifecycleStatus::Error, Some(&detail));
              report.issues.push(ReconcileIssue {
                kind: ReconcileIssueKind::MissingNodeRuntimeData,
                node_id: Some(node.id.clone()),
                tunnel_id: None,
                repaired: false,
                detail,
              });
            }
          }
        }
      }

      let tunnels = db
        .list_tunnels_for_node(&node.id)
        .map_err(|source| OrchestratorError::Database {
          detail: "Failed listing tunnels during startup reconciliation.".to_string(),
          source,
        })?;

      let mut any_connected = false;
      for tunnel in tunnels {
        match wireguard.read_tunnel_status(&tunnel.name) {
          Ok(runtime) => {
            any_connected |= runtime.state == crate::backend::wireguard::TunnelServiceState::Running;
            let _ = persist_tunnel_stats_snapshot(db, &tunnel.id, &runtime);

            let reconciled_status = match runtime.state {
              crate::backend::wireguard::TunnelServiceState::Running => TunnelStatus::Connected,
              crate::backend::wireguard::TunnelServiceState::Stopped => TunnelStatus::Disconnected,
              crate::backend::wireguard::TunnelServiceState::Unknown => tunnel.status,
            };

            if reconciled_status != tunnel.status {
              let _ = db.update_tunnel_status(&tunnel.id, reconciled_status, None);
              // A session that ended ends its clock. One found already running keeps whatever
              // start time it has — including none, when the app did not start it.
              if reconciled_status == TunnelStatus::Disconnected {
                let _ = db.set_tunnel_connected_at(&tunnel.id, false);
              }
              report.issues.push(ReconcileIssue {
                kind: ReconcileIssueKind::StaleTunnelStatusRepaired,
                node_id: Some(node.id.clone()),
                tunnel_id: Some(tunnel.id.clone()),
                repaired: true,
                detail: format!(
                  "Tunnel status repaired from '{}' to '{}'.",
                  tunnel.status,
                  reconciled_status
                ),
              });
            }
          }
          Err(WireGuardError::TunnelNotFound { .. }) => {
            if tunnel.status == TunnelStatus::Connected {
              let _ = db.update_tunnel_status(&tunnel.id, TunnelStatus::Disconnected, None);
              let _ = db.set_tunnel_connected_at(&tunnel.id, false);
              report.issues.push(ReconcileIssue {
                kind: ReconcileIssueKind::StaleTunnelStatusRepaired,
                node_id: Some(node.id.clone()),
                tunnel_id: Some(tunnel.id.clone()),
                repaired: true,
                detail: "Tunnel status repaired from 'connected' to 'disconnected' because no local WireGuard service exists."
                  .to_string(),
              });
            }
          }
          Err(WireGuardError::NotInstalled { .. } | WireGuardError::UnsupportedPlatform { .. }) => {
            report.issues.push(ReconcileIssue {
              kind: ReconcileIssueKind::StartupDependencyUnavailable,
              node_id: Some(node.id.clone()),
              tunnel_id: Some(tunnel.id.clone()),
              repaired: false,
              detail: "WireGuard runtime tooling is unavailable; install/repair WireGuard for Windows."
                .to_string(),
            });
          }
          Err(other) => {
            let detail = format!("Runtime status probe failed: {other}");
            let _ = db.update_tunnel_status(&tunnel.id, TunnelStatus::Error, Some(&detail));
            report.issues.push(ReconcileIssue {
              kind: ReconcileIssueKind::StartupDependencyUnavailable,
              node_id: Some(node.id.clone()),
              tunnel_id: Some(tunnel.id.clone()),
              repaired: false,
              detail,
            });
          }
        }
      }

      if node.lifecycle_status == NodeLifecycleStatus::Connected && !any_connected {
        let _ = db.transition_vpn_node_status(&node.id, NodeLifecycleStatus::Ready, None);
        report.issues.push(ReconcileIssue {
          kind: ReconcileIssueKind::StaleTunnelStatusRepaired,
          node_id: Some(node.id.clone()),
          tunnel_id: None,
          repaired: true,
          detail: "Node lifecycle was Connected but no running tunnel was found; set node to Ready."
            .to_string(),
        });
      }

      if node.lifecycle_status == NodeLifecycleStatus::Ready && any_connected {
        let _ = db.transition_vpn_node_status(&node.id, NodeLifecycleStatus::Connected, None);
        report.issues.push(ReconcileIssue {
          kind: ReconcileIssueKind::StaleTunnelStatusRepaired,
          node_id: Some(node.id.clone()),
          tunnel_id: None,
          repaired: true,
          detail: "Node lifecycle was Ready but active tunnel exists; set node to Connected."
            .to_string(),
        });
      }
    }

    // Startup is the one moment the app can notice something that changed while it was not
    // running — a tunnel stopped, a droplet deleted from the provider's own console. Recording it
    // means the user sees that it happened, not just the repaired end state.
    for issue in &report.issues {
      let mut recorder = ActivityRecorder::new(db, "app", None);
      if let Some(node_id) = issue.node_id.as_deref() {
        recorder = recorder.for_node(node_id);
      }
      recorder.record(
        "startup.reconcile",
        if issue.repaired { "info" } else { "error" },
        &issue.detail,
      );
    }

    Ok(report)
  }
}

/// Unix epoch milliseconds as a bare number.
///
/// Used for `generated_at` and therefore for the exported diagnostics filename, so it must stay
/// free of characters that are illegal in a Windows filename.
fn epoch_millis() -> u64 {
  use std::time::{SystemTime, UNIX_EPOCH};
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|duration| duration.as_millis() as u64)
    .unwrap_or_default()
}

fn epoch_millis_label() -> String {
  format!("{}", epoch_millis())
}

fn persist_tunnel_stats_snapshot(
  db: &Database,
  tunnel_id: &str,
  status: &TunnelRuntimeStatus,
) -> Result<(), OrchestratorError> {
  db
    .update_tunnel_stats_snapshot(
      tunnel_id,
      status.last_handshake_age_seconds.map(|value| value as i64),
      status.rx_bytes.map(|value| value as i64),
      status.tx_bytes.map(|value| value as i64),
    )
    .map_err(|source| OrchestratorError::Database {
      detail: "Failed persisting tunnel runtime stats snapshot.".to_string(),
      source,
    })
}

/// Whether an endpoint is reachable only through this machine's own network.
///
/// Loopback, RFC1918, and link-local endpoints all describe a server that sits behind this host
/// rather than out on the internet — a local container, a VM on a host-only network, a lab box.
/// Judged from the endpoint itself rather than the provider slug, so the orchestrator stays
/// provider-neutral. A real VPN server has a publicly routable endpoint, so this never matches
/// one.
fn endpoint_is_locally_scoped(endpoint: &str) -> bool {
  match endpoint.trim().parse::<std::net::IpAddr>() {
    Ok(std::net::IpAddr::V4(address)) => {
      address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_unspecified()
    }
    Ok(std::net::IpAddr::V6(address)) => address.is_loopback() || address.is_unspecified(),
    Err(_) => false,
  }
}

/// Narrow a default route to the tunnel subnet when the server is not publicly routable.
///
/// A full-tunnel route (`0.0.0.0/0`) to a local endpoint sends the host's entire default route
/// into a server reachable only *through* that same host. For the Docker dev provider that means
/// all traffic depends on the container's onward NAT, and a failure there takes the machine off
/// the network — with the app that caused it now unreachable over that network too.
///
/// A real provider endpoint is publicly routable and is untouched, so full-tunnel remains the
/// behaviour that matters in production.
fn scope_allowed_ips_for_endpoint(
  allowed_ips: Vec<String>,
  endpoint: &str,
  client_address_cidr: &str,
) -> Vec<String> {
  if !endpoint_is_locally_scoped(endpoint) {
    return allowed_ips;
  }

  let carries_default_route = allowed_ips
    .iter()
    .any(|entry| matches!(entry.trim(), "0.0.0.0/0" | "::/0"));

  if !carries_default_route {
    return allowed_ips;
  }

  // Keep only the tunnel's own subnet, derived from the client address so it tracks the
  // configured CIDR rather than a hardcoded range.
  let subnet = tunnel_subnet_for_client(client_address_cidr);
  log::warn!(
    "event=connect.allowed_ips.scoped endpoint_is_locally_scoped=true scoped_to={} reason=default_route_to_local_endpoint",
    subnet
  );

  vec![subnet]
}

/// Whether WireGuard's built-in leak protection is in force for these routes.
///
/// Mirrors wireguard-windows' own rule: with a single peer and no `Table = off` — both always true
/// of configs this app generates — a `/0` route in `AllowedIPs` makes it install WFP filters that
/// block untunneled traffic. Anything narrower and it restricts nothing.
///
/// `routes` must be the routes the *running config* carries, which is not always what the tunnel
/// row stores: `scope_allowed_ips_for_endpoint` rewrites them for locally-scoped endpoints, so a
/// Docker dev tunnel is unprotected while its stored `allowed_ips` still reads `0.0.0.0/0`.
fn derive_kill_switch_state(routes: &[String], connected: bool) -> KillSwitchState {
  let blocks_ipv4 = routes.iter().any(|entry| entry.trim() == "0.0.0.0/0");
  let blocks_ipv6 = routes.iter().any(|entry| entry.trim() == "::/0");
  let carries_default_route = blocks_ipv4 || blocks_ipv6;

  let reason = if !carries_default_route {
    KillSwitchReason::RoutesScoped
  } else if !connected {
    KillSwitchReason::TunnelNotRunning
  } else {
    KillSwitchReason::Active
  };

  KillSwitchState {
    active: matches!(reason, KillSwitchReason::Active),
    // Reported for the routes themselves, so the UI can say which families would be covered even
    // while the tunnel is down.
    blocks_ipv4,
    blocks_ipv6,
    reason,
    routes: routes.to_vec(),
  }
}

/// `10.66.0.2/32` -> `10.66.0.0/24`: the subnet shared with the server.
fn tunnel_subnet_for_client(client_address_cidr: &str) -> String {
  let address = client_address_cidr.split('/').next().unwrap_or_default().trim();

  match address.parse::<std::net::Ipv4Addr>() {
    Ok(ipv4) => {
      let octets = ipv4.octets();
      format!("{}.{}.{}.0/24", octets[0], octets[1], octets[2])
    }
    Err(_) => DEFAULT_CLIENT_CIDR.to_string(),
  }
}

fn parse_allowed_ips_csv(raw: Option<&str>) -> Vec<String> {
  let mut parsed = raw
    .unwrap_or_default()
    .split(',')
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(ToOwned::to_owned)
    .collect::<Vec<_>>();

  if parsed.is_empty() {
    parsed = DEFAULT_CLIENT_ALLOWED_IPS
      .iter()
      .map(|value| (*value).to_string())
      .collect();
  }

  parsed
}

fn load_connect_config_for_tunnel(
  secure_storage: &SecureStorage,
  _provider_slug: &str,
  node: &crate::backend::db::VpnNodeRecord,
  tunnel: &crate::backend::db::TunnelRecord,
  settings: &TunnelSettings,
) -> Result<String, OrchestratorError> {
  let server_public_key = node
    .server_public_key
    .clone()
    .ok_or_else(|| OrchestratorError::InvalidState {
      node_id: node.id.clone(),
      detail: "VPN node is missing server public key; cannot connect tunnel.".to_string(),
    })?;

  let endpoint_ip = resolve_server_endpoint_ip_for_connect(node)?;

  let private_key_ref = tunnel
    .client_private_key_ref
    .as_deref()
    .ok_or_else(|| OrchestratorError::InvalidState {
      node_id: node.id.clone(),
      detail: "Tunnel is unconfigured (missing secure private-key reference). Configure tunnel first."
        .to_string(),
    })?;

  let private_key = secure_storage
    .get_client_private_key(private_key_ref)
    .map_err(|source| OrchestratorError::SecretStorage {
      detail: "Could not read tunnel private key from secure storage during connect.".to_string(),
      source,
    })?
    .ok_or_else(|| OrchestratorError::InvalidState {
      node_id: node.id.clone(),
      detail: "Tunnel private key reference is stale in secure storage. Reconfigure the tunnel and retry connect."
        .to_string(),
    })?;

  let client_address_cidr = tunnel
    .client_address_cidr
    .clone()
    .filter(|value| !value.trim().is_empty())
    .unwrap_or_else(|| DEFAULT_CLIENT_CIDR.to_string());

  let allowed_ips = scope_allowed_ips_for_endpoint(
    parse_allowed_ips_csv(tunnel.allowed_ips.as_deref()),
    &endpoint_ip,
    &client_address_cidr,
  );

  let config = ClientConfigParams {
    private_key,
    client_address_cidr,
    // From settings, not the tunnel row. The row records what the last connect used, which keeps
    // diagnostics accurate, but a single source of truth decides what the next one will use.
    dns_server: settings.dns_server.clone(),
    server_public_key,
    server_endpoint_ip: endpoint_ip,
    // Reported by the provider adapter at provision time. Falls back to the default only for
    // nodes created before per-node ports existed.
    server_listen_port: node.endpoint_port.unwrap_or(DEFAULT_WIREGUARD_PORT),
    allowed_ips,
    persistent_keepalive_seconds: settings.keepalive_seconds,
  };

  crate::backend::wireguard::build_client_config(&config)
    .map_err(|source| OrchestratorError::WireGuard {
      detail: "Could not render client WireGuard config during connect flow.".to_string(),
      source,
    })
}

/// The endpoint comes from whatever the provider adapter reported at provision time.
///
/// This used to special-case `provider_slug == "docker"` and return a hardcoded `127.0.0.1`.
/// Adapters now report their own endpoint — Docker persists `127.0.0.1` plus its published host
/// port like any other node — so the orchestrator no longer needs to know Docker exists.
fn resolve_server_endpoint_ip_for_connect(
  node: &crate::backend::db::VpnNodeRecord,
) -> Result<String, OrchestratorError> {
  node.public_ipv4.clone().ok_or_else(|| OrchestratorError::InvalidState {
    node_id: node.id.clone(),
    detail: "VPN node is missing public endpoint IP; cannot connect tunnel.".to_string(),
  })
}

#[cfg(test)]
mod tests {
  use std::sync::{Arc, Mutex};

  use std::path::PathBuf;
  use std::time::{SystemTime, UNIX_EPOCH};

  use super::*;
  use crate::backend::provider::{
    Account, BootstrapWaitRequest, ProviderAdapter, ProviderAdapterError, ProviderDropletStatus,
  };
  use crate::backend::secure_storage::SecureStorage;
  use crate::backend::provider::ProviderRegistry;
  use crate::backend::wireguard::{CommandExecError, CommandOutput, CommandRunner};

  const TEST_PROVIDER_ID: &str = "provider-1";

  struct MockAdapterState {
    delete_calls: usize,
    droplet_exists: bool,
    droplet_status: ProviderDropletStatus,
  }

  impl Default for MockAdapterState {
    fn default() -> Self {
      Self {
        delete_calls: 0,
        droplet_exists: true,
        droplet_status: ProviderDropletStatus::Active,
      }
    }
  }

  struct MockProviderAdapter {
    fail_bootstrap_wait: bool,
    state: Arc<Mutex<MockAdapterState>>,
  }

  impl MockProviderAdapter {
    fn new(fail_bootstrap_wait: bool) -> Self {
      Self {
        fail_bootstrap_wait,
        state: Arc::new(Mutex::new(MockAdapterState::default())),
      }
    }
  }

  impl ProviderAdapter for MockProviderAdapter {
    fn provider_name(&self) -> &'static str {
      "digitalocean"
    }

    fn validate_token(&self, _token: &str) -> Result<Account, ProviderAdapterError> {
      Ok(Account {
        uuid: "acct-1".to_string(),
        email: "vpn@example.com".to_string(),
        status: "active".to_string(),
      })
    }

    fn list_regions(&self, _token: &str) -> Result<Vec<ProviderRegion>, ProviderAdapterError> {
      Ok(vec![self.describe_region("nyc1"), self.describe_region("sfo3")])
    }

    fn describe_region(&self, slug: &str) -> ProviderRegion {
      crate::backend::provider::digitalocean_regions::describe_region(slug)
    }

    fn list_plan_pricing(
      &self,
      _token: &str,
      region: &str,
    ) -> Result<Vec<crate::backend::provider::ProviderPlanPricing>, ProviderAdapterError> {
      Ok(vec![
        crate::backend::provider::ProviderPlanPricing {
          plan_key: "s-1vcpu-1gb".to_string(),
          plan_label: "Basic".to_string(),
          size_slug: "s-1vcpu-1gb".to_string(),
          region: region.to_string(),
          memory_mb: 1024,
          vcpus: 1,
          disk_gb: 25,
          transfer_tb: 1.0,
          transfer_note: "Includes up to 1.00 TB transfer before provider overage policies apply.".to_string(),
          monthly_usd: 6.0,
          hourly_usd: 0.00893,
          estimated: false,
          source_timestamp: "0".to_string(),
        },
        crate::backend::provider::ProviderPlanPricing {
          plan_key: "s-2vcpu-2gb".to_string(),
          plan_label: "Basic".to_string(),
          size_slug: "s-2vcpu-2gb".to_string(),
          region: region.to_string(),
          memory_mb: 2048,
          vcpus: 2,
          disk_gb: 60,
          transfer_tb: 3.0,
          transfer_note: "Includes up to 3.00 TB transfer before provider overage policies apply.".to_string(),
          monthly_usd: 12.0,
          hourly_usd: 0.01786,
          estimated: false,
          source_timestamp: "0".to_string(),
        },
      ])
    }

    fn create_droplet(
      &self,
      _token: &str,
      _droplet_name: &str,
      _region: &str,
      _size_slug: &str,
      _init_payload: &str,
    ) -> Result<ProvisionedDroplet, ProviderAdapterError> {
      Ok(ProvisionedDroplet {
        droplet_id: 42,
        public_ipv4: "203.0.113.90".to_string(),
        endpoint_port: DEFAULT_WIREGUARD_PORT,
      })
    }

    fn wait_for_bootstrap_ready(
      &self,
      _token: &str,
      request: &BootstrapWaitRequest,
    ) -> Result<crate::backend::bootstrap::BootstrapOutputContract, ProviderAdapterError> {
      if self.fail_bootstrap_wait {
        return Err(ProviderAdapterError::Unsupported {
          detail: "mock bootstrap marker not ready".to_string(),
        });
      }

      Ok(crate::backend::bootstrap::BootstrapOutputContract {
        server_endpoint_ip: request.endpoint_ip.clone(),
        server_public_key: "server-pub-key-123".to_string(),
        listen_port: DEFAULT_WIREGUARD_PORT,
      })
    }

    fn delete_droplet(&self, _token: &str, _droplet_id: u64) -> Result<(), ProviderAdapterError> {
      let mut state = self.state.lock().expect("lock must work");
      state.delete_calls += 1;
      Ok(())
    }

    fn droplet_exists(&self, _token: &str, _droplet_id: u64) -> Result<bool, ProviderAdapterError> {
      let state = self.state.lock().expect("lock must work");
      Ok(state.droplet_exists)
    }

    fn droplet_status(&self, _token: &str, _droplet_id: u64) -> Result<ProviderDropletStatus, ProviderAdapterError> {
      let state = self.state.lock().expect("lock must work");
      Ok(state.droplet_status)
    }
  }

  #[derive(Clone)]
  struct MockWireGuardRunner {
    state: Arc<Mutex<MockWireGuardState>>,
  }

  #[derive(Clone)]
  struct MockWireGuardState {
    service_exists: bool,
    service_running: bool,
    install_creates_service: bool,
    deactivate_calls: usize,
    activate_calls: usize,
    install_calls: usize,
    unregister_calls: usize,
  }

  impl MockWireGuardRunner {
    fn running() -> Self {
      Self {
        state: Arc::new(Mutex::new(MockWireGuardState {
          service_exists: true,
          service_running: true,
          install_creates_service: true,
          deactivate_calls: 0,
          activate_calls: 0,
          install_calls: 0,
          unregister_calls: 0,
        })),
      }
    }

    fn stopped() -> Self {
      Self {
        state: Arc::new(Mutex::new(MockWireGuardState {
          service_exists: true,
          service_running: false,
          install_creates_service: true,
          deactivate_calls: 0,
          activate_calls: 0,
          install_calls: 0,
          unregister_calls: 0,
        })),
      }
    }

    fn missing_service() -> Self {
      Self {
        state: Arc::new(Mutex::new(MockWireGuardState {
          service_exists: false,
          service_running: false,
          install_creates_service: true,
          deactivate_calls: 0,
          activate_calls: 0,
          install_calls: 0,
          unregister_calls: 0,
        })),
      }
    }

    fn missing_service_unrecoverable() -> Self {
      Self {
        state: Arc::new(Mutex::new(MockWireGuardState {
          service_exists: false,
          service_running: false,
          install_creates_service: false,
          deactivate_calls: 0,
          activate_calls: 0,
          install_calls: 0,
          unregister_calls: 0,
        })),
      }
    }
  }

  impl CommandRunner for MockWireGuardRunner {
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

      if program_name == "wg" && args == ["genkey"] {
        return Ok(CommandOutput {
          stdout: "client-private-key".to_string(),
        });
      }

      if program_name == "wg" && args == ["pubkey"] {
        return Ok(CommandOutput {
          stdout: "client-public-key".to_string(),
        });
      }

      if program_name == "sc" && args.len() >= 2 {
        let mut state = self.state.lock().expect("lock must succeed");
        match args[0] {
          "query" => {
            if !state.service_exists {
              return Err(CommandExecError::NonZeroExit {
                program: "sc".to_string(),
                code: Some(1060),
                detail: "service missing".to_string(),
              });
            }

            let service_state = if state.service_running {
              "STATE              : 4  RUNNING"
            } else {
              "STATE              : 1  STOPPED"
            };

            return Ok(CommandOutput {
              stdout: service_state.to_string(),
            });
          }
          "start" => {
            state.activate_calls += 1;
            if !state.service_exists {
              return Err(CommandExecError::NonZeroExit {
                program: "sc".to_string(),
                code: Some(1060),
                detail: "service missing".to_string(),
              });
            }
            state.service_running = true;
            return Ok(CommandOutput {
              stdout: "started".to_string(),
            });
          }
          "stop" => {
            if !state.service_exists {
              return Err(CommandExecError::NonZeroExit {
                program: "sc".to_string(),
                code: Some(1060),
                detail: "service missing".to_string(),
              });
            }
            state.service_running = false;
            state.deactivate_calls += 1;
            return Ok(CommandOutput {
              stdout: "stopped".to_string(),
            });
          }
          _ => {}
        }
      }

      if program_name == "wireguard" && args.len() == 2 && args[0] == "/uninstalltunnelservice" {
        let mut state = self.state.lock().expect("lock must succeed");
        state.unregister_calls += 1;
        state.service_exists = false;
        state.service_running = false;
        return Ok(CommandOutput {
          stdout: format!("uninstalled {}", args[1]),
        });
      }

      if program_name == "wireguard" && args.len() == 2 && args[0] == "/installtunnelservice" {
        let mut state = self.state.lock().expect("lock must succeed");
        state.install_calls += 1;
        if state.install_creates_service {
          state.service_exists = true;
          state.service_running = false;
        }
        return Ok(CommandOutput {
          stdout: format!("installed {}", args[1]),
        });
      }

      if program_name == "wg" && args.len() == 3 && args[0] == "show" && args[2] == "dump" {
        let state = self.state.lock().expect("lock must succeed");
        if !state.service_exists {
          return Err(CommandExecError::NonZeroExit {
            program: "sc".to_string(),
            code: Some(1060),
            detail: "service missing".to_string(),
          });
        }

        let handshake = if state.service_running { "1700000050" } else { "0" };
        let dump = format!(
          "iface-private\tiface-public\t51820\toff\npeerpub\t(none)\t203.0.113.10:51820\t0.0.0.0/0\t{}\t10\t20\t25\n",
          handshake
        );
        return Ok(CommandOutput {
          stdout: dump,
        });
      }

      if program_name == "wireguard" && args == ["/uninstalltunnelservice", "wg-node-partial"] {
        let mut state = self.state.lock().expect("lock must succeed");
        state.unregister_calls += 1;
        state.service_exists = false;
        state.service_running = false;
        return Ok(CommandOutput {
          stdout: "uninstalled".to_string(),
        });
      }

      Err(CommandExecError::NonZeroExit {
        program: program.to_string(),
        code: Some(1),
        detail: "unsupported mock command".to_string(),
      })
    }
  }

  fn setup_connected_node_and_tunnel(db: &Database) {
    db
      .create_vpn_node("node-connect", TEST_PROVIDER_ID, "Node Connect", "nyc1")
      .expect("node should insert");
    db
      .transition_vpn_node_status("node-connect", NodeLifecycleStatus::Provisioning, None)
      .expect("transition to provisioning should succeed");
    db
      .transition_vpn_node_status("node-connect", NodeLifecycleStatus::Bootstrapping, None)
      .expect("transition to bootstrapping should succeed");
    db
      .transition_vpn_node_status("node-connect", NodeLifecycleStatus::Ready, None)
      .expect("transition to ready should succeed");
    db
      .update_vpn_node_runtime_details(
        "node-connect",
        Some("42"),
        Some("203.0.113.10"),
        Some("server-pub-key-123"),
        Some(51820),
      )
      .expect("runtime details should persist");
    db
      .create_tunnel("tunnel-connect", "node-connect", "wg-node-connect", Some("wg-node-connect"))
      .expect("tunnel should insert");
    db
      .update_tunnel_status("tunnel-connect", TunnelStatus::Connected, None)
      .expect("tunnel status should update");
    db
      .transition_vpn_node_status("node-connect", NodeLifecycleStatus::Connected, None)
      .expect("node should transition to connected");
  }

  fn setup_ready_node_and_tunnel(db: &Database, secure_storage: &SecureStorage) {
    db
      .create_vpn_node("node-ready", TEST_PROVIDER_ID, "Node Ready", "nyc1")
      .expect("node should insert");
    db
      .transition_vpn_node_status("node-ready", NodeLifecycleStatus::Provisioning, None)
      .expect("transition to provisioning should succeed");
    db
      .transition_vpn_node_status("node-ready", NodeLifecycleStatus::Bootstrapping, None)
      .expect("transition to bootstrapping should succeed");
    db
      .transition_vpn_node_status("node-ready", NodeLifecycleStatus::Ready, None)
      .expect("transition to ready should succeed");
    db
      .update_vpn_node_runtime_details(
        "node-ready",
        Some("42"),
        Some("203.0.113.10"),
        Some("server-pub-key-123"),
        Some(51820),
      )
      .expect("runtime details should persist");
    db
      .create_tunnel("tunnel-ready", "node-ready", "Primary-Node-wg", Some("Primary-Node-wg"))
      .expect("tunnel should insert");
    let key_ref = secure_storage
      .store_client_private_key("node-ready", "client-private-key")
      .expect("client private key should persist");
    db
      .update_tunnel_client_config(
        "tunnel-ready",
        Some(&key_ref),
        Some(DEFAULT_CLIENT_CIDR),
        Some("1.1.1.1"),
        Some("0.0.0.0/0,::/0"),
      )
      .expect("tunnel client config should persist");
    db
      .update_tunnel_status("tunnel-ready", TunnelStatus::Ready, None)
      .expect("tunnel status should update");
  }

  fn test_db_path(test_name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("clock must be after unix epoch")
      .as_nanos();
    path.push(format!("goatstare_orchestrator_{test_name}_{nanos}.sqlite3"));
    path
  }

  fn setup_provider_and_token(db: &Database, secure_storage: &SecureStorage) {
    let token_ref = secure_storage
      .store_digitalocean_api_token(TEST_PROVIDER_ID, "token-abc")
      .expect("token should store");

    db
      .update_provider_token_ref(TEST_PROVIDER_ID, Some(&token_ref))
      .expect("provider token ref should update");
  }

  #[test]
  fn create_vpn_node_success_transitions_to_ready() {
    let db = Database::new(test_db_path("create_success"));
    db.run_migrations().expect("migrations should succeed");

    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);

    let adapter = Arc::new(MockProviderAdapter::new(false));
    let providers = ProviderRegistry::from_adapter(adapter);
    let orchestrator = Orchestrator::new();
    let bootstrap = Bootstrap::new();

    let result = orchestrator
      .create_vpn_node_to_ready(
        &providers,
        &bootstrap,
        &db,
        &secure_storage,
        &CreateVpnNodeRequest {
          node_id: "node-1".to_string(),
          provider_id: TEST_PROVIDER_ID.to_string(),
          node_name: "node-main".to_string(),
          region: "nyc1".to_string(),
          plan_key: "s-1vcpu-1gb".to_string(),
        },
      )
      .expect("create flow should succeed");

    assert_eq!(result.status, NodeLifecycleStatus::Ready);
    assert_eq!(result.droplet_id, 42);
    assert_eq!(result.server_public_key, "server-pub-key-123");

    let node = db
      .get_vpn_node("node-1")
      .expect("query should succeed")
      .expect("node should exist");
    assert_eq!(node.lifecycle_status, NodeLifecycleStatus::Ready);
    assert_eq!(node.server_public_key.as_deref(), Some("server-pub-key-123"));
  }

  #[test]
  fn create_vpn_node_failure_rolls_back_droplet_and_marks_error() {
    let db = Database::new(test_db_path("create_fail"));
    db.run_migrations().expect("migrations should succeed");

    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);

    let adapter = Arc::new(MockProviderAdapter::new(true));
    let state = adapter.state.clone();
    let providers = ProviderRegistry::from_adapter(adapter);

    let orchestrator = Orchestrator::new();
    let bootstrap = Bootstrap::new();

    let result = orchestrator.create_vpn_node_to_ready(
      &providers,
      &bootstrap,
      &db,
      &secure_storage,
      &CreateVpnNodeRequest {
        node_id: "node-2".to_string(),
        provider_id: TEST_PROVIDER_ID.to_string(),
        node_name: "node-main".to_string(),
        region: "nyc1".to_string(),
        plan_key: "s-1vcpu-1gb".to_string(),
      },
    );

    assert!(result.is_err());

    let node = db
      .get_vpn_node("node-2")
      .expect("query should succeed")
      .expect("node should exist");
    assert_eq!(node.lifecycle_status, NodeLifecycleStatus::Error);
    assert!(node.error_reason.unwrap_or_default().contains("create_vpn_node_to_ready failed"));

    let delete_calls = state.lock().expect("lock should work").delete_calls;
    assert_eq!(delete_calls, 1);
  }

  #[test]
  fn list_nodes_enriches_live_provider_status() {
    let db = Database::new(test_db_path("list_nodes_live_status"));
    db.run_migrations().expect("migrations should succeed");

    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);

    db
      .create_vpn_node("node-1", TEST_PROVIDER_ID, "Node", "nyc1")
      .expect("node should insert");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Provisioning, None)
      .expect("transition should succeed");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Bootstrapping, None)
      .expect("transition should succeed");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Ready, None)
      .expect("transition should succeed");
    db
      .update_vpn_node_runtime_details(
        "node-1",
        Some("42"),
        Some("203.0.113.10"),
        Some("server-pub-key-123"),
        Some(51820),
      )
      .expect("runtime details should persist");

    let adapter = Arc::new(MockProviderAdapter::new(false));
    {
      let mut state = adapter.state.lock().expect("lock should work");
      state.droplet_status = ProviderDropletStatus::Active;
    }
    let providers = ProviderRegistry::from_adapter(adapter);
    let orchestrator = Orchestrator::new();

    let nodes = orchestrator
      .list_nodes(&providers, &db, &secure_storage)
      .expect("list nodes should succeed");

    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].provider_status.as_deref(), Some("active"));
  }

  #[test]
  fn list_nodes_marks_missing_remote_as_deleted() {
    let db = Database::new(test_db_path("list_nodes_deleted_status"));
    db.run_migrations().expect("migrations should succeed");

    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);

    db
      .create_vpn_node("node-1", TEST_PROVIDER_ID, "Node", "nyc1")
      .expect("node should insert");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Provisioning, None)
      .expect("transition should succeed");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Bootstrapping, None)
      .expect("transition should succeed");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Ready, None)
      .expect("transition should succeed");
    db
      .update_vpn_node_runtime_details(
        "node-1",
        Some("42"),
        Some("203.0.113.10"),
        Some("server-pub-key-123"),
        Some(51820),
      )
      .expect("runtime details should persist");

    let adapter = Arc::new(MockProviderAdapter::new(false));
    {
      let mut state = adapter.state.lock().expect("lock should work");
      state.droplet_status = ProviderDropletStatus::Deleted;
    }
    let providers = ProviderRegistry::from_adapter(adapter);
    let orchestrator = Orchestrator::new();

    let nodes = orchestrator
      .list_nodes(&providers, &db, &secure_storage)
      .expect("list nodes should succeed");

    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].provider_status.as_deref(), Some("deleted"));
  }

  #[test]
  fn connect_flow_requires_existing_tunnel() {
    let db = Database::new(test_db_path("connect_missing_tunnel"));
    db.run_migrations().expect("migrations should succeed");
    db
      .update_provider_token_ref(TEST_PROVIDER_ID, Some("do-token:provider-1:abc"))
      .expect("provider token ref should update");
    db
      .create_vpn_node("node-1", TEST_PROVIDER_ID, "Node", "nyc1")
      .expect("node should insert");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Provisioning, None)
      .expect("transition should succeed");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Bootstrapping, None)
      .expect("transition should succeed");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Ready, None)
      .expect("transition should succeed");

    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::stopped()));
    let orchestrator = Orchestrator::new();
    let secure_storage = SecureStorage::dev_mock();
    let result = orchestrator.connect_tunnel(
      &wireguard,
      &db,
      &secure_storage,
      &ConnectTunnelRequest {
        node_id: "node-1".to_string(),
        tunnel_id: "missing-tunnel".to_string(),
      },
    );

    assert!(matches!(
      result,
      Err(OrchestratorError::Database {
        source: DbError::NotFound { .. },
        ..
      })
    ));
  }

  #[test]
  fn connect_flow_imports_service_on_demand_and_connects() {
    let db = Database::new(test_db_path("connect_on_demand_service_import"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_ready_node_and_tunnel(&db, &secure_storage);

    let runner = Arc::new(MockWireGuardRunner::missing_service());
    let wireguard = WireGuardManager::with_runner(runner.clone());
    let orchestrator = Orchestrator::new();

    orchestrator
      .connect_tunnel(
        &wireguard,
        &db,
        &secure_storage,
        &ConnectTunnelRequest {
          node_id: "node-ready".to_string(),
          tunnel_id: "tunnel-ready".to_string(),
        },
      )
      .expect("connect should import and activate tunnel service on demand");

    let tunnel = db
      .get_tunnel("tunnel-ready")
      .expect("query should succeed")
      .expect("tunnel should exist");
    assert_eq!(tunnel.status, TunnelStatus::Connected);

    let node = db
      .get_vpn_node("node-ready")
      .expect("query should succeed")
      .expect("node should exist");
    assert_eq!(node.lifecycle_status, NodeLifecycleStatus::Connected);

    let state = runner.state.lock().expect("lock should work");
    assert_eq!(state.install_calls, 1);
    assert_eq!(state.activate_calls, 1);
  }

  #[test]
  fn connect_flow_fails_when_service_import_does_not_create_service() {
    let db = Database::new(test_db_path("connect_import_missing_service"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_ready_node_and_tunnel(&db, &secure_storage);

    let runner = Arc::new(MockWireGuardRunner::missing_service_unrecoverable());
    let wireguard = WireGuardManager::with_runner(runner.clone());
    let orchestrator = Orchestrator::new();

    let result = orchestrator.connect_tunnel(
      &wireguard,
      &db,
      &secure_storage,
      &ConnectTunnelRequest {
        node_id: "node-ready".to_string(),
        tunnel_id: "tunnel-ready".to_string(),
      },
    );

    assert!(matches!(
      result,
      Err(OrchestratorError::WireGuard {
        source: WireGuardError::TunnelNotFound { .. },
        ..
      })
    ));

    let state = runner.state.lock().expect("lock should work");
    assert_eq!(state.install_calls, 1);
    assert_eq!(state.activate_calls, 0);
  }

  /// The endpoint is whatever the adapter reported, with no provider branching. A Docker node
  /// persists `127.0.0.1` because its adapter said so, not because the orchestrator special-cases
  /// the slug.
  #[test]
  fn connect_endpoint_uses_adapter_reported_endpoint_verbatim() {
    let node = crate::backend::db::VpnNodeRecord {
      id: "node-docker".to_string(),
      provider_id: TEST_PROVIDER_ID.to_string(),
      name: "Docker Node".to_string(),
      region: "docker-local".to_string(),
      plan_key: None,
      size_slug: None,
      price_monthly_usd: None,
      price_hourly_usd: None,
      price_source_timestamp: None,
      lifecycle_status: NodeLifecycleStatus::Ready,
      provider_status: None,
      region_geo: None,
      external_ref: Some("42".to_string()),
      public_ipv4: Some("127.0.0.1".to_string()),
      server_public_key: Some("server-public-key".to_string()),
      error_reason: None,
      created_at: "now".to_string(),
      updated_at: "now".to_string(),
      client_private_key_ref: None,
      client_public_key: None,
      endpoint_port: Some(49154),
    };

    let endpoint = resolve_server_endpoint_ip_for_connect(&node)
      .expect("endpoint resolution should succeed");

    assert_eq!(endpoint, "127.0.0.1");
    // The published host port travels with the node, so two Docker nodes can coexist.
    assert_eq!(node.endpoint_port, Some(49154));
  }

  /// A default route to a server on this machine would take the host off the network.
  #[test]
  fn full_tunnel_is_scoped_to_the_subnet_for_a_loopback_endpoint() {
    let scoped = scope_allowed_ips_for_endpoint(
      vec!["0.0.0.0/0".to_string(), "::/0".to_string()],
      "127.0.0.1",
      "10.66.0.2/32",
    );

    assert_eq!(scoped, vec!["10.66.0.0/24".to_string()]);
  }

  /// Full-tunnel is the point of a real VPN and must survive untouched.
  #[test]
  fn full_tunnel_is_preserved_for_a_real_provider_endpoint() {
    let allowed = vec!["0.0.0.0/0".to_string(), "::/0".to_string()];
    let scoped = scope_allowed_ips_for_endpoint(allowed.clone(), "203.0.113.10", "10.66.0.2/32");

    assert_eq!(scoped, allowed);
  }

  /// A server on a private network is behind this host just as a loopback one is.
  ///
  /// The Docker dev provider reports the WSL2 VM's RFC1918 address when the daemon lives there,
  /// so scoping cannot key on loopback alone.
  #[test]
  fn full_tunnel_is_scoped_for_private_and_link_local_endpoints() {
    for endpoint in ["172.20.34.93", "192.168.1.50", "10.0.0.7", "169.254.10.1"] {
      let scoped = scope_allowed_ips_for_endpoint(
        vec!["0.0.0.0/0".to_string(), "::/0".to_string()],
        endpoint,
        "10.66.0.2/32",
      );

      assert_eq!(
        scoped,
        vec!["10.66.0.0/24".to_string()],
        "a default route to {endpoint} must be scoped"
      );
    }
  }

  #[test]
  fn public_endpoints_are_not_treated_as_locally_scoped() {
    assert!(!endpoint_is_locally_scoped("203.0.113.10"));
    assert!(!endpoint_is_locally_scoped("8.8.8.8"));
    assert!(endpoint_is_locally_scoped("127.0.0.1"));
    assert!(endpoint_is_locally_scoped("172.20.34.93"));
    // A hostname cannot be judged, so it is left alone rather than guessed at.
    assert!(!endpoint_is_locally_scoped("vpn.example.com"));
  }

  /// A split-tunnel selection is the user's choice and is left alone either way.
  #[test]
  fn split_tunnel_selection_is_left_untouched_on_loopback() {
    let allowed = vec!["10.66.0.0/24".to_string()];
    let scoped = scope_allowed_ips_for_endpoint(allowed.clone(), "127.0.0.1", "10.66.0.2/32");

    assert_eq!(scoped, allowed);
  }

  #[test]
  fn tunnel_subnet_is_derived_from_the_configured_client_address() {
    assert_eq!(tunnel_subnet_for_client("10.66.0.2/32"), "10.66.0.0/24");
    assert_eq!(tunnel_subnet_for_client("192.168.240.7/32"), "192.168.240.0/24");
    // Unparseable input falls back to the documented default rather than inventing a route.
    assert_eq!(tunnel_subnet_for_client("not-an-address"), DEFAULT_CLIENT_CIDR);
  }

  #[test]
  fn connect_endpoint_requires_public_ip() {
    let node = crate::backend::db::VpnNodeRecord {
      id: "node-do".to_string(),
      provider_id: TEST_PROVIDER_ID.to_string(),
      name: "DO Node".to_string(),
      region: "nyc1".to_string(),
      plan_key: None,
      size_slug: None,
      price_monthly_usd: None,
      price_hourly_usd: None,
      price_source_timestamp: None,
      lifecycle_status: NodeLifecycleStatus::Ready,
      provider_status: None,
      region_geo: None,
      external_ref: Some("77".to_string()),
      public_ipv4: None,
      server_public_key: Some("server-public-key".to_string()),
      error_reason: None,
      created_at: "now".to_string(),
      updated_at: "now".to_string(),
      client_private_key_ref: None,
      client_public_key: None,
      endpoint_port: None,
    };

    let result = resolve_server_endpoint_ip_for_connect(&node);

    assert!(matches!(result, Err(OrchestratorError::InvalidState { .. })));
  }

  #[test]
  fn reconcile_flags_deleted_droplet() {
    let db = Database::new(test_db_path("reconcile_deleted_droplet"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);
    db
      .create_vpn_node("node-1", TEST_PROVIDER_ID, "Node", "nyc1")
      .expect("node should insert");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Provisioning, None)
      .expect("transition should succeed");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Bootstrapping, None)
      .expect("transition should succeed");
    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Ready, None)
      .expect("transition should succeed");
    db
      .update_vpn_node_runtime_details(
        "node-1",
        Some("99"),
        Some("203.0.113.10"),
        Some("server-pub-key-123"),
        Some(51820),
      )
      .expect("runtime details should persist");

    let adapter = Arc::new(MockProviderAdapter::new(false));
    {
      let mut state = adapter.state.lock().expect("lock should work");
      state.droplet_exists = false;
    }
    let providers = ProviderRegistry::from_adapter(adapter);
    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::stopped()));

    let report = Orchestrator::new()
      .reconcile_startup(&providers, &wireguard, &db, &secure_storage)
      .expect("reconcile should complete");

    assert!(report
      .issues
      .iter()
      .any(|issue| issue.kind == ReconcileIssueKind::MissingDroplet));

    let node = db
      .get_vpn_node("node-1")
      .expect("query should succeed")
      .expect("node should exist");
    assert_eq!(node.lifecycle_status, NodeLifecycleStatus::Error);
  }

  #[test]
  fn reconcile_repairs_stale_connected_status_to_ready() {
    let db = Database::new(test_db_path("reconcile_stale_status"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);
    setup_connected_node_and_tunnel(&db);
    let providers = ProviderRegistry::from_adapter(Arc::new(MockProviderAdapter::new(false)));
    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::stopped()));

    let report = Orchestrator::new()
      .reconcile_startup(&providers, &wireguard, &db, &secure_storage)
      .expect("reconcile should complete");

    assert!(report
      .issues
      .iter()
      .any(|issue| issue.kind == ReconcileIssueKind::StaleTunnelStatusRepaired));

    let tunnel = db
      .get_tunnel("tunnel-connect")
      .expect("query should succeed")
      .expect("tunnel should exist");
    assert_eq!(tunnel.status, TunnelStatus::Disconnected);
    assert!(tunnel.stats_updated_at.is_some());

    let node = db
      .get_vpn_node("node-connect")
      .expect("query should succeed")
      .expect("node should exist");
    assert_eq!(node.lifecycle_status, NodeLifecycleStatus::Ready);
  }

  #[test]
  fn reconcile_marks_connected_tunnel_as_disconnected_when_service_missing() {
    let db = Database::new(test_db_path("reconcile_missing_tunnel_artifact"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);
    setup_connected_node_and_tunnel(&db);
    let providers = ProviderRegistry::from_adapter(Arc::new(MockProviderAdapter::new(false)));
    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::missing_service()));

    let report = Orchestrator::new()
      .reconcile_startup(&providers, &wireguard, &db, &secure_storage)
      .expect("reconcile should complete");

    assert!(report
      .issues
      .iter()
      .any(|issue| issue.kind == ReconcileIssueKind::StaleTunnelStatusRepaired));

    assert!(report
      .issues
      .iter()
      .any(|issue| issue.kind == ReconcileIssueKind::StaleTunnelStatusRepaired && issue.repaired));

    let tunnel = db
      .get_tunnel("tunnel-connect")
      .expect("query should succeed");
    let tunnel = tunnel.expect("tunnel should remain as metadata record");
    assert_eq!(tunnel.status, TunnelStatus::Disconnected);
  }

  #[test]
  fn force_cleanup_removes_partial_provisioning_state() {
    let db = Database::new(test_db_path("force_cleanup_partial"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);
    db
      .create_vpn_node("node-partial", TEST_PROVIDER_ID, "Node Partial", "nyc1")
      .expect("node should insert");
    db
      .transition_vpn_node_status("node-partial", NodeLifecycleStatus::Provisioning, None)
      .expect("transition should succeed");
    db
      .update_vpn_node_runtime_details(
        "node-partial",
        Some("42"),
        Some("203.0.113.5"),
        None,
        Some(51820),
      )
      .expect("runtime details should persist");
    db
      .create_tunnel("tunnel-partial", "node-partial", "wg-node-partial", Some("wg-node-partial"))
      .expect("tunnel should insert");

    let adapter = Arc::new(MockProviderAdapter::new(false));
    let provider_state = adapter.state.clone();
    let providers = ProviderRegistry::from_adapter(adapter);
    let wireguard_runner = Arc::new(MockWireGuardRunner::stopped());
    let wireguard = WireGuardManager::with_runner(wireguard_runner.clone());

    let result = Orchestrator::new()
      .force_cleanup_vpn_node(&providers, &wireguard, &db, &secure_storage, "node-partial")
      .expect("force cleanup should complete");

    assert_eq!(result.local_tunnels_removed, 1);
    assert!(result.remote_droplet_delete_attempted);
    assert!(result.remote_droplet_delete_succeeded);
    assert!(result.node_record_removed);

    assert!(db.get_vpn_node("node-partial").expect("query should succeed").is_none());
    assert!(db.get_tunnel("tunnel-partial").expect("query should succeed").is_none());

    let delete_calls = provider_state.lock().expect("lock should work").delete_calls;
    assert_eq!(delete_calls, 1);

    let wireguard_state = wireguard_runner.state.lock().expect("lock should work");
    // Unregister stops the service as part of removing it, so cleanup issues one privileged
    // command rather than a stop followed by an uninstall.
    assert_eq!(wireguard_state.deactivate_calls, 0);
    assert_eq!(wireguard_state.unregister_calls, 1);
  }

  #[test]
  fn diagnostics_bundle_redacts_provider_token_refs() {
    let db = Database::new(test_db_path("diagnostics_redaction"));
    db.run_migrations().expect("migrations should succeed");
    let wireguard = WireGuardManager::new();

    setup_provider_and_token(&db, &SecureStorage::dev_mock());

    let bundle = Orchestrator::new()
      .build_diagnostics_bundle(&wireguard, &db)
      .expect("bundle should build");

    assert!(bundle.providers.iter().all(|provider| provider.token_ref.is_none()));

    let bundle_json = serde_json::to_string(&bundle).expect("bundle should serialize");
    assert!(!bundle_json.contains("token_ref"));
  }

  #[test]
  fn wireguard_start_timeout_maps_to_wait_then_retry_guidance() {
    let error = OrchestratorError::WireGuard {
      detail: "Could not start the WireGuard tunnel service. If the tunnel was just imported, wait a few seconds and retry once. If it still fails, the tunnel configuration may be invalid, the service may require elevation, or the Windows WireGuard installation may be unhealthy.".to_string(),
      source: crate::backend::wireguard::WireGuardError::Command {
        command: "sc start WireGuardTunnel$Primary-Node-wg".to_string(),
        detail: "The service did not respond to the start request in a timely fashion.".to_string(),
        source: crate::backend::wireguard::CommandExecError::NonZeroExit {
          program: "sc".to_string(),
          code: Some(1053),
          detail: "The service did not respond to the start request in a timely fashion.".to_string(),
        },
      },
    };

    let user_facing = error.to_user_facing();

    assert_eq!(user_facing.kind, UserFacingErrorKind::WireGuard);
    assert!(user_facing.message.contains("did not respond to the start request in a timely fashion"));
    assert!(user_facing.next_action.contains("retry Connect"));
  }

  /// A tunnel with no local service is "not connected", not a failure.
  ///
  /// Health polls every few seconds, so returning an error here produced a recurring
  /// "WireGuard issue" toast throughout the entire normal period after a disconnect — unregister
  /// removes the service, so the next poll immediately reported a failure — and for any tunnel
  /// whose connect never completed.
  #[test]
  fn health_summary_reports_a_missing_tunnel_service_as_disconnected() {
    let db = Database::new(test_db_path("health_missing_service"));
    db.run_migrations().expect("migrations should succeed");

    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);

    db.create_vpn_node("node-health", TEST_PROVIDER_ID, "Health Node", "nyc1")
      .expect("node should insert");
    db.transition_vpn_node_status(&"node-health".to_string(), NodeLifecycleStatus::Provisioning, None)
      .expect("transition to provisioning");
    db.transition_vpn_node_status(&"node-health".to_string(), NodeLifecycleStatus::Bootstrapping, None)
      .expect("transition to bootstrapping");
    db.transition_vpn_node_status(&"node-health".to_string(), NodeLifecycleStatus::Ready, None)
      .expect("transition to ready");
    db.update_vpn_node_runtime_details(
      "node-health",
      Some("42"),
      Some("203.0.113.10"),
      Some("server-pub-key"),
      Some(51820),
    )
    .expect("runtime details should persist");

    db.create_tunnel("tunnel-health", "node-health", "wg-missing", Some("wg-missing"))
      .expect("tunnel should insert");
    // The stored status still claims connected, as it would right after a service disappeared.
    db.update_tunnel_status("tunnel-health", TunnelStatus::Connected, None)
      .expect("tunnel status should persist");

    // Reports no such service, exactly as `sc query` does for a tunnel that was never installed.
    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::missing_service()));

    let summary = Orchestrator::with_ip_probe(StaticIpProbe::none())
      .get_health_summary(&wireguard, &db, &secure_storage, "node-health", Some("tunnel-health"), false)
      .expect("a missing tunnel service must not fail the health summary");

    assert_eq!(summary.tunnel_status, Some(TunnelStatus::Disconnected));
    assert_eq!(summary.handshake_age_seconds, None);
  }

  /// A single node is unambiguously the one you mean, so it needs no explicit choice.
  #[test]
  fn a_lone_node_is_current_without_being_chosen() {
    let db = Database::new(test_db_path("current_single"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);
    db.create_vpn_node("node-only", TEST_PROVIDER_ID, "Only", "nyc1").expect("node inserts");

    let current = Orchestrator::new().get_current_node_id(&db).expect("read should succeed");
    assert_eq!(current.as_deref(), Some("node-only"));
  }

  /// With two nodes and no choice made, guessing would aim the connect button at an arbitrary
  /// server. The honest answer is that there is no current node yet.
  #[test]
  fn two_nodes_with_no_choice_have_no_current_node() {
    let db = Database::new(test_db_path("current_ambiguous"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);
    db.create_vpn_node("node-a", TEST_PROVIDER_ID, "A", "nyc1").expect("node inserts");
    db.create_vpn_node("node-b", TEST_PROVIDER_ID, "B", "sfo3").expect("node inserts");

    let orchestrator = Orchestrator::new();
    assert_eq!(orchestrator.get_current_node_id(&db).expect("read"), None);

    orchestrator.set_current_node_id(&db, Some("node-b")).expect("set should succeed");
    assert_eq!(
      orchestrator.get_current_node_id(&db).expect("read").as_deref(),
      Some("node-b")
    );
  }

  /// A pointer to a destroyed node would send connect at a server that no longer exists. Three
  /// nodes, so the survivors stay ambiguous and the stale id cannot be masked by the lone-node
  /// fallback.
  #[test]
  fn a_current_node_pointing_at_a_deleted_node_is_discarded() {
    let db = Database::new(test_db_path("current_stale"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);
    for (id, region) in [("node-a", "nyc1"), ("node-b", "sfo3"), ("node-c", "fra1")] {
      db.create_vpn_node(id, TEST_PROVIDER_ID, id, region).expect("node inserts");
    }

    let orchestrator = Orchestrator::new();
    orchestrator.set_current_node_id(&db, Some("node-a")).expect("set should succeed");
    db.delete_vpn_node("node-a").expect("node deletes");

    assert_eq!(
      orchestrator.get_current_node_id(&db).expect("read"),
      None,
      "a stale pointer must not resolve, and must not silently fall through to another node"
    );
    assert_eq!(
      db.get_setting("current_node_id").expect("read"),
      None,
      "the stale key should also be cleared, not just ignored on read"
    );
  }

  /// Destroying the last-but-one node leaves a single unambiguous survivor, which becomes current
  /// on its own — the user has exactly one server, so there is nothing to choose between.
  #[test]
  fn the_last_remaining_node_becomes_current_after_the_other_is_destroyed() {
    let db = Database::new(test_db_path("current_survivor"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);
    db.create_vpn_node("node-a", TEST_PROVIDER_ID, "A", "nyc1").expect("node inserts");
    db.create_vpn_node("node-b", TEST_PROVIDER_ID, "B", "sfo3").expect("node inserts");

    let orchestrator = Orchestrator::new();
    orchestrator.set_current_node_id(&db, Some("node-a")).expect("set should succeed");
    db.delete_vpn_node("node-a").expect("node deletes");

    assert_eq!(
      orchestrator.get_current_node_id(&db).expect("read").as_deref(),
      Some("node-b"),
    );
  }

  #[test]
  fn setting_an_unknown_node_as_current_is_rejected() {
    let db = Database::new(test_db_path("current_unknown"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);

    let error = Orchestrator::new()
      .set_current_node_id(&db, Some("node-ghost"))
      .expect_err("an unknown node must not become current");

    assert!(matches!(error, OrchestratorError::InvalidState { .. }));
  }

  /// Node rows carry only a slug; naming the location must work with no token and no network,
  /// because the fleet list stays visible when a provider key goes bad.
  #[test]
  fn listing_nodes_attaches_region_geography_without_a_provider_call() {
    let db = Database::new(test_db_path("region_enrich"));
    db.run_migrations().expect("migrations should succeed");
    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);
    db.create_vpn_node("node-sfo", TEST_PROVIDER_ID, "West", "sfo3").expect("node inserts");
    db.create_vpn_node("node-huh", TEST_PROVIDER_ID, "Mystery", "mars1").expect("node inserts");

    // Registry with no reachable API: enrichment must not depend on one.
    let providers = ProviderRegistry::from_adapter(Arc::new(MockProviderAdapter::new(false)));
    let nodes = Orchestrator::new()
      .list_nodes(&providers, &db, &secure_storage)
      .expect("listing should succeed");

    let sfo = nodes.iter().find(|node| node.id == "node-sfo").expect("node present");
    let geo = sfo.region_geo.as_ref().expect("a known slug should carry geography");
    assert_eq!(geo.city.as_deref(), Some("San Francisco"));
    assert_eq!(geo.country_code.as_deref(), Some("US"));

    let unknown = nodes.iter().find(|node| node.id == "node-huh").expect("node present");
    let geo = unknown.region_geo.as_ref().expect("an entry is still returned");
    assert_eq!(geo.slug, "mars1");
    assert_eq!(geo.city, None, "an unrecognised slug must not be given an invented city");
  }

  fn settings_db(label: &str) -> Database {
    let db = Database::new(test_db_path(label));
    db.run_migrations().expect("migrations should succeed");
    db
  }

  #[test]
  fn tunnel_settings_fall_back_to_the_shipped_defaults_when_unset() {
    let db = settings_db("settings_defaults");

    let settings = Orchestrator::new().get_tunnel_settings(&db).expect("read should succeed");
    assert_eq!(settings.dns_server.as_deref(), Some("1.1.1.1"));
    assert_eq!(settings.keepalive_seconds, 25);
  }

  #[test]
  fn tunnel_settings_round_trip() {
    let db = settings_db("settings_roundtrip");
    let orchestrator = Orchestrator::new();

    orchestrator
      .set_tunnel_settings(&db, &TunnelSettings {
        dns_server: Some("9.9.9.9".to_string()),
        keepalive_seconds: 15,
      })
      .expect("write should succeed");

    let settings = orchestrator.get_tunnel_settings(&db).expect("read should succeed");
    assert_eq!(settings.dns_server.as_deref(), Some("9.9.9.9"));
    assert_eq!(settings.keepalive_seconds, 15);
  }

  /// An invalid DNS value would render a `.conf` WireGuard rejects, failing every subsequent
  /// connect — so it is refused at the boundary rather than stored.
  #[test]
  fn setting_a_dns_server_that_is_not_an_ip_is_rejected() {
    let db = settings_db("settings_bad_dns");
    let orchestrator = Orchestrator::new();

    let error = orchestrator
      .set_tunnel_settings(&db, &TunnelSettings {
        dns_server: Some("not-an-ip".to_string()),
        keepalive_seconds: 25,
      })
      .expect_err("a non-IP DNS server must be rejected");

    assert!(matches!(error, OrchestratorError::InvalidInput { .. }));
    assert_eq!(
      db.get_setting("wireguard_dns_server").expect("read"),
      None,
      "a rejected value must not be persisted"
    );
  }

  /// Clearing DNS is meaningful — it leaves the system resolver in place — so an empty value is
  /// stored as "no DNS line" rather than falling back to the default.
  #[test]
  fn an_empty_dns_setting_means_no_dns_line_rather_than_the_default() {
    let db = settings_db("settings_empty_dns");
    let orchestrator = Orchestrator::new();

    orchestrator
      .set_tunnel_settings(&db, &TunnelSettings { dns_server: None, keepalive_seconds: 25 })
      .expect("write should succeed");

    assert_eq!(orchestrator.get_tunnel_settings(&db).expect("read").dns_server, None);
  }

  /// A value that no longer parses must not brick connect: it is treated as unset.
  #[test]
  fn a_corrupt_stored_setting_falls_back_instead_of_breaking_connect() {
    let db = settings_db("settings_corrupt");
    db.set_setting("wireguard_dns_server", Some("garbage")).expect("write");
    db.set_setting("wireguard_keepalive_seconds", Some("not-a-number")).expect("write");

    let settings = Orchestrator::new().get_tunnel_settings(&db).expect("read should succeed");
    assert_eq!(settings.dns_server.as_deref(), Some("1.1.1.1"));
    assert_eq!(settings.keepalive_seconds, 25);
  }

  #[test]
  fn keepalive_of_zero_is_stored_and_returned() {
    let db = settings_db("settings_keepalive_zero");
    let orchestrator = Orchestrator::new();

    orchestrator
      .set_tunnel_settings(&db, &TunnelSettings {
        dns_server: Some("1.1.1.1".to_string()),
        keepalive_seconds: 0,
      })
      .expect("write should succeed");

    assert_eq!(orchestrator.get_tunnel_settings(&db).expect("read").keepalive_seconds, 0);
  }

  /// The production case: default routes on a public endpoint, so WireGuard installs its filters.
  #[test]
  fn kill_switch_is_active_for_default_routes_on_a_running_tunnel() {
    let routes = vec!["0.0.0.0/0".to_string(), "::/0".to_string()];
    let state = derive_kill_switch_state(&routes, true);

    assert!(state.active);
    assert_eq!(state.reason, KillSwitchReason::Active);
    assert!(state.blocks_ipv4);
    assert!(state.blocks_ipv6);
  }

  /// The filters live with the tunnel service, so a stopped tunnel protects nothing — the routes
  /// still describe what *would* be covered, which is what the UI explains.
  #[test]
  fn kill_switch_is_inactive_while_the_tunnel_is_not_running() {
    let routes = vec!["0.0.0.0/0".to_string(), "::/0".to_string()];
    let state = derive_kill_switch_state(&routes, false);

    assert!(!state.active);
    assert_eq!(state.reason, KillSwitchReason::TunnelNotRunning);
    assert!(state.blocks_ipv4, "the route set is still reported");
  }

  /// A Docker dev tunnel has its default routes stripped, so WireGuard restricts nothing. Reading
  /// the stored `allowed_ips` column instead would claim protection that does not exist.
  #[test]
  fn kill_switch_is_inactive_when_routes_are_scoped_to_the_tunnel_subnet() {
    let scoped = scope_allowed_ips_for_endpoint(
      vec!["0.0.0.0/0".to_string(), "::/0".to_string()],
      "127.0.0.1",
      "10.66.0.2/32",
    );
    assert_eq!(scoped, vec!["10.66.0.0/24".to_string()], "precondition: routes are scoped");

    let state = derive_kill_switch_state(&scoped, true);
    assert!(!state.active, "a scoped tunnel must not claim leak protection");
    assert_eq!(state.reason, KillSwitchReason::RoutesScoped);
    assert!(!state.blocks_ipv4);
    assert!(!state.blocks_ipv6);
  }

  #[test]
  fn kill_switch_reports_partial_coverage_for_an_ipv4_only_default_route() {
    let state = derive_kill_switch_state(&["0.0.0.0/0".to_string()], true);

    assert!(state.active);
    assert!(state.blocks_ipv4);
    assert!(!state.blocks_ipv6);
  }

  /// The Amsterdam regression, at the level that caused it.
  ///
  /// `thiserror` prints only the outermost message, so the provider's actual complaint — "droplet
  /// name must be a valid hostname" — was attached to the error and never recorded anywhere the
  /// user or a later reader could see. Diagnosing it meant opening their SQLite file by hand.
  #[test]
  fn a_recorded_failure_carries_the_whole_cause_chain_not_just_the_outer_message() {
    #[derive(Debug, thiserror::Error)]
    #[error("the provider rejected the request")]
    struct Outer {
      #[source]
      source: Inner,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("name must be a valid hostname")]
    struct Inner;

    let db = watch_test_db("failure_chain");
    ActivityRecorder::new(&db, "provision", Some("run-1"))
      .for_node("node-a")
      .failure("create.error", &Outer { source: Inner });

    let events = db.list_activity_events(10).expect("query should succeed");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, "error");
    assert!(
      events[0].detail.contains("name must be a valid hostname"),
      "the cause must survive into the record: {}",
      events[0].detail
    );
    assert!(
      events[0].detail.contains("the provider rejected the request"),
      "the outer message must survive too: {}",
      events[0].detail
    );
  }

  /// A failed connect must leave a trail, and it must be tied to the attempt that produced it.
  #[test]
  fn a_failed_connect_records_start_and_error_under_one_correlation_id() {
    let orchestrator = Orchestrator::new();
    let db = watch_test_db("connect_failure_recorded");
    let wireguard = WireGuardManager::new();
    let secure_storage = SecureStorage::dev_mock();

    let result = orchestrator.connect_tunnel(
      &wireguard,
      &db,
      &secure_storage,
      &ConnectTunnelRequest {
        node_id: "node-missing".to_string(),
        tunnel_id: "tunnel-missing".to_string(),
      },
    );
    assert!(result.is_err(), "connecting to a node that does not exist must fail");

    let events = db.list_activity_events(10).expect("query should succeed");
    let stages: Vec<&str> = events.iter().map(|event| event.stage.as_str()).collect();
    assert!(stages.contains(&"connect.start"), "stages: {stages:?}");
    assert!(stages.contains(&"connect.error"), "stages: {stages:?}");

    let correlation_ids: HashSet<&str> = events
      .iter()
      .filter_map(|event| event.correlation_id.as_deref())
      .collect();
    assert_eq!(
      correlation_ids.len(),
      1,
      "both stages belong to the same attempt: {correlation_ids:?}"
    );

    let failure = events
      .iter()
      .find(|event| event.stage == "connect.error")
      .expect("the failure should be recorded");
    assert_eq!(failure.outcome, "error");
    assert_eq!(failure.category, "connect");
    assert_eq!(failure.node_id.as_deref(), Some("node-missing"));
  }

  fn watch_test_db(test_name: &str) -> Database {
    let db = Database::new(test_db_path(test_name));
    db.run_migrations().expect("migrations should succeed");
    db
  }

  /// The distinction the app could not previously make: a crash and a Disconnect click both land
  /// on `Disconnected`, but only one leaves the user unprotected without asking.
  #[test]
  fn a_tunnel_stopping_on_its_own_is_reported_as_an_unexpected_drop() {
    let orchestrator = Orchestrator::new();
    let db = watch_test_db("drop_reported");

    assert_eq!(
      orchestrator.observe_tunnel_runtime(&db, "tunnel-a", "node-a", true),
      None,
      "a healthy observation is not a drop"
    );

    let drop = orchestrator
      .observe_tunnel_runtime(&db, "tunnel-a", "node-a", false)
      .expect("a stop with no matching intent is a drop");
    assert_eq!(drop.tunnel_id, "tunnel-a");
    assert_eq!(drop.node_id, "node-a");
    assert!(drop.observed_at_epoch_ms > 0);

    // The banner is in-memory and gone on restart; the history is what survives to explain a gap
    // in protection the user may only notice later.
    let events = db.list_activity_events(10).expect("query should succeed");
    let dropped = events
      .iter()
      .find(|event| event.stage == "connect.dropped")
      .expect("the drop should be recorded in activity history");
    assert_eq!(dropped.outcome, "error");
    assert_eq!(dropped.node_id.as_deref(), Some("node-a"));
  }

  #[test]
  fn a_disconnect_the_app_asked_for_is_not_reported_as_a_drop() {
    let orchestrator = Orchestrator::new();
    let db = watch_test_db("drop_intentional");
    orchestrator.observe_tunnel_runtime(&db, "tunnel-a", "node-a", true);

    orchestrator.expect_tunnel_stop("tunnel-a");

    assert_eq!(
      orchestrator.observe_tunnel_runtime(&db, "tunnel-a", "node-a", false),
      None,
      "an intentional stop must not raise a drop alert"
    );

    let events = db.list_activity_events(10).expect("query should succeed");
    assert!(
      !events.iter().any(|event| event.stage == "connect.dropped"),
      "an intentional stop must not be recorded as a drop either"
    );
  }

  /// Polling continues after a drop; the alert must persist rather than clearing itself on the
  /// next identical observation.
  #[test]
  fn a_drop_persists_across_polls_until_the_tunnel_is_connected_again() {
    let orchestrator = Orchestrator::new();
    let db = watch_test_db("drop_persists");
    orchestrator.observe_tunnel_runtime(&db, "tunnel-a", "node-a", true);
    orchestrator.observe_tunnel_runtime(&db, "tunnel-a", "node-a", false);

    assert!(
      orchestrator.observe_tunnel_runtime(&db, "tunnel-a", "node-a", false).is_some(),
      "a repeat poll must keep reporting the drop"
    );

    assert_eq!(
      orchestrator.observe_tunnel_runtime(&db, "tunnel-a", "node-a", true),
      None,
      "reconnecting clears it"
    );

    // Health polls every 15s, so recording on read rather than on the transition would append a
    // row per poll for as long as the tunnel stayed down.
    let dropped = db
      .list_activity_events(50)
      .expect("query should succeed")
      .into_iter()
      .filter(|event| event.stage == "connect.dropped")
      .count();
    assert_eq!(dropped, 1, "one drop must record exactly one event");
  }

  /// A tunnel first seen stopped was not observed dropping, so warning about it would be a guess.
  #[test]
  fn a_tunnel_first_seen_stopped_is_not_a_drop() {
    let orchestrator = Orchestrator::new();
    let db = watch_test_db("drop_first_seen_stopped");

    assert_eq!(
      orchestrator.observe_tunnel_runtime(&db, "tunnel-a", "node-a", false),
      None
    );
  }

  /// Egress probe that answers from fixed values instead of the network.
  ///
  /// The health path probes on every call, so without this the normal `cargo test` run would make
  /// live HTTP requests to a third-party IP service — non-deterministic, and a network dependency
  /// the suite is not allowed to have.
  struct StaticIpProbe {
    ipv4: Option<String>,
    ipv6: Option<String>,
  }

  impl StaticIpProbe {
    fn none() -> Arc<dyn ExternalIpProbe> {
      Arc::new(Self { ipv4: None, ipv6: None })
    }

    fn v4(address: &str) -> Arc<dyn ExternalIpProbe> {
      Arc::new(Self { ipv4: Some(address.to_string()), ipv6: None })
    }

    fn both(v4: &str, v6: &str) -> Arc<dyn ExternalIpProbe> {
      Arc::new(Self { ipv4: Some(v4.to_string()), ipv6: Some(v6.to_string()) })
    }
  }

  impl ExternalIpProbe for StaticIpProbe {
    fn probe(&self, family: IpFamily) -> Option<String> {
      match family {
        IpFamily::V4 => self.ipv4.clone(),
        IpFamily::V6 => self.ipv6.clone(),
      }
    }
  }

  /// Sets up a ready node with one tunnel, so health-summary tests can vary only what matters.
  fn health_fixture(label: &str) -> (Database, SecureStorage) {
    let db = Database::new(test_db_path(label));
    db.run_migrations().expect("migrations should succeed");

    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);

    db.create_vpn_node("node-h", TEST_PROVIDER_ID, "Health Node", "nyc1")
      .expect("node should insert");
    for status in [
      NodeLifecycleStatus::Provisioning,
      NodeLifecycleStatus::Bootstrapping,
      NodeLifecycleStatus::Ready,
    ] {
      db.transition_vpn_node_status(&"node-h".to_string(), status, None)
        .expect("node transition should persist");
    }
    db.update_vpn_node_runtime_details("node-h", Some("42"), Some("203.0.113.10"), Some("k"), Some(51820))
      .expect("runtime details should persist");
    db.create_tunnel("tunnel-h", "node-h", "wg-h", Some("wg-h"))
      .expect("tunnel should insert");

    (db, secure_storage)
  }

  /// The bug this fixes: the dashboard read a stale row and showed "Idle" with a Connect button
  /// while the tunnel service was running.
  #[test]
  fn health_summary_repairs_a_stale_disconnected_row_when_the_service_is_running() {
    let (db, secure_storage) = health_fixture("health_reconcile_up");
    db.update_tunnel_status("tunnel-h", TunnelStatus::Disconnected, None)
      .expect("stale status should persist");

    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::running()));
    let summary = Orchestrator::with_ip_probe(StaticIpProbe::none())
      .get_health_summary(&wireguard, &db, &secure_storage, "node-h", Some("tunnel-h"), false)
      .expect("health summary should succeed");

    assert_eq!(summary.tunnel_status, Some(TunnelStatus::Connected));
    let stored = db.get_tunnel("tunnel-h").expect("query").expect("row");
    assert_eq!(
      stored.status,
      TunnelStatus::Connected,
      "the stale row must be repaired, not just papered over in the response"
    );
  }

  /// An `Error` row carries a `last_error` the user still needs; polling must not erase it.
  #[test]
  fn health_summary_reconciliation_preserves_an_error_status_and_its_last_error() {
    let (db, secure_storage) = health_fixture("health_reconcile_error");
    db.update_tunnel_status("tunnel-h", TunnelStatus::Error, Some("import failed"))
      .expect("error status should persist");

    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::running()));
    Orchestrator::with_ip_probe(StaticIpProbe::none())
      .get_health_summary(&wireguard, &db, &secure_storage, "node-h", Some("tunnel-h"), false)
      .expect("health summary should succeed");

    let stored = db.get_tunnel("tunnel-h").expect("query").expect("row");
    assert_eq!(stored.status, TunnelStatus::Error);
    assert_eq!(stored.last_error.as_deref(), Some("import failed"));
  }

  /// Health polls every few seconds, so an unchanged status must not churn `updated_at`.
  #[test]
  fn health_summary_does_not_touch_the_row_when_the_status_already_matches() {
    let (db, secure_storage) = health_fixture("health_reconcile_noop");
    db.update_tunnel_status("tunnel-h", TunnelStatus::Connected, None)
      .expect("status should persist");
    let before = db.get_tunnel("tunnel-h").expect("query").expect("row").updated_at;

    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::running()));
    Orchestrator::with_ip_probe(StaticIpProbe::none())
      .get_health_summary(&wireguard, &db, &secure_storage, "node-h", Some("tunnel-h"), false)
      .expect("health summary should succeed");

    let after = db.get_tunnel("tunnel-h").expect("query").expect("row").updated_at;
    assert_eq!(before, after, "an unchanged status must not bump updated_at on every poll");
  }

  /// The card shows the user their own address in both states, so the change on connect is the
  /// evidence the tunnel works. That needs a reading from the disconnected side too.
  #[test]
  fn health_summary_measures_the_egress_ip_while_disconnected() {
    let (db, secure_storage) = health_fixture("health_egress_down");

    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::stopped()));
    let summary = Orchestrator::with_ip_probe(StaticIpProbe::v4("198.51.100.7"))
      .get_health_summary(&wireguard, &db, &secure_storage, "node-h", Some("tunnel-h"), false)
      .expect("health summary should succeed");

    assert_eq!(summary.tunnel_status, Some(TunnelStatus::Disconnected));
    assert_eq!(summary.external_ipv4.as_deref(), Some("198.51.100.7"));
    assert_eq!(
      summary.endpoint_ip.as_deref(),
      Some("203.0.113.10"),
      "the server address stays a separate field, so the two can be compared"
    );
  }

  /// Substituting the node's own address made "verified through the tunnel" indistinguishable
  /// from "the probe failed", which let the UI claim a check it had not performed.
  #[test]
  fn health_summary_reports_a_failed_egress_probe_as_absent_not_as_the_server_address() {
    let (db, secure_storage) = health_fixture("health_egress_failed");

    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::running()));
    let summary = Orchestrator::with_ip_probe(StaticIpProbe::none())
      .get_health_summary(&wireguard, &db, &secure_storage, "node-h", Some("tunnel-h"), false)
      .expect("health summary should succeed");

    assert_eq!(summary.tunnel_status, Some(TunnelStatus::Connected));
    assert_eq!(summary.external_ipv4, None);
    assert_ne!(
      summary.external_ipv4, summary.endpoint_ip,
      "a failed probe must never be reported as a verified match against the server"
    );
  }

  /// This deployment's tunnel is IPv4-only, so any IPv6 egress while connected is traffic leaving
  /// outside it. The field has to survive to the UI for that to be visible.
  #[test]
  fn health_summary_reports_ipv6_egress_separately() {
    let (db, secure_storage) = health_fixture("health_egress_v6");

    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::running()));
    let summary = Orchestrator::with_ip_probe(StaticIpProbe::both("203.0.113.10", "2001:db8::1"))
      .get_health_summary(&wireguard, &db, &secure_storage, "node-h", Some("tunnel-h"), false)
      .expect("health summary should succeed");

    assert_eq!(summary.external_ipv4.as_deref(), Some("203.0.113.10"));
    assert_eq!(summary.external_ipv6.as_deref(), Some("2001:db8::1"));
  }

  /// Health polls every few seconds. Probing a third-party service that often would be abusive,
  /// but a connection change must still update the card immediately rather than after the TTL.
  #[test]
  fn egress_probe_is_cached_across_polls_and_reprobed_when_the_connection_changes() {
    struct CountingProbe {
      calls: Mutex<usize>,
    }

    impl ExternalIpProbe for CountingProbe {
      fn probe(&self, family: IpFamily) -> Option<String> {
        if family == IpFamily::V4 {
          let mut calls = self.calls.lock().expect("probe counter should lock");
          *calls += 1;
          return Some(format!("198.51.100.{}", *calls));
        }
        None
      }
    }

    let (db, secure_storage) = health_fixture("health_egress_cache");
    let probe = Arc::new(CountingProbe { calls: Mutex::new(0) });
    let orchestrator = Orchestrator::with_ip_probe(probe.clone());

    let stopped = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::stopped()));
    let first = orchestrator
      .get_health_summary(&stopped, &db, &secure_storage, "node-h", Some("tunnel-h"), false)
      .expect("health summary should succeed");
    let second = orchestrator
      .get_health_summary(&stopped, &db, &secure_storage, "node-h", Some("tunnel-h"), false)
      .expect("health summary should succeed");

    assert_eq!(
      first.external_ipv4, second.external_ipv4,
      "a repeat poll in the same state must be served from cache"
    );
    assert_eq!(*probe.calls.lock().expect("counter"), 1);

    // Connecting changes the answer, so the cache must not be trusted across the transition.
    let running = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::running()));
    let after_connect = orchestrator
      .get_health_summary(&running, &db, &secure_storage, "node-h", Some("tunnel-h"), false)
      .expect("health summary should succeed");

    assert_ne!(first.external_ipv4, after_connect.external_ipv4);
    assert_eq!(*probe.calls.lock().expect("counter"), 2);

    // An explicit user refresh re-probes even though nothing else changed.
    orchestrator
      .get_health_summary(&running, &db, &secure_storage, "node-h", Some("tunnel-h"), true)
      .expect("health summary should succeed");
    assert_eq!(*probe.calls.lock().expect("counter"), 3);
  }

  /// A disconnected tunnel has no session, so it must not report a start time.
  #[test]
  fn health_summary_reports_no_session_start_while_disconnected() {
    let (db, secure_storage) = health_fixture("health_connected_at_down");
    db.set_tunnel_connected_at("tunnel-h", true)
      .expect("stale session start should persist");

    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::stopped()));
    let summary = Orchestrator::with_ip_probe(StaticIpProbe::none())
      .get_health_summary(&wireguard, &db, &secure_storage, "node-h", Some("tunnel-h"), false)
      .expect("health summary should succeed");

    assert_eq!(summary.connected_at, None);
  }

  /// A tunnel found already running has no knowable start time. Stamping "now" would render a
  /// timer counting from an arbitrary moment, which reads as fact.
  #[test]
  fn reconciliation_does_not_invent_a_session_start_for_an_already_running_tunnel() {
    let (db, secure_storage) = health_fixture("health_connected_at_unknown");
    db.update_tunnel_status("tunnel-h", TunnelStatus::Disconnected, None)
      .expect("status should persist");

    let wireguard = WireGuardManager::with_runner(Arc::new(MockWireGuardRunner::running()));
    let summary = Orchestrator::with_ip_probe(StaticIpProbe::none())
      .get_health_summary(&wireguard, &db, &secure_storage, "node-h", Some("tunnel-h"), false)
      .expect("health summary should succeed");

    assert_eq!(summary.tunnel_status, Some(TunnelStatus::Connected));
    assert_eq!(
      summary.connected_at, None,
      "an unknown start time must stay unknown rather than being backfilled"
    );
  }

  /// Full backend path against a real DigitalOcean droplet.
  ///
  /// **This creates a billable resource.** It is destroyed on every exit path, including panics,
  /// and the assertions run only after teardown so a failure cannot strand a droplet. Requires
  /// `GOATSTARE_DO_TEST_TOKEN`.
  ///
  /// Proves the production path end to end: keys generated locally, baked into cloud-init, and a
  /// droplet reaching ready with an endpoint the client can dial — with nothing ever read back
  /// off the server.
  #[test]
  #[ignore = "Creates a real, billable DigitalOcean droplet; needs GOATSTARE_DO_TEST_TOKEN"]
  fn real_digitalocean_full_backend_path_produces_a_connect_config() {
    use crate::backend::provider::DigitalOceanProviderAdapter;
    use crate::backend::wireguard::{build_client_config, ClientConfigParams};

    let token = match std::env::var("GOATSTARE_DO_TEST_TOKEN") {
      Ok(value) if !value.trim().is_empty() => value,
      _ => {
        eprintln!("GOATSTARE_DO_TEST_TOKEN is not set; skipping.");
        return;
      }
    };

    let suffix = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_secs())
      .unwrap_or(0);

    let db = Database::new(test_db_path(&format!("real_do_{suffix}")));
    db.run_migrations().expect("migrations should succeed");

    let secure_storage = SecureStorage::dev_mock();
    // Migration 0007 seeds this provider row.
    const DO_PROVIDER_ID: &str = "provider-1";
    let token_ref = secure_storage
      .store_digitalocean_api_token(DO_PROVIDER_ID, &token)
      .expect("token should store");
    db.update_provider_token_ref(DO_PROVIDER_ID, Some(&token_ref))
      .expect("token ref should persist");

    let providers = ProviderRegistry::from_adapter(Arc::new(
      DigitalOceanProviderAdapter::new().expect("adapter should construct"),
    ));
    let orchestrator = Orchestrator::new();
    let bootstrap = Bootstrap::new();
    let wireguard = WireGuardManager::new();

    let node_id = format!("node-do-{suffix}");
    let created = orchestrator.create_vpn_node_to_ready(
      &providers,
      &bootstrap,
      &db,
      &secure_storage,
      &CreateVpnNodeRequest {
        node_id: node_id.clone(),
        provider_id: DO_PROVIDER_ID.to_string(),
        node_name: format!("goatstare-e2e-{suffix}"),
        region: "nyc1".to_string(),
        plan_key: "s-1vcpu-1gb".to_string(),
      },
    );

    // Gather everything needed for assertions while the node still exists, then tear down before
    // asserting anything. A panic between provisioning and teardown would leave a paid droplet
    // running.
    let findings = created.as_ref().ok().map(|created| {
      let node = db
        .get_vpn_node(&node_id)
        .ok()
        .flatten()
        .expect("node row should exist");

      let client_config = node
        .client_private_key_ref
        .as_deref()
        .and_then(|key_ref| secure_storage.get_client_private_key(key_ref).ok().flatten())
        .and_then(|private_key| {
          build_client_config(&ClientConfigParams {
            private_key,
            client_address_cidr: DEFAULT_CLIENT_CIDR.to_string(),
            dns_server: None,
            server_public_key: created.server_public_key.clone(),
            server_endpoint_ip: node.public_ipv4.clone().unwrap_or_default(),
            server_listen_port: node.endpoint_port.unwrap_or(DEFAULT_WIREGUARD_PORT),
            allowed_ips: vec!["0.0.0.0/0".to_string(), "::/0".to_string()],
            persistent_keepalive_seconds: DEFAULT_CLIENT_KEEPALIVE_SECONDS,
          })
          .ok()
        });

      (node, created.server_public_key.clone(), client_config)
    });

    // Teardown first, unconditionally.
    let cleanup = orchestrator.force_cleanup_vpn_node(
      &providers,
      &wireguard,
      &db,
      &secure_storage,
      &node_id,
    );

    let created = created.unwrap_or_else(|error| panic!("node creation should reach Ready: {error:?}"));
    let (node, server_public_key, client_config) =
      findings.expect("findings should exist when creation succeeded");

    let cleanup = cleanup.unwrap_or_else(|error| panic!("cleanup must succeed: {error:?}"));
    assert!(
      cleanup.remote_droplet_delete_attempted && cleanup.remote_droplet_delete_succeeded,
      "the droplet must be destroyed, or it keeps billing: {cleanup:?}"
    );

    assert_eq!(node.lifecycle_status, NodeLifecycleStatus::Ready);
    assert!(!created.server_public_key.trim().is_empty());

    // A real droplet endpoint is publicly routable, so a full tunnel is preserved.
    let endpoint = node.public_ipv4.clone().expect("a public IPv4 should be recorded");
    assert!(
      !endpoint_is_locally_scoped(&endpoint),
      "a DigitalOcean endpoint should be publicly routable, got {endpoint}"
    );
    assert_eq!(node.endpoint_port, Some(DEFAULT_WIREGUARD_PORT));

    let config = client_config.expect("a client config should render");
    assert!(config.contains("[Peer]"));
    assert!(config.contains(&format!("PublicKey = {server_public_key}")));
    assert!(config.contains(&format!("Endpoint = {endpoint}:{DEFAULT_WIREGUARD_PORT}")));
    assert!(
      config.contains("AllowedIPs = 0.0.0.0/0"),
      "full tunnel must survive for a public endpoint: {config}"
    );

    let path = std::env::temp_dir().join(format!("goatstare-do-{suffix}.conf"));
    std::fs::write(&path, config.replace("\r\n", "\n")).expect("config should be writable");
    println!("\n=== DigitalOcean client config: {} ===", path.display());
    println!("=== endpoint {endpoint}:{DEFAULT_WIREGUARD_PORT} ===");
    println!("=== droplet destroyed; this config is kept only as a record ===\n");
  }

  /// The production path, proven by an actual handshake against a real droplet.
  ///
  /// **Creates a billable resource**, destroyed on every exit path. Needs
  /// `GOATSTARE_DO_TEST_TOKEN` and a Docker daemon.
  ///
  /// The previous test stops at "the provider says ready", which is all the app can observe — the
  /// droplet reports active well before cloud-init finishes. That leaves the important question
  /// unanswered: did the baked-in payload actually bring WireGuard up? This answers it by
  /// connecting as a real client.
  ///
  /// The client runs in a container, so no Windows elevation is involved. That container is the
  /// *client*; nothing execs into the provisioned droplet, and no key material is read back from
  /// it — the config is built entirely from values generated locally.
  #[test]
  #[ignore = "Creates a real, billable DigitalOcean droplet; needs GOATSTARE_DO_TEST_TOKEN and Docker"]
  fn real_digitalocean_server_completes_a_wireguard_handshake() {
    use crate::backend::provider::DigitalOceanProviderAdapter;
    use crate::backend::wireguard::{build_client_config, ClientConfigParams};
    use std::process::Command;

    let token = match std::env::var("GOATSTARE_DO_TEST_TOKEN") {
      Ok(value) if !value.trim().is_empty() => value,
      _ => {
        eprintln!("GOATSTARE_DO_TEST_TOKEN is not set; skipping.");
        return;
      }
    };

    let suffix = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_secs())
      .unwrap_or(0);
    let client_container = format!("goatstare-do-client-{suffix}");

    let db = Database::new(test_db_path(&format!("real_do_hs_{suffix}")));
    db.run_migrations().expect("migrations should succeed");

    let secure_storage = SecureStorage::dev_mock();
    const DO_PROVIDER_ID: &str = "provider-1";
    let token_ref = secure_storage
      .store_digitalocean_api_token(DO_PROVIDER_ID, &token)
      .expect("token should store");
    db.update_provider_token_ref(DO_PROVIDER_ID, Some(&token_ref))
      .expect("token ref should persist");

    let providers = ProviderRegistry::from_adapter(Arc::new(
      DigitalOceanProviderAdapter::new().expect("adapter should construct"),
    ));
    let orchestrator = Orchestrator::new();
    let wireguard = WireGuardManager::new();

    let node_id = format!("node-do-hs-{suffix}");
    let created = orchestrator.create_vpn_node_to_ready(
      &providers,
      &Bootstrap::new(),
      &db,
      &secure_storage,
      &CreateVpnNodeRequest {
        node_id: node_id.clone(),
        provider_id: DO_PROVIDER_ID.to_string(),
        node_name: format!("goatstare-hs-{suffix}"),
        region: "nyc1".to_string(),
        plan_key: "s-1vcpu-1gb".to_string(),
      },
    );

    // Everything below runs inside a closure so the droplet and container are torn down on every
    // path, including a panic, before any assertion runs.
    let outcome: Result<(String, String), String> = (|| {
      let created = created.as_ref().map_err(|error| format!("create failed: {error:?}"))?;

      let node = db
        .get_vpn_node(&node_id)
        .map_err(|error| format!("node lookup failed: {error:?}"))?
        .ok_or_else(|| "node row should exist".to_string())?;

      let endpoint = node
        .public_ipv4
        .clone()
        .ok_or_else(|| "a public IPv4 should be recorded".to_string())?;

      let private_key = node
        .client_private_key_ref
        .as_deref()
        .ok_or_else(|| "client key reference should be recorded".to_string())
        .and_then(|key_ref| {
          secure_storage
            .get_client_private_key(key_ref)
            .map_err(|error| format!("key lookup failed: {error:?}"))?
            .ok_or_else(|| "client private key should be readable".to_string())
        })?;

      // Route only the tunnel subnet: this client is verifying a handshake, not taking over the
      // container's default route.
      let config = build_client_config(&ClientConfigParams {
        private_key,
        client_address_cidr: DEFAULT_CLIENT_CIDR.to_string(),
        dns_server: None,
        server_public_key: created.server_public_key.clone(),
        server_endpoint_ip: endpoint.clone(),
        server_listen_port: node.endpoint_port.unwrap_or(DEFAULT_WIREGUARD_PORT),
        allowed_ips: vec!["10.66.0.0/24".to_string()],
        persistent_keepalive_seconds: DEFAULT_CLIENT_KEEPALIVE_SECONDS,
      })
      .map_err(|error| format!("client config should render: {error:?}"))?;

      // Bring the client up and leave it retrying. WireGuard re-sends handshake initiations every
      // few seconds, so the client simply waits out cloud-init rather than needing to detect it.
      let run = Command::new("docker")
        .args([
          "run", "-d", "--name", &client_container,
          "--cap-add", "NET_ADMIN",
          "-e", &format!("WG_CONF={}", config.replace("\r\n", "\n")),
          "--entrypoint", "/bin/sh",
          "alpine:latest",
          "-c",
          "apk add --no-cache wireguard-tools iproute2 >/dev/null 2>&1; \
           mkdir -p /etc/wireguard; \
           printf '%s' \"$WG_CONF\" > /etc/wireguard/wg0.conf; \
           wg-quick up wg0 >/tmp/up.log 2>&1; \
           exec tail -f /dev/null",
        ])
        .output()
        .map_err(|error| format!("could not start the client container: {error}"))?;

      if !run.status.success() {
        return Err(format!(
          "client container failed to start: {}",
          String::from_utf8_lossy(&run.stderr).trim()
        ));
      }

      // Cloud-init installs WireGuard on a fresh droplet, so allow generous time.
      //
      // The probe keys off the *exit status*, never stdout. `docker exec` output does not reach
      // the caller when the daemon is reached over TCP, which is the case when it runs inside
      // WSL2 — an earlier version read stdout here and so reported "no handshake" for every run,
      // regardless of what the server was doing.
      let deadline = std::time::Instant::now() + Duration::from_secs(300);

      while std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(10));

        let handshook = Command::new("docker")
          .args([
            "exec", &client_container, "sh", "-c",
            "test \"$(wg show wg0 latest-handshakes 2>/dev/null | awk '{print $2}' | head -1)\" -gt 0",
          ])
          .status()
          .map(|status| status.success())
          .unwrap_or(false);

        if handshook {
          return Ok((endpoint, "confirmed".to_string()));
        }
      }

      // Preserve the client's own view for diagnosis, again without relying on exec stdout.
      let _ = Command::new("docker")
        .args([
          "exec", &client_container, "sh", "-c",
          "{ echo '--- wg-quick ---'; cat /tmp/up.log; echo '--- wg show ---'; wg show; } > /tmp/diag.txt 2>&1",
        ])
        .status();
      let _ = Command::new("docker")
        .args(["cp", &format!("{client_container}:/tmp/diag.txt"), "./do-handshake-diag.txt"])
        .status();

      Err(format!(
        "no handshake completed within 5 minutes against {endpoint}; client state written to do-handshake-diag.txt"
      ))
    })();

    // Teardown, unconditionally and before any assertion.
    let _ = Command::new("docker").args(["rm", "-f", &client_container]).output();
    let cleanup = orchestrator.force_cleanup_vpn_node(
      &providers,
      &wireguard,
      &db,
      &secure_storage,
      &node_id,
    );

    let cleanup = cleanup.unwrap_or_else(|error| panic!("cleanup must succeed: {error:?}"));
    assert!(
      cleanup.remote_droplet_delete_attempted && cleanup.remote_droplet_delete_succeeded,
      "the droplet must be destroyed, or it keeps billing: {cleanup:?}"
    );

    let (endpoint, handshake_stamp) = outcome.unwrap_or_else(|failure| panic!("{failure}"));

    println!("\n=== DigitalOcean handshake confirmed ===");
    println!("=== endpoint {endpoint}:{DEFAULT_WIREGUARD_PORT} latest-handshake={handshake_stamp} ===");
    println!("=== droplet destroyed ===\n");
  }

  /// Full backend path against a real container, stopping short of touching the local machine.
  ///
  /// Covers what the adapter-level tests cannot: provisioning through the orchestrator, the
  /// client keypair persisting across node creation and tunnel creation, and the connect config
  /// the WireGuard layer would be handed. Everything here is server-side or in-memory — no tunnel
  /// service is installed and no local interface is created.
  #[test]
  #[ignore = "Requires local Docker daemon; provisions a real container"]
  fn real_docker_full_backend_path_produces_a_connect_config() {
    use crate::backend::provider::docker::DockerProviderAdapter;

    let suffix = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|value| value.as_nanos())
      .unwrap_or(0);

    let db = Database::new(test_db_path(&format!("real_docker_backend_{suffix}")));
    db.run_migrations().expect("migrations should succeed");

    let secure_storage = SecureStorage::dev_mock();
    // Migration 0007 already seeds this provider row.
    const DOCKER_PROVIDER_ID: &str = "provider-docker";
    let token_ref = secure_storage
      .store_digitalocean_api_token(DOCKER_PROVIDER_ID, "dkr_v1_local_dev_mock_token_2026")
      .expect("faux token should store");
    db.update_provider_token_ref(DOCKER_PROVIDER_ID, Some(&token_ref))
      .expect("token ref should persist");

    let providers = ProviderRegistry::from_adapter(Arc::new(DockerProviderAdapter::new()));
    let orchestrator = Orchestrator::new();
    let bootstrap = Bootstrap::new();
    let wireguard = WireGuardManager::new();

    let node_id = format!("node-e2e-{suffix}");
    let created = orchestrator.create_vpn_node_to_ready(
      &providers,
      &bootstrap,
      &db,
      &secure_storage,
      &CreateVpnNodeRequest {
        node_id: node_id.clone(),
        provider_id: DOCKER_PROVIDER_ID.to_string(),
        node_name: format!("e2e-{suffix}"),
        region: "docker-local".to_string(),
        plan_key: "docker-small".to_string(),
      },
    );

    // Always tear the container down, whatever the assertions do.
    let cleanup = || {
      let _ = orchestrator.force_cleanup_vpn_node(
        &providers,
        &wireguard,
        &db,
        &secure_storage,
        &node_id,
      );
    };

    let created = match created {
      Ok(created) => created,
      Err(error) => {
        cleanup();
        panic!("node creation should reach Ready: {error:?}");
      }
    };

    let outcome = (|| -> Result<String, String> {
      let node = db
        .get_vpn_node(&node_id)
        .map_err(|error| format!("node lookup failed: {error:?}"))?
        .ok_or_else(|| "node row should exist".to_string())?;

      // The client key was generated at provision time and recorded against the node.
      let client_public_key = node
        .client_public_key
        .clone()
        .ok_or_else(|| "node should record the client public key".to_string())?;
      if node.client_private_key_ref.is_none() {
        return Err("node should record the client private key reference".to_string());
      }

      // Docker publishes an ephemeral host port, so the endpoint must not be the container's own.
      let endpoint_port = node
        .endpoint_port
        .ok_or_else(|| "node should record the endpoint port".to_string())?;
      if endpoint_port == DEFAULT_WIREGUARD_PORT {
        return Err(format!(
          "expected an ephemeral published port, got the container port {endpoint_port}"
        ));
      }

      let tunnel_id = format!("tunnel-e2e-{suffix}");
      orchestrator
        .create_and_activate_client_tunnel(
          &wireguard,
          &db,
          &secure_storage,
          &CreateClientTunnelRequest {
            tunnel_id: tunnel_id.clone(),
            node_id: node_id.clone(),
            tunnel_name: format!("wg-e2e-{suffix}"),
            client_address_cidr: DEFAULT_CLIENT_CIDR.to_string(),
            dns_server: None,
            // Ask for a full tunnel, as the dashboard does.
            allowed_ips: vec!["0.0.0.0/0".to_string(), "::/0".to_string()],
          },
        )
        .map_err(|error| format!("tunnel creation should succeed: {error:?}"))?;

      let tunnel = db
        .get_tunnel(&tunnel_id)
        .map_err(|error| format!("tunnel lookup failed: {error:?}"))?
        .ok_or_else(|| "tunnel row should exist".to_string())?;

      // Tunnel creation must reuse the provisioned key, not mint a new one the server has never
      // heard of.
      let derived = wireguard
        .derive_public_key(
          &secure_storage
            .get_client_private_key(
              tunnel
                .client_private_key_ref
                .as_deref()
                .ok_or_else(|| "tunnel should carry a key reference".to_string())?,
            )
            .map_err(|error| format!("key lookup failed: {error:?}"))?
            .ok_or_else(|| "client private key should be readable".to_string())?,
        )
        .map_err(|error| format!("public key derivation failed: {error:?}"))?;

      if derived != client_public_key {
        return Err("tunnel key must match the peer baked into the server".to_string());
      }

      load_connect_config_for_tunnel(&secure_storage, "docker", &node, &tunnel, &TunnelSettings::default())
        .map_err(|error| format!("connect config should render: {error:?}"))
    })();

    cleanup();

    let config = outcome.unwrap_or_else(|failure| panic!("{failure}"));

    assert!(config.contains("[Interface]"));
    assert!(config.contains("[Peer]"));
    assert!(
      config.contains(&format!("PublicKey = {}", created.server_public_key)),
      "config must target the provisioned server key"
    );
    // The loopback guard must have narrowed the requested default route.
    assert!(
      config.contains("AllowedIPs = 10.66.0.0/24"),
      "a default route to a loopback endpoint must be scoped to the tunnel subnet: {config}"
    );
    assert!(!config.contains("0.0.0.0/0"), "full tunnel must not survive: {config}");
  }
}

fn select_plan_pricing<'a>(
  pricing: &'a [ProviderPlanPricing],
  plan_key: &str,
) -> Option<&'a ProviderPlanPricing> {
  pricing.iter().find(|entry| entry.plan_key == plan_key)
}

impl OrchestratorError {
  pub fn to_user_facing(&self) -> UserFacingError {
    match self {
      Self::InvalidState { detail, .. } => UserFacingError {
        kind: UserFacingErrorKind::InvalidState,
        message: detail.clone(),
        next_action:
          "Use Force Cleanup on the affected node, refresh the dashboard, then retry.".to_string(),
      },
      Self::InvalidInput { detail } => UserFacingError {
        kind: UserFacingErrorKind::InvalidInput,
        message: detail.clone(),
        next_action: "Correct the value and try again.".to_string(),
      },
      Self::Database { detail, source } => {
        let kind = if matches!(source, DbError::NotFound { .. }) {
          UserFacingErrorKind::MissingResource
        } else {
          UserFacingErrorKind::Persistence
        };

        UserFacingError {
          kind,
          message: detail.clone(),
          next_action:
            "Refresh the dashboard; if the local state still looks stale, use Force Cleanup.".to_string(),
        }
      }
      // The provider's own complaint is the only part a user can act on — "invalid size for
      // region", "name must be a valid hostname". Showing just the wrapper left them with a
      // failure and no cause.
      Self::Provider { detail, source } => UserFacingError {
        kind: UserFacingErrorKind::Provider,
        message: format!("{detail} {}", logging::describe_error_chain(source)),
        next_action:
          "Check the reported cause above. If provisioning was interrupted, use Force Cleanup on the node, then retry."
            .to_string(),
      },
      Self::WireGuard { detail, source } => UserFacingError {
        kind: UserFacingErrorKind::WireGuard,
        message: format!("{detail}: {source}"),
        next_action: {
          let lowered_source = source.to_string().to_ascii_lowercase();
          let lowered_detail = detail.to_ascii_lowercase();
          let tunnel_name = extract_tunnel_name_from_wireguard_error(&source.to_string());
          let tunnel_repair_commands = tunnel_name
            .as_deref()
            .map(|name| {
              format!(
                "PowerShell recovery: sc.exe query \"WireGuardTunnel${name}\"; wireguard.exe /uninstalltunnelservice {name}; then click Connect again in Nodes."
              )
            })
            .unwrap_or_else(|| {
              "PowerShell recovery: sc.exe query WireGuardManager; then click Connect again in Nodes."
                .to_string()
            });

          if lowered_source.contains("access is denied")
            || lowered_source.contains("requires elevation")
            || lowered_source.contains("administrator")
          {
            format!(
              "Approve the Windows UAC prompt (or run GoatStare as Administrator), then retry Connect. {tunnel_repair_commands}"
            )
          } else if lowered_detail.contains("did not reach running")
            || lowered_detail.contains("did not report running")
            || lowered_source.contains("timely fashion")
            || lowered_source.contains("start pending")
          {
            format!(
              "The tunnel service start timed out. Wait 5-10 seconds and retry Connect once. If it fails again: {tunnel_repair_commands} If that still fails, use Nodes -> Uninstall Host to destroy the node and re-provision."
            )
          } else if lowered_source.contains("does not exist")
            || lowered_detail.contains("not found locally")
            || lowered_detail.contains("re-import the tunnel configuration")
          {
            format!(
              "The local tunnel service is missing. {tunnel_repair_commands}"
            )
          } else {
            format!(
              "Repair WireGuard for Windows and retry Connect. If the issue persists: {tunnel_repair_commands}"
            )
          }
        },
      },
      Self::SecretStorage { detail, .. } => UserFacingError {
        kind: UserFacingErrorKind::Persistence,
        message: detail.clone(),
        next_action: "Open Settings, re-save provider credentials, then retry Connect."
          .to_string(),
      },
      Self::Bootstrap { detail, .. } => UserFacingError {
        kind: UserFacingErrorKind::Unknown,
        message: detail.clone(),
        next_action:
          "Retry node creation after checking bootstrap settings, or Force Cleanup first if the node already exists."
            .to_string(),
      },
      Self::Keygen { detail, .. } => UserFacingError {
        kind: UserFacingErrorKind::Unknown,
        message: detail.clone(),
        next_action:
          "Ensure WireGuard is installed and accessible, then retry node creation."
            .to_string(),
      },
    }
  }
}

fn extract_tunnel_name_from_wireguard_error(source_text: &str) -> Option<String> {
  let marker = "WireGuardTunnel$";
  let marker_index = source_text.find(marker)?;
  let remainder = &source_text[(marker_index + marker.len())..];
  let tunnel_name: String = remainder
    .chars()
    .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
    .collect();

  if tunnel_name.is_empty() {
    None
  } else {
    Some(tunnel_name)
  }
}

/// Which address family an egress probe should report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpFamily {
  V4,
  V6,
}

impl IpFamily {
  /// Hosts that resolve on this family only, which is what forces the request onto it.
  fn endpoints(self) -> [&'static str; 2] {
    match self {
      Self::V4 => ["https://api.ipify.org", "https://ipv4.icanhazip.com"],
      Self::V6 => ["https://api6.ipify.org", "https://ipv6.icanhazip.com"],
    }
  }

  fn matches(self, address: &IpAddr) -> bool {
    match self {
      Self::V4 => address.is_ipv4(),
      Self::V6 => address.is_ipv6(),
    }
  }
}

/// Reads the address the internet currently sees for this machine.
///
/// Injectable so the normal test suite never makes a network call — the health path probes on any
/// connected tunnel, so a hardcoded implementation would have put live HTTP requests in unit tests.
pub trait ExternalIpProbe: Send + Sync {
  fn probe(&self, family: IpFamily) -> Option<String>;
}

pub struct HttpExternalIpProbe;

impl ExternalIpProbe for HttpExternalIpProbe {
  fn probe(&self, family: IpFamily) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
      .timeout(EXTERNAL_IP_PROBE_TIMEOUT)
      .build()
      .ok()?;

    for endpoint in family.endpoints() {
      let response = match client.get(endpoint).send() {
        Ok(value) => value,
        Err(_) => continue,
      };
      if !response.status().is_success() {
        continue;
      }

      let body = match response.text() {
        Ok(value) => value,
        Err(_) => continue,
      };
      let candidate = body.lines().next().unwrap_or_default().trim();
      if candidate.is_empty() {
        continue;
      }

      // A v6 endpoint answering with a v4 address (or the reverse) means something rewrote the
      // request; reporting it under the wrong family would misdescribe the route.
      if let Ok(parsed) = candidate.parse::<IpAddr>() {
        if family.matches(&parsed) {
          return Some(parsed.to_string());
        }
      }
    }

    None
  }
}

fn canonical_tunnel_name_for_node(node_name: &str) -> String {
  let sanitized = node_name
    .trim()
    .chars()
    .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '-' })
    .collect::<String>();

  let collapsed = sanitized
    .split('-')
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("-");

  let base = if collapsed.is_empty() {
    DEFAULT_TUNNEL_BASENAME.to_string()
  } else {
    collapsed
  };

  let mut name = format!("{base}-wg");
  if name.len() > 63 {
    name.truncate(63);
    name = name.trim_end_matches('-').to_string();
  }

  if name.is_empty() {
    "primary-wg".to_string()
  } else {
    name
  }
}

