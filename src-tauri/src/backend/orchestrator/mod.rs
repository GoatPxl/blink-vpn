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

    if let Err(error) = wireguard.activate_tunnel(&request.tunnel_name) {
      let _ = db.update_tunnel_status(
        &request.tunnel_id,
        TunnelStatus::Error,
        Some("WireGuard tunnel activation failed"),
      );
      return Err(OrchestratorError::WireGuard {
        detail: "Could not activate client tunnel service.".to_string(),
        source: error,
      });
    }

    db
      .update_tunnel_status(&request.tunnel_id, TunnelStatus::Connected, None)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed marking tunnel as Connected after activation.".to_string(),
        source,
      })?;

    let _ = db.transition_vpn_node_status(&request.node_id, NodeLifecycleStatus::Connected, None);

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

    wireguard
      .deactivate_tunnel(&tunnel.name)
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not deactivate WireGuard tunnel service.".to_string(),
        source,
      })?;

    db
      .update_tunnel_status(tunnel_id, TunnelStatus::Disconnected, None)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed marking tunnel as Disconnected after stop.".to_string(),
        source,
      })?;

    let _ = db.transition_vpn_node_status(&tunnel.node_id, NodeLifecycleStatus::Ready, None);
    Ok(())
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

    wireguard
      .activate_tunnel(&tunnel.name)
      .map_err(|source| OrchestratorError::WireGuard {
        detail: "Could not activate WireGuard tunnel service.".to_string(),
        source,
      })?;

    db
      .update_tunnel_status(tunnel_id, TunnelStatus::Connected, None)
      .map_err(|source| OrchestratorError::Database {
        detail: "Failed marking tunnel as Connected after activation.".to_string(),
        source,
      })?;

    let _ = db.transition_vpn_node_status(&tunnel.node_id, NodeLifecycleStatus::Connected, None);
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

    Ok(status)
  }
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

  #[derive(Default)]
  struct MockAdapterState {
    delete_calls: usize,
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
}