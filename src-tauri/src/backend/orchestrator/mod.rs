use thiserror::Error;
use uuid::Uuid;
use serde::Serialize;
use std::net::IpAddr;
use std::time::Duration;

use crate::backend::bootstrap::{
  Bootstrap, BootstrapError, BootstrapParams, DEFAULT_CLIENT_CIDR, DEFAULT_INTERFACE_NAME,
  DEFAULT_SERVER_INTERFACE_CIDR, DEFAULT_WIREGUARD_PORT,
};
use crate::backend::crypto::keypair::{generate_client_keypair, generate_server_keypair, KeygenError};
use crate::backend::db::Database;
use crate::backend::db::{DbError, NodeLifecycleStatus, TunnelStatus};
use crate::backend::logging;
use crate::backend::provider::{
  BootstrapWaitRequest, NodeInitRequest, ProvisionedDroplet, ProviderDropletStatus, ProviderFlowError,
  ProviderPlanPricing, ProviderRegistry, DEFAULT_REGION, DEFAULT_SIZE,
};
use crate::backend::secure_storage::SecureStorage;
use crate::backend::secure_storage::SecretStoreError;
use crate::backend::wireguard::{
  ClientConfigParams, DEFAULT_CLIENT_ALLOWED_IPS, DEFAULT_CLIENT_KEEPALIVE_SECONDS,
  TunnelRuntimeStatus, WireGuardError,
  WireGuardManager,
};

#[derive(Clone)]
pub struct Orchestrator;

