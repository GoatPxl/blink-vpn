use thiserror::Error;
use uuid::Uuid;

use crate::backend::bootstrap::{
  Bootstrap, BootstrapError, BootstrapParams, DEFAULT_CLIENT_CIDR, DEFAULT_INTERFACE_NAME,
  DEFAULT_SERVER_INTERFACE_CIDR, DEFAULT_WIREGUARD_PORT,
};
use crate::backend::db::Database;
use crate::backend::db::{DbError, NodeLifecycleStatus, TunnelStatus};
use crate::backend::provider::{
  ProvisionedDroplet, ProviderFlowError, ProviderRegistry,
};
use crate::backend::secure_storage::SecureStorage;
use crate::backend::wireguard::{
  ClientConfigParams, DEFAULT_CLIENT_KEEPALIVE_SECONDS, TunnelRuntimeStatus, WireGuardError,
  WireGuardManager,
};

#[derive(Clone)]
pub struct Orchestrator;

#[derive(Debug, Clone)]
pub struct CreateVpnNodeRequest {
  pub node_id: String,
  pub provider_id: String,
  pub node_name: String,
  pub region: String,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct CreateClientTunnelRequest {
  pub tunnel_id: String,
  pub node_id: String,
  pub tunnel_name: String,
  pub client_address_cidr: String,
  pub dns_server: Option<String>,
  pub allowed_ips: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreateClientTunnelResult {
  pub tunnel_id: String,
  pub tunnel_name: String,
  pub client_public_key: String,
}

#[derive(Debug, Clone)]
pub struct ConnectTunnelRequest {
  pub node_id: String,
  pub tunnel_id: String,
}

#[derive(Debug, Clone)]
pub struct DisconnectTunnelRequest {
  pub node_id: String,
  pub tunnel_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileIssueKind {
  StaleTunnelStatusRepaired,
  MissingDroplet,
  MissingTunnelArtifact,
  MissingNodeRuntimeData,
  StartupDependencyUnavailable,
  ProviderCheckFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileIssue {
  pub kind: ReconcileIssueKind,
  pub node_id: Option<String>,
  pub tunnel_id: Option<String>,
  pub repaired: bool,
  pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct ReconcileReport {
  pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserFacingErrorKind {
  InvalidState,
  MissingResource,
  Provider,
  WireGuard,
  Persistence,
  Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFacingError {
  pub kind: UserFacingErrorKind,
  pub message: String,
  pub next_action: String,
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
}

impl Orchestrator {
  pub fn new() -> Self {
    Self
  }

  pub fn status(&self) -> &'static str {
    "idle"
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
    let artifact = bootstrap
      .render_cloud_init(&BootstrapParams {
        server_endpoint_ip: "0.0.0.0".to_string(),
        listen_port: DEFAULT_WIREGUARD_PORT,
        server_interface_cidr: DEFAULT_SERVER_INTERFACE_CIDR.to_string(),
        client_cidr: DEFAULT_CLIENT_CIDR.to_string(),
        interface_name: DEFAULT_INTERFACE_NAME.to_string(),
      })
      .map_err(|source| OrchestratorError::Bootstrap {
        detail: "Could not render bootstrap cloud-init for droplet create.".to_string(),
        source,
      })?;

    providers
      .create_provider_droplet(
        db,
        secure_storage,
        provider_id,
        droplet_name,
        &artifact.cloud_init_user_data,
      )
      .map_err(|source| OrchestratorError::Provider {
        detail: "Provisioning flow failed while creating or activating droplet.".to_string(),
        source,
      })
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
      "correlation_id={} node_id={} stage=start_create_vpn_node provider_id={}",
      correlation_id,
      request.node_id,
      request.provider_id
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
        detail: "Provider token failed validation before provisioning node.".to_string(),
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

    let bootstrap_artifact = bootstrap
      .render_cloud_init(&BootstrapParams {
        server_endpoint_ip: "0.0.0.0".to_string(),
        listen_port: DEFAULT_WIREGUARD_PORT,
        server_interface_cidr: DEFAULT_SERVER_INTERFACE_CIDR.to_string(),
        client_cidr: DEFAULT_CLIENT_CIDR.to_string(),
        interface_name: DEFAULT_INTERFACE_NAME.to_string(),
      })
      .map_err(|source| OrchestratorError::Bootstrap {
        detail: "Failed rendering cloud-init user-data for node bootstrap.".to_string(),
        source,
      })?;

    let mut created_droplet: Option<ProvisionedDroplet> = None;

    let flow_result: Result<CreateVpnNodeResult, OrchestratorError> = (|| {
      let droplet = providers
        .create_provider_droplet(
          db,
          secure_storage,
          &request.provider_id,
          &request.node_name,
          &bootstrap_artifact.cloud_init_user_data,
        )
        .map_err(|source| OrchestratorError::Provider {
          detail: "Droplet creation call failed in create-node flow.".to_string(),
          source,
        })?;

      created_droplet = Some(droplet.clone());

      db
        .update_vpn_node_runtime_details(
          &request.node_id,
          Some(&droplet.droplet_id.to_string()),
          Some(&droplet.public_ipv4),
          None,
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
          droplet.droplet_id,
          &droplet.public_ipv4,
          &bootstrap_artifact.readiness_marker_path,
        )
        .map_err(|source| OrchestratorError::Provider {
          detail: "Bootstrap readiness marker wait failed.".to_string(),
          source,
        })?;

      db
        .update_vpn_node_runtime_details(
          &request.node_id,
          Some(&droplet.droplet_id.to_string()),
          Some(&contract.server_endpoint_ip),
          Some(&contract.server_public_key),
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
        "correlation_id={} node_id={} stage=create_vpn_node_ready_success droplet_id={} endpoint_ip={}",
        correlation_id,
        request.node_id,
        droplet.droplet_id,
        contract.server_endpoint_ip
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
        }
      }

      let _ = db.transition_vpn_node_status(
        &request.node_id,
        NodeLifecycleStatus::Error,
        Some(&final_reason),
      );

      log::error!(
        "correlation_id={} node_id={} stage=create_vpn_node_ready_error reason={}",
        correlation_id,
        request.node_id,
        final_reason
      );

      return Err(error);
    }

    flow_result
  }

  pub fn create_and_activate_client_tunnel(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
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

    let server_public_key = node
      .server_public_key
      .ok_or_else(|| OrchestratorError::InvalidState {
        node_id: request.node_id.clone(),
        detail: "VPN node is missing server public key; cannot build client config.".to_string(),
      })?;
    let endpoint_ip = node.public_ipv4.ok_or_else(|| OrchestratorError::InvalidState {
      node_id: request.node_id.clone(),
      detail: "VPN node is missing public endpoint IP; cannot build client config.".to_string(),
    })?;

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
          &request.tunnel_name,
          Some(&request.tunnel_name),
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
    }

    let keypair = wireguard
      .generate_client_keypair()
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not generate client WireGuard keypair locally.".to_string(),
        source,
      })?;

    let config = wireguard
      .build_client_config(&ClientConfigParams {
        private_key: keypair.private_key,
        client_address_cidr: request.client_address_cidr.clone(),
        dns_server: request.dns_server.clone(),
        server_public_key,
        server_endpoint_ip: endpoint_ip,
        server_listen_port: DEFAULT_WIREGUARD_PORT,
        allowed_ips: request.allowed_ips.clone(),
        persistent_keepalive_seconds: DEFAULT_CLIENT_KEEPALIVE_SECONDS,
      })
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not render client WireGuard config.".to_string(),
        source,
      })?;

    if let Err(error) = wireguard.import_tunnel(&request.tunnel_name, &config) {
      let _ = db.update_tunnel_status(
        &request.tunnel_id,
        TunnelStatus::Error,
        Some("WireGuard tunnel import failed"),
      );
      return Err(OrchestratorError::WireGuard {
        detail: "Could not import client tunnel into WireGuard for Windows.".to_string(),
        source: error,
      });
    }

    self.connect_tunnel(
      wireguard,
      db,
      &ConnectTunnelRequest {
        node_id: request.node_id.clone(),
        tunnel_id: request.tunnel_id.clone(),
      },
    )?;

    Ok(CreateClientTunnelResult {
      tunnel_id: request.tunnel_id.clone(),
      tunnel_name: request.tunnel_name.clone(),
      client_public_key: keypair.public_key,
    })
  }

  pub fn deactivate_client_tunnel(
    &self,
    wireguard: &WireGuardManager,
    db: &Database,
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

    if tunnel.status == TunnelStatus::Connected && node.lifecycle_status == NodeLifecycleStatus::Connected {
      let runtime = self.read_client_tunnel_status(wireguard, db, &request.tunnel_id)?;
      if runtime.state == crate::backend::wireguard::TunnelServiceState::Running {
        return Ok(());
      }
    }

    wireguard
      .activate_tunnel(&tunnel.name)
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not activate WireGuard tunnel service in connect flow.".to_string(),
        source,
      })?;

