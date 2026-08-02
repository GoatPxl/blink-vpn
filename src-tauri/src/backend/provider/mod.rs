pub mod digitalocean;
pub mod retry;

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::Serialize;
use thiserror::Error;

use crate::backend::bootstrap::BootstrapOutputContract;
use crate::backend::db::{Database, DbError, ProviderRecord};
use crate::backend::secure_storage::{SecretStoreError, SecureStorage};

#[allow(unused_imports)]
pub use digitalocean::{
  Account, CreateDropletRequest, DEFAULT_IMAGE, DEFAULT_REGION, DEFAULT_SIZE, DigitalOceanConfig,
};

use digitalocean::{DigitalOceanApiClient, DigitalOceanError};
use retry::RetryPolicy;

pub trait ProviderAdapter: Send + Sync {
  fn provider_name(&self) -> &'static str;
  fn default_regions(&self) -> &'static [&'static str];
  fn validate_token(&self, token: &str) -> Result<Account, ProviderAdapterError>;
  fn list_regions(&self, token: &str) -> Result<Vec<String>, ProviderAdapterError>;
  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError>;
  fn create_droplet(
    &self,
    token: &str,
    droplet_name: &str,
    region: &str,
    size_slug: &str,
    cloud_init_user_data: &str,
  ) -> Result<ProvisionedDroplet, ProviderAdapterError>;
  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    droplet_id: u64,
    endpoint_ip: &str,
    readiness_marker_path: &str,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError>;
  fn droplet_exists(&self, token: &str, droplet_id: u64) -> Result<bool, ProviderAdapterError>;
  fn delete_droplet(&self, token: &str, droplet_id: u64) -> Result<(), ProviderAdapterError>;
}

#[derive(Clone)]
pub struct DigitalOceanProviderAdapter {
  client: DigitalOceanApiClient,
  config: DigitalOceanConfig,
}

impl DigitalOceanProviderAdapter {
  pub fn new() -> Result<Self, ProviderAdapterError> {
    let config = DigitalOceanConfig::phase1_defaults();
    let client = DigitalOceanApiClient::new_with_timeout(
      "https://api.digitalocean.com",
      RetryPolicy::digitalocean_default(),
      config.request_timeout,
    )
    .map_err(|source| ProviderAdapterError::Api {
      detail: "Unable to initialize DigitalOcean API client.".to_string(),
      source,
    })?;

    Ok(Self {
      client,
      config,
    })
  }
}