const DEFAULT_TUNNEL_BASENAME: &str = "primary";

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
  MissingTunnelArtifact,
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
  pub external_ipv4: Option<String>,
  pub tunnel_status: Option<TunnelStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsBundle {
  pub generated_at: String,
  pub providers: Vec<crate::backend::db::ProviderRecord>,
  pub nodes: Vec<crate::backend::db::VpnNodeRecord>,
  pub tunnels: Vec<crate::backend::db::TunnelRecord>,
  pub init_runs: Vec<crate::backend::db::InitRunLogRecord>,
  pub redacted_notes: Vec<String>,
  pub wireguard_debug: crate::backend::wireguard::WireGuardDebugSnapshot,
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
    Self
  }

  pub fn status(&self) -> &'static str {
    "idle"
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
    Ok(nodes)
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
      endpoint_ip: "0.0.0.0".to_string(),
      readiness_marker_path: artifact.readiness_marker_path,
      server_keypair,
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
    let _ = self.force_cleanup_vpn_node(providers, wireguard, db, secure_storage, node_id);
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
    let correlation_id = Uuid::new_v4().to_string();
    log::warn!(
      "event=vpn_node.force_cleanup.start correlation_id={} node_id={}",
      correlation_id,
      node_id
    );

    let node = db.get_vpn_node(node_id).map_err(|source| OrchestratorError::Database {
      detail: "Failed loading vpn node before force cleanup flow.".to_string(),
      source,
    })?;

    let tunnels = db.list_tunnels_for_node(node_id).map_err(|source| OrchestratorError::Database {
      detail: "Failed listing tunnels before force cleanup flow.".to_string(),
      source,
    })?;

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

    log::warn!(
      "event=vpn_node.force_cleanup.completed correlation_id={} node_id={} local_tunnels_removed={} remote_droplet_delete_attempted={} remote_droplet_delete_succeeded={} node_record_removed={}",
      correlation_id,
      node_id,
      local_tunnels_removed,
      remote_droplet_delete_attempted,
      remote_droplet_delete_succeeded,
      node_record_removed
    );

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
    let init_kind = if provider_slug == "docker" {
      "docker-init"
    } else {
      "cloud-init"
    };
    let record_init = |stage: &str, outcome: &str, output: String| {
      let _ = db.append_init_run_log(
        &request.node_id,
        &provider_slug,
        init_kind,
        stage,
        outcome,
        &logging::redact_sensitive_text(&output),
      );
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
        endpoint_ip: "0.0.0.0".to_string(),
        readiness_marker_path: bootstrap_artifact.readiness_marker_path.clone(),
        server_keypair: server_keypair.clone(),
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
            readiness_marker_path: bootstrap_artifact.readiness_marker_path.clone(),
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
      let mut final_reason = format!("create_vpn_node_to_ready failed: {error}");

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

    let keypair = crate::backend::wireguard::ClientKeyPair {
      private_key,
      public_key,
    };

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
      client_public_key: keypair.public_key,
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
    self.connect_tunnel_inner(wireguard, db, secure_storage, request)
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

    let connect_config =
      load_connect_config_for_tunnel(secure_storage, &provider.slug, &node, &tunnel)?;

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
      let _ = db.transition_vpn_node_status(&request.node_id, NodeLifecycleStatus::Connected, None);
      return Ok(());
    }

    let _ = wireguard.unregister_tunnel(&canonical_tunnel_name);
    let _ = db.update_tunnel_status(&request.tunnel_id, TunnelStatus::Ready, Some("Tunnel runtime did not report Running; service was removed."));

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

    wireguard
      .deactivate_tunnel(&tunnel.name)
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not deactivate WireGuard tunnel service in disconnect flow.".to_string(),
        source,
      })?;

    wireguard
      .unregister_tunnel(&tunnel.name)
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not unregister WireGuard tunnel service in disconnect flow.".to_string(),
        source,
      })?;

    let _ = db.update_tunnel_stats_snapshot(&request.tunnel_id, None, None, None);

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
      let _ = wireguard.deactivate_tunnel(&tunnel.name);
      let _ = wireguard.unregister_tunnel(&tunnel.name);
      let _ = db.update_tunnel_stats_snapshot(&tunnel.id, None, None, None);
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
          if let Ok(config) = load_connect_config_for_tunnel(secure_storage, &provider.slug, &node, tunnel) {
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

    let runtime = if let Some(tunnel) = selected_tunnel.as_ref() {      Some(
        wireguard
          .read_tunnel_status(&tunnel.name)
          .map_err(|source| OrchestratorError::WireGuard {
            detail: "Failed reading tunnel runtime for health summary.".to_string(),
            source,
          })?,
      )
    } else {
      None
    };

    let runtime_status = runtime.as_ref().map(|value| match value.state {
      crate::backend::wireguard::TunnelServiceState::Running => TunnelStatus::Connected,
      crate::backend::wireguard::TunnelServiceState::Stopped => TunnelStatus::Disconnected,
      crate::backend::wireguard::TunnelServiceState::Unknown => TunnelStatus::Error,
    });

    let external_ipv4 = if matches!(runtime_status, Some(TunnelStatus::Connected)) {
      probe_external_ipv4().or_else(|| node.public_ipv4.clone())
    } else {
      node.public_ipv4.clone()
    };

    Ok(HealthSummary {
      node_id: node.id,
      tunnel_id: selected_tunnel.as_ref().map(|tunnel| tunnel.id.clone()),
      endpoint_ip: node.public_ipv4.clone(),
      handshake_age_seconds: runtime.as_ref().and_then(|value| value.last_handshake_age_seconds),
      rx_bytes: runtime.as_ref().and_then(|value| value.rx_bytes),
      tx_bytes: runtime.as_ref().and_then(|value| value.tx_bytes),
      external_ipv4,
      tunnel_status: runtime_status.or_else(|| selected_tunnel.map(|tunnel| tunnel.status)),
    })
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

    let init_runs = db.list_init_run_logs(200).map_err(|source| OrchestratorError::Database {
      detail: "Failed loading init run logs for diagnostics bundle.".to_string(),
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

    let init_runs = init_runs
      .into_iter()
      .map(|mut init_run| {
        init_run.output = logging::redact_sensitive_text(&init_run.output);
        init_run
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
    for init_run in &init_runs {
      if init_run.outcome.eq_ignore_ascii_case("error") {
        redacted_notes.push(format!(
          "init:{}:{} stage={} output={}",
          init_run.node_id,
          init_run.init_kind,
          init_run.stage,
          logging::redact_sensitive_text(&init_run.output)
        ));
      }
    }

    Ok(DiagnosticsBundle {
      generated_at: chrono_like_timestamp(),
      providers,
      nodes,
      tunnels,
      init_runs,
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

    Ok(report)
  }
}

fn redact_text(value: &str) -> String {
  let mut output = value.to_string();
  if output.len() > 96 {
    output.truncate(96);
    output.push_str("...");
  }
  output
}

fn chrono_like_timestamp() -> String {
  use std::time::{SystemTime, UNIX_EPOCH};
  let millis = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|duration| duration.as_millis())
    .unwrap_or_default();
  format!("{millis}")
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

/// Whether a resolved endpoint lives on this machine.
///
/// Judged from the endpoint itself rather than the provider slug, so the orchestrator stays
/// provider-neutral: any adapter that reports a loopback endpoint gets the same treatment.
fn endpoint_is_loopback(endpoint: &str) -> bool {
  endpoint
    .trim()
    .parse::<std::net::IpAddr>()
    .map(|address| address.is_loopback())
    .unwrap_or(false)
}

/// Narrow a default route to the tunnel subnet when the server is on this machine.
///
/// A full-tunnel route (`0.0.0.0/0`) to a loopback endpoint sends the host's entire default route
/// into a server reachable only *through* that same host. For the local Docker dev provider that
/// means all traffic depends on the container's onward NAT, and a failure there takes the machine
/// off the network — with the app that caused it now unreachable over that network too.
///
/// A real provider endpoint is untouched, so full-tunnel remains the behaviour that matters in
/// production.
fn scope_allowed_ips_for_endpoint(
  allowed_ips: Vec<String>,
  endpoint: &str,
  client_address_cidr: &str,
) -> Vec<String> {
  if !endpoint_is_loopback(endpoint) {
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
    "event=connect.allowed_ips.scoped endpoint_is_loopback=true scoped_to={} reason=default_route_to_local_endpoint",
    subnet
  );

  vec![subnet]
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
    dns_server: tunnel.dns_server.clone(),
    server_public_key,
    server_endpoint_ip: endpoint_ip,
    // Reported by the provider adapter at provision time. Falls back to the default only for
    // nodes created before per-node ports existed.
    server_listen_port: node.endpoint_port.unwrap_or(DEFAULT_WIREGUARD_PORT),
    allowed_ips,
    persistent_keepalive_seconds: DEFAULT_CLIENT_KEEPALIVE_SECONDS,
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

    fn default_regions(&self) -> &'static [&'static str] {
      &["nyc1"]
    }

    fn validate_token(&self, _token: &str) -> Result<Account, ProviderAdapterError> {
      Ok(Account {
        uuid: "acct-1".to_string(),
        email: "vpn@example.com".to_string(),
        status: "active".to_string(),
      })
    }

    fn list_regions(&self, _token: &str) -> Result<Vec<String>, ProviderAdapterError> {
      Ok(vec!["nyc1".to_string(), "sfo3".to_string()])
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
          stderr: String::new(),
        });
      }

      if program_name == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "wg" && args == ["genkey"] {
        return Ok(CommandOutput {
          stdout: "client-private-key".to_string(),
          stderr: String::new(),
        });
      }

      if program_name == "wg" && args == ["pubkey"] {
        return Ok(CommandOutput {
          stdout: "client-public-key".to_string(),
          stderr: String::new(),
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
              stderr: String::new(),
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
              stderr: String::new(),
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
              stderr: String::new(),
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
          stderr: String::new(),
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
          stderr: String::new(),
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
          stderr: String::new(),
        });
      }

      if program_name == "wireguard" && args == ["/uninstalltunnelservice", "wg-node-partial"] {
        let mut state = self.state.lock().expect("lock must succeed");
        state.unregister_calls += 1;
        state.service_exists = false;
        state.service_running = false;
        return Ok(CommandOutput {
          stdout: "uninstalled".to_string(),
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

  #[test]
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
    assert_eq!(wireguard_state.deactivate_calls, 1);
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
      Self::Provider { detail, .. } => UserFacingError {
        kind: UserFacingErrorKind::Provider,
        message: detail.clone(),
        next_action:
          "Validate the provider token, then retry or use Force Cleanup if provisioning was interrupted."
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

fn probe_external_ipv4() -> Option<String> {
  let client = reqwest::blocking::Client::builder()
    .timeout(Duration::from_secs(3))
    .build()
    .ok()?;

  let endpoints = [
    "https://api.ipify.org",
    "https://ipv4.icanhazip.com",
  ];

  for endpoint in endpoints {
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

    if let Ok(parsed) = candidate.parse::<IpAddr>() {
      if parsed.is_ipv4() {
        return Some(parsed.to_string());
      }
    }
  }

  None
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