    let runtime = wireguard
      .read_tunnel_status(&tunnel.name)
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

    let runtime = wireguard
      .read_tunnel_status(&tunnel.name)
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not verify tunnel runtime state after disconnect.".to_string(),
        source,
      })?;

    persist_tunnel_stats_snapshot(db, &request.tunnel_id, &runtime)?;

    db
      .update_tunnel_status(&request.tunnel_id, TunnelStatus::Disconnected, None)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed persisting tunnel status after disconnect flow.".to_string(),
        source,
      })?;

    let _ = db.transition_vpn_node_status(&request.node_id, NodeLifecycleStatus::Ready, None);
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

    let status = wireguard
      .read_tunnel_status(&tunnel.name)
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not read WireGuard runtime status.".to_string(),
        source,
      })?;

    let db_status = match status.state {
      crate::backend::wireguard::TunnelServiceState::Running => TunnelStatus::Connected,
      crate::backend::wireguard::TunnelServiceState::Stopped => TunnelStatus::Disconnected,
      crate::backend::wireguard::TunnelServiceState::Unknown => tunnel.status,
    };
    let _ = db.update_tunnel_status(tunnel_id, db_status, None);
    let _ = persist_tunnel_stats_snapshot(db, tunnel_id, &status);

    Ok(status)
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
            let detail = "Tunnel service is missing locally; re-import tunnel configuration."
              .to_string();
            let _ = db.update_tunnel_status(&tunnel.id, TunnelStatus::Error, Some(&detail));
            report.issues.push(ReconcileIssue {
              kind: ReconcileIssueKind::MissingTunnelArtifact,
              node_id: Some(node.id.clone()),
              tunnel_id: Some(tunnel.id.clone()),
              repaired: false,
              detail,
            });
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