impl ProviderAdapter for DigitalOceanProviderAdapter {
  fn provider_name(&self) -> &'static str {
    "digitalocean"
  }

  fn default_regions(&self) -> &'static [&'static str] {
    &["nyc1", "sfo3", "fra1"]
  }

  fn validate_token(&self, token: &str) -> Result<Account, ProviderAdapterError> {
    self
      .client
      .validate_token(token)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean token validation request failed.".to_string(),
        source,
      })
  }

  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError> {
    let mut sizes = self
      .client
      .list_sizes(token)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean size catalog request failed.".to_string(),
        source,
      })?;

    sizes.sort_by(|left, right| {
      left
        .price_monthly
        .total_cmp(&right.price_monthly)
        .then_with(|| left.slug.cmp(&right.slug))
    });

    let fetched_at_epoch_seconds = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
      Ok(value) => value.as_secs().to_string(),
      Err(_) => "0".to_string(),
    };

    let pricing: Vec<ProviderPlanPricing> = sizes
      .into_iter()
      .filter(|size| size.available && size.regions.iter().any(|candidate| candidate == region))
      .map(|size| {
        let plan_label = if size.description.trim().is_empty() {
          size.slug.to_uppercase()
        } else {
          size.description.trim().to_string()
        };

        ProviderPlanPricing {
          plan_key: size.slug.clone(),
          plan_label,
          size_slug: size.slug,
          region: region.to_string(),
          memory_mb: size.memory,
          vcpus: size.vcpus,
          disk_gb: size.disk,
          transfer_tb: size.transfer,
          transfer_note: format!("Includes up to {:.2} TB transfer before provider overage policies apply.", size.transfer),
          monthly_usd: size.price_monthly,
          hourly_usd: size.price_hourly,
          estimated: false,
          source_timestamp: fetched_at_epoch_seconds.clone(),
        }
      })
      .collect();

    Ok(pricing)
  }

  fn list_regions(&self, token: &str) -> Result<Vec<String>, ProviderAdapterError> {
    let sizes = self
      .client
      .list_sizes(token)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean size catalog request failed.".to_string(),
        source,
      })?;

    let mut regions = BTreeSet::new();
    for size in sizes.into_iter().filter(|size| size.available) {
      for region in size.regions {
        if !region.trim().is_empty() {
          regions.insert(region);
        }
      }
    }

    Ok(regions.into_iter().collect())
  }

  fn create_droplet(
    &self,
    token: &str,
    droplet_name: &str,
    region: &str,
    size_slug: &str,
    cloud_init_user_data: &str,
  ) -> Result<ProvisionedDroplet, ProviderAdapterError> {
    let request = CreateDropletRequest {
      name: droplet_name.to_string(),
      region: if region.trim().is_empty() {
        self.config.region.to_string()
      } else {
        region.trim().to_string()
      },
      size: if size_slug.trim().is_empty() {
        self.config.size.to_string()
      } else {
        size_slug.trim().to_string()
      },
      image: self.config.image.to_string(),
      tags: vec!["goatstare-phase1".to_string()],
      user_data: Some(cloud_init_user_data.to_string()),
    };

    let created = self
      .client
      .create_droplet(token, &request)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean droplet create call failed.".to_string(),
        source,
      })?;

    let active = self
      .client
      .wait_for_droplet_active(
        token,
        created.id,
        self.config.poll_interval,
        self.config.poll_max_attempts,
        self.config.droplet_ready_timeout,
      )
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean droplet did not become active before timeout.".to_string(),
        source,
      })?;

    let public_ipv4 = self
      .client
      .extract_public_ipv4(&active)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "Active droplet did not expose public IPv4.".to_string(),
        source,
      })?;

    Ok(ProvisionedDroplet {
      droplet_id: active.id,
      public_ipv4,
    })
  }

  fn wait_for_bootstrap_ready(
    &self,
    _token: &str,
    _droplet_id: u64,
    _endpoint_ip: &str,
    readiness_marker_path: &str,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
    Err(ProviderAdapterError::Unsupported {
      detail: format!(
        "Cloud-init readiness marker retrieval from '{}' requires a remote retrieval channel and is not yet available in this phase.",
        readiness_marker_path
      ),
    })
  }

  fn droplet_exists(&self, token: &str, droplet_id: u64) -> Result<bool, ProviderAdapterError> {
    match self.client.get_droplet(token, droplet_id) {
      Ok(_) => Ok(true),
      Err(DigitalOceanError::NotFound { .. }) => Ok(false),
      Err(source) => Err(ProviderAdapterError::Api {
        detail: "DigitalOcean droplet existence check failed.".to_string(),
        source,
      }),
    }
  }

  fn delete_droplet(&self, token: &str, droplet_id: u64) -> Result<(), ProviderAdapterError> {
    self
      .client
      .delete_droplet(token, droplet_id)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean droplet delete call failed.".to_string(),
        source,
      })
  }
}

