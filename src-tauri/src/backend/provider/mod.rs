use thiserror::Error;

use crate::backend::db::{Database, DbError, ProviderRecord};
use crate::backend::secure_storage::{SecretStoreError, SecureStorage};

#[derive(Clone)]
pub struct DigitalOceanProviderAdapter;

pub trait ProviderAdapter {
  fn provider_name(&self) -> &'static str;
  fn default_regions(&self) -> &'static [&'static str];
}

impl ProviderAdapter for DigitalOceanProviderAdapter {
  fn provider_name(&self) -> &'static str {
    "digitalocean"
  }

  fn default_regions(&self) -> &'static [&'static str] {
    &["nyc1", "sfo3", "fra1"]
  }
}

#[derive(Clone)]
pub struct ProviderRegistry {
  digitalocean: DigitalOceanProviderAdapter,
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
  #[error("Provider '{provider_id}' was not found after persistence. {detail}")]
  MissingPersistedProvider { provider_id: String, detail: String },
}

impl ProviderRegistry {
  pub fn new() -> Self {
    Self {
      digitalocean: DigitalOceanProviderAdapter,
    }
  }

  pub fn active(&self) -> &dyn ProviderAdapter {
    &self.digitalocean
  }

  pub fn create_digitalocean_provider_with_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    provider_name: &str,
    api_token: &str,
  ) -> Result<ProviderRecord, ProviderFlowError> {
    let token_ref = secure_storage
      .store_digitalocean_api_token(provider_id, api_token)
      .map_err(|source| ProviderFlowError::SecretStore {
        detail: "Unable to save DigitalOcean API token in protected storage."
          .to_string(),
        source,
      })?;

    db
      .create_provider(
        provider_id,
        provider_name,
        self.digitalocean.provider_name(),
        Some(&token_ref),
      )
      .map_err(|source| ProviderFlowError::Database {
        detail: "Failed to create provider row with token reference only."
          .to_string(),
        source,
      })?;

    db
      .get_provider(provider_id)
      .map_err(|source| ProviderFlowError::Database {
        detail: "Failed to read back provider row after insert."
          .to_string(),
        source,
      })?
      .ok_or_else(|| ProviderFlowError::MissingPersistedProvider {
        provider_id: provider_id.to_string(),
        detail: "Expected provider row to exist after create call.".to_string(),
      })
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
}