#[cfg(test)]
mod tests {
  use std::sync::{Arc, Mutex};

  use std::path::PathBuf;
  use std::time::{SystemTime, UNIX_EPOCH};

  use super::*;
  use crate::backend::provider::{Account, ProviderAdapter, ProviderAdapterError};
  use crate::backend::secure_storage::SecureStorage;
  use crate::backend::provider::ProviderRegistry;
  use crate::backend::wireguard::{CommandExecError, CommandOutput, CommandRunner};

  #[derive(Default)]
  struct MockAdapterState {
    delete_calls: usize,
    droplet_exists: bool,
  }

  struct MockProviderAdapter {
    fail_bootstrap_wait: bool,
    state: Arc<Mutex<MockAdapterState>>,
  }

  impl MockProviderAdapter {
    fn new(fail_bootstrap_wait: bool) -> Self {
      Self {
        fail_bootstrap_wait,
        state: Arc::new(Mutex::new(MockAdapterState {
          delete_calls: 0,
          droplet_exists: true,
        })),
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

    fn create_droplet(
      &self,
      _token: &str,
      _droplet_name: &str,
      _cloud_init_user_data: &str,
    ) -> Result<ProvisionedDroplet, ProviderAdapterError> {
      Ok(ProvisionedDroplet {
        droplet_id: 42,
        public_ipv4: "203.0.113.90".to_string(),
      })
    }

    fn wait_for_bootstrap_ready(
      &self,
      _token: &str,
      _droplet_id: u64,
      endpoint_ip: &str,
      _readiness_marker_path: &str,
    ) -> Result<crate::backend::bootstrap::BootstrapOutputContract, ProviderAdapterError> {
      if self.fail_bootstrap_wait {
        return Err(ProviderAdapterError::Unsupported {
          detail: "mock bootstrap marker not ready".to_string(),
        });
      }

      Ok(crate::backend::bootstrap::BootstrapOutputContract {
        server_endpoint_ip: endpoint_ip.to_string(),
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
  }

  #[derive(Clone)]
  struct MockWireGuardRunner {
    state: Arc<Mutex<MockWireGuardState>>,
  }

  #[derive(Clone)]
  struct MockWireGuardState {
    service_exists: bool,
    service_running: bool,
  }

  impl MockWireGuardRunner {
    fn running() -> Self {
      Self {
        state: Arc::new(Mutex::new(MockWireGuardState {
          service_exists: true,
          service_running: true,
        })),
      }
    }

    fn stopped() -> Self {
      Self {
        state: Arc::new(Mutex::new(MockWireGuardState {
          service_exists: true,
          service_running: false,
        })),
      }
    }

    fn missing_service() -> Self {
      Self {
        state: Arc::new(Mutex::new(MockWireGuardState {
          service_exists: false,
          service_running: false,
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
      if program == "wg" && args == ["--version"] {
        return Ok(CommandOutput {
          stdout: "wireguard-tools v1".to_string(),
          stderr: String::new(),
        });
      }

      if program == "wireguard" && args == ["/help"] {
        return Ok(CommandOutput {
          stdout: "WireGuard for Windows".to_string(),
          stderr: String::new(),
        });
      }

      if program == "sc" && args.len() >= 2 {
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
            return Ok(CommandOutput {
              stdout: "stopped".to_string(),
              stderr: String::new(),
            });
          }
          _ => {}
        }
      }

      if program == "wg" && args.len() == 3 && args[0] == "show" && args[2] == "dump" {
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

      Err(CommandExecError::NonZeroExit {
        program: program.to_string(),
        code: Some(1),
        detail: "unsupported mock command".to_string(),
      })
    }
  }

  fn setup_connected_node_and_tunnel(db: &Database) {
    db
      .create_vpn_node("node-connect", "provider-1", "Node Connect", "nyc1")
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
      .store_digitalocean_api_token("provider-1", "token-abc")
      .expect("token should store");

    db
      .create_provider("provider-1", "DigitalOcean", "digitalocean", Some(&token_ref))
      .expect("provider should insert");
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
          provider_id: "provider-1".to_string(),
          node_name: "node-main".to_string(),
          region: "nyc1".to_string(),
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
        provider_id: "provider-1".to_string(),
        node_name: "node-main".to_string(),
        region: "nyc1".to_string(),
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
  fn connect_flow_requires_existing_tunnel() {
    let db = Database::new(test_db_path("connect_missing_tunnel"));
    db.run_migrations().expect("migrations should succeed");

    db
      .create_provider("provider-1", "DigitalOcean", "digitalocean", Some("do-token:provider-1:abc"))
      .expect("provider should insert");
    db
      .create_vpn_node("node-1", "provider-1", "Node", "nyc1")
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
    let result = orchestrator.connect_tunnel(
      &wireguard,
      &db,
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
  fn reconcile_flags_deleted_droplet() {
    let db = Database::new(test_db_path("reconcile_deleted_droplet"));
    db.run_migrations().expect("migrations should succeed");

    let secure_storage = SecureStorage::dev_mock();
    setup_provider_and_token(&db, &secure_storage);
    db
      .create_vpn_node("node-1", "provider-1", "Node", "nyc1")
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
  fn reconcile_flags_missing_local_tunnel_artifact() {
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
      .any(|issue| issue.kind == ReconcileIssueKind::MissingTunnelArtifact));

    let tunnel = db
      .get_tunnel("tunnel-connect")
      .expect("query should succeed")
      .expect("tunnel should exist");
    assert_eq!(tunnel.status, TunnelStatus::Error);
  }
}

impl OrchestratorError {
  pub fn to_user_facing(&self) -> UserFacingError {
    match self {
      Self::InvalidState { detail, .. } => UserFacingError {
        kind: UserFacingErrorKind::InvalidState,
        message: detail.clone(),
        next_action: "Resolve lifecycle mismatch and retry the requested action.".to_string(),
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
          next_action: "Verify local app data integrity and retry.".to_string(),
        }
      }
      Self::Provider { detail, .. } => UserFacingError {
        kind: UserFacingErrorKind::Provider,
        message: detail.clone(),
        next_action: "Verify provider token and droplet state, then retry.".to_string(),
      },
      Self::WireGuard { detail, source } => UserFacingError {
        kind: UserFacingErrorKind::WireGuard,
        message: format!("{detail}: {source}"),
        next_action: "Ensure WireGuard for Windows is installed and tunnel service exists.".to_string(),
      },
      Self::Bootstrap { detail, .. } => UserFacingError {
        kind: UserFacingErrorKind::Unknown,
        message: detail.clone(),
        next_action: "Retry node creation and inspect bootstrap settings.".to_string(),
      },
    }
  }
}