#[derive(Debug, Clone)]
pub struct ProvisionedDroplet {
  pub droplet_id: u64,
  pub public_ipv4: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderPlanPricing {
  pub plan_key: String,
  pub plan_label: String,
  pub size_slug: String,
  pub region: String,
  pub memory_mb: u64,
  pub vcpus: u64,
  pub disk_gb: u64,
  pub transfer_tb: f64,
  pub transfer_note: String,
  pub monthly_usd: f64,
  pub hourly_usd: f64,
  pub estimated: bool,
  pub source_timestamp: String,
}

#[derive(Clone)]
pub struct ProviderRegistry {
  digitalocean: Arc<dyn ProviderAdapter>,
}

#[derive(Debug, Error)]
pub enum ProviderAdapterError {
  #[error("DigitalOcean provider API call failed: {detail}")]
  Api {
    detail: String,
    #[source]
    source: DigitalOceanError,
  },
  #[error("Provider capability not available yet: {detail}")]
  Unsupported { detail: String },
}

#[derive(Debug, Error)]
pub enum ProviderFlowError {
  #[error("Provider database operation failed: {detail}")]
  Database {
    detail: String,
    #[source]
    source: DbError,
  },
  #[error("Provider secret storage operation failed: {detail}")]
  SecretStore {
    detail: String,
    #[source]
    source: SecretStoreError,
  },
  #[error("Provider adapter operation failed: {detail}")]
  Adapter {
    detail: String,
    #[source]
    source: ProviderAdapterError,
  },
  #[error("Provider '{provider_id}' was not found after persistence. {detail}")]
  MissingPersistedProvider { provider_id: String, detail: String },
  #[error("Provider '{provider_id}' has no token_ref. {detail}")]
  MissingTokenReference { provider_id: String, detail: String },
  #[error("Provider token_ref '{token_ref}' has no stored secret. {detail}")]
  MissingSecret { token_ref: String, detail: String },
}

impl ProviderRegistry {
  pub fn new() -> Self {
    let adapter = DigitalOceanProviderAdapter::new().unwrap_or_else(|err| {
      panic!("failed to initialize DigitalOcean adapter: {err}")
    });

    Self {
      digitalocean: Arc::new(adapter),
    }
  }

  pub fn from_adapter(adapter: Arc<dyn ProviderAdapter>) -> Self {
    Self { digitalocean: adapter }
  }

  pub fn active(&self) -> &dyn ProviderAdapter {
    self.digitalocean.as_ref()
  }

  pub fn create_digitalocean_provider_with_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    provider_name: &str,
    api_token: &str,
  ) -> Result<ProviderRecord, ProviderFlowError> {
    self.upsert_digitalocean_provider_with_token(db, secure_storage, provider_id, provider_name, api_token)
  }

  pub fn upsert_digitalocean_provider_with_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    provider_name: &str,
    api_token: &str,
  ) -> Result<ProviderRecord, ProviderFlowError> {
    let existing = db
      .get_provider(provider_id)
      .map_err(|source| ProviderFlowError::Database {
        detail: "Failed to inspect provider before token upsert.".to_string(),
        source,
      })?;
    let previous_token_ref = existing.as_ref().and_then(|record| record.token_ref.clone());

    let token_ref = secure_storage
      .store_digitalocean_api_token(provider_id, api_token)
      .map_err(|source| ProviderFlowError::SecretStore {
        detail: "Unable to save DigitalOcean API token in protected storage."
          .to_string(),
        source,
      })?;

    let update_result = match existing {
      Some(_) => db.update_provider(
        provider_id,
        provider_name,
        self.active().provider_name(),
        Some(&token_ref),
      ),
      None => db.create_provider(
        provider_id,
        provider_name,
        self.active().provider_name(),
        Some(&token_ref),
      ),
    };

    if let Err(source) = update_result {
      let _ = secure_storage.delete_digitalocean_api_token(&token_ref);
      return Err(ProviderFlowError::Database {
        detail: "Failed to persist provider row with updated token reference.".to_string(),
        source,
      });
    }

    if let Some(previous_token_ref) = previous_token_ref {
      let _ = secure_storage.delete_digitalocean_api_token(&previous_token_ref);
    }

    db
      .get_provider(provider_id)
      .map_err(|source| ProviderFlowError::Database {
        detail: "Failed to read back provider row after insert."
          .to_string(),
        source,
      })?
      .ok_or_else(|| ProviderFlowError::MissingPersistedProvider {
        provider_id: provider_id.to_string(),
        detail: "Expected provider row to exist after upsert call.".to_string(),
      })
  }

  pub fn validate_provider_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
  ) -> Result<Account, ProviderFlowError> {
    let token = self.resolve_provider_token(db, secure_storage, provider_id)?;
    self
      .active()
      .validate_token(&token)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Token validation through provider adapter failed.".to_string(),
        source,
      })
  }

  pub fn create_provider_droplet(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    droplet_name: &str,
    region: &str,
    size_slug: &str,
    cloud_init_user_data: &str,
  ) -> Result<ProvisionedDroplet, ProviderFlowError> {
    let token = self.resolve_provider_token(db, secure_storage, provider_id)?;
    self
      .active()
      .create_droplet(&token, droplet_name, region, size_slug, cloud_init_user_data)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Droplet provisioning through provider adapter failed.".to_string(),
        source,
      })
  }

  pub fn list_provider_plan_pricing(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderFlowError> {
    let token = self.resolve_provider_token(db, secure_storage, provider_id)?;
    self
      .active()
      .list_plan_pricing(&token, region)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Provider pricing lookup failed through adapter.".to_string(),
        source,
      })
  }

  pub fn list_provider_regions(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
  ) -> Result<Vec<String>, ProviderFlowError> {
    let token = self.resolve_provider_token(db, secure_storage, provider_id)?;
    self
      .active()
      .list_regions(&token)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Provider region lookup failed through adapter.".to_string(),
        source,
      })
  }

  pub fn wait_for_provider_bootstrap_ready(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    droplet_id: u64,
    endpoint_ip: &str,
    readiness_marker_path: &str,
  ) -> Result<BootstrapOutputContract, ProviderFlowError> {
    let token = self.resolve_provider_token(db, secure_storage, provider_id)?;
    self
      .active()
      .wait_for_bootstrap_ready(&token, droplet_id, endpoint_ip, readiness_marker_path)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Bootstrap readiness wait failed in provider adapter.".to_string(),
        source,
      })
  }

  pub fn delete_provider_droplet(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    droplet_id: u64,
  ) -> Result<(), ProviderFlowError> {
    let token = self.resolve_provider_token(db, secure_storage, provider_id)?;
    self
      .active()
      .delete_droplet(&token, droplet_id)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Droplet deletion through provider adapter failed.".to_string(),
        source,
      })
  }

  pub fn provider_droplet_exists(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    droplet_id: u64,
  ) -> Result<bool, ProviderFlowError> {
    let token = self.resolve_provider_token(db, secure_storage, provider_id)?;
    self
      .active()
      .droplet_exists(&token, droplet_id)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Droplet existence check through provider adapter failed.".to_string(),
        source,
      })
  }

  fn resolve_provider_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
  ) -> Result<String, ProviderFlowError> {
    let provider = db
      .get_provider(provider_id)
      .map_err(|source| ProviderFlowError::Database {
        detail: "Failed loading provider row for token resolution.".to_string(),
        source,
      })?
      .ok_or_else(|| ProviderFlowError::MissingPersistedProvider {
        provider_id: provider_id.to_string(),
        detail: "Cannot use provider because no DB row exists for this id.".to_string(),
      })?;

    let token_ref = provider
      .token_ref
      .ok_or_else(|| ProviderFlowError::MissingTokenReference {
        provider_id: provider_id.to_string(),
        detail: "Provider must be initialized with a token before adapter operations."
          .to_string(),
      })?;

    let resolved = secure_storage
      .get_digitalocean_api_token(&token_ref)
      .map_err(|source| ProviderFlowError::SecretStore {
        detail: "Failed retrieving provider token from secure storage.".to_string(),
        source,
      })?;

    if let Some(token) = resolved {
      return Ok(token);
    }

    // Self-heal stale DB pointer so UI can re-register token cleanly.
    let _ = db.update_provider_token_ref(provider_id, None);

    Err(ProviderFlowError::MissingSecret {
        token_ref,
        detail: "Stored token reference is stale: secure storage entry is missing. Re-save your DigitalOcean token in onboarding Step 2."
          .to_string(),
      })
  }

  pub fn resolve_provider_token_for_orchestrator(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
  ) -> Result<String, ProviderFlowError> {
    self.resolve_provider_token(db, secure_storage, provider_id)
  }
}


#[cfg(test)]
mod tests {
  use std::path::PathBuf;
  use std::time::{SystemTime, UNIX_EPOCH};

  use rusqlite::{Connection, params};

  use crate::backend::db::Database;
  use crate::backend::secure_storage::SecureStorage;

  use super::ProviderRegistry;

  fn test_db_path(test_name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("clock must be after unix epoch")
      .as_nanos();
    path.push(format!("goatstare_provider_{test_name}_{nanos}.sqlite3"));
    path
  }

  #[test]
  fn provider_flow_persists_token_reference_not_plaintext() {
    let db = Database::new(test_db_path("token_ref_only"));
    db.run_migrations().expect("migrations should succeed");

    let registry = ProviderRegistry::new();
    let secure_storage = SecureStorage::dev_mock();
    let plaintext_token = "dop_v1_sensitive_plaintext_token";

    let provider = registry
      .create_digitalocean_provider_with_token(
        &db,
        &secure_storage,
        "provider-1",
        "DigitalOcean",
        plaintext_token,
      )
      .expect("provider flow should succeed");

    let token_ref = provider.token_ref.expect("provider should store token_ref");
    assert_ne!(token_ref, plaintext_token);

    let recovered = secure_storage
      .get_digitalocean_api_token(&token_ref)
      .expect("token retrieval should succeed")
      .expect("token should exist in mock secret storage");
    assert_eq!(recovered, plaintext_token);

    let connection = Connection::open(db.path()).expect("db path should be openable");
    let row = connection
      .query_row(
        "SELECT id, name, slug, IFNULL(token_ref, '') FROM providers WHERE id = ?1",
        params!["provider-1"],
        |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
          ))
        },
      )
      .expect("provider row should be queryable");

    assert_ne!(row.0, plaintext_token);
    assert_ne!(row.1, plaintext_token);
    assert_ne!(row.2, plaintext_token);
    assert_ne!(row.3, plaintext_token);
  }

  #[test]
  fn validate_token_clears_stale_token_reference_when_secret_missing() {
    let db = Database::new(test_db_path("stale_token_ref_cleared"));
    db.run_migrations().expect("migrations should succeed");

    let registry = ProviderRegistry::new();
    let secure_storage = SecureStorage::dev_mock();

    let provider = registry
      .create_digitalocean_provider_with_token(
        &db,
        &secure_storage,
        "provider-1",
        "DigitalOcean",
        "dop_v1_stale_test_token",
      )
      .expect("provider create should succeed");

    let token_ref = provider
      .token_ref
      .clone()
      .expect("token ref should be present");

    secure_storage
      .delete_digitalocean_api_token(&token_ref)
      .expect("mock token delete should succeed");

    let result = registry.validate_provider_token(&db, &secure_storage, "provider-1");
    assert!(result.is_err());

    let updated = db
      .get_provider("provider-1")
      .expect("provider lookup should succeed")
      .expect("provider should still exist");
    assert!(updated.token_ref.is_none());
  }

  #[cfg(target_os = "windows")]
  #[test]
  fn provider_flow_roundtrip_with_windows_secure_storage_and_db() {
    let db = Database::new(test_db_path("windows_secure_storage_roundtrip"));
    db.run_migrations().expect("migrations should succeed");

    let registry = ProviderRegistry::new();
    let secure_storage = SecureStorage::new();
    let token = "dop_v1_windows_roundtrip_test_token";

    let provider = registry
      .create_digitalocean_provider_with_token(
        &db,
        &secure_storage,
        "provider-1",
        "DigitalOcean",
        token,
      )
      .expect("provider create should succeed with Windows secure storage");

    let token_ref = provider
      .token_ref
      .clone()
      .expect("token ref should exist");

    let persisted = db
      .get_provider("provider-1")
      .expect("provider lookup should succeed")
      .expect("provider row should exist");

    assert_eq!(persisted.token_ref.as_deref(), Some(token_ref.as_str()));

    let recovered = secure_storage
      .get_digitalocean_api_token(&token_ref)
      .expect("token lookup should succeed");
    assert_eq!(recovered.as_deref(), Some(token));

    secure_storage
      .delete_digitalocean_api_token(&token_ref)
      .expect("cleanup delete should succeed");
  }
}