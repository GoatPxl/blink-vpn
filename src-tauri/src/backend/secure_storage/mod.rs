use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
  DigitalOceanApiToken,
  ClientPrivateKey,
}

impl SecretKind {
  fn prefix(self) -> &'static str {
    match self {
      Self::DigitalOceanApiToken => "do-token",
      Self::ClientPrivateKey => "wg-client-key",
    }
  }
}

pub trait SecretStore: Send + Sync {
  fn store_secret(
    &self,
    kind: SecretKind,
    owner_id: &str,
    secret: &str,
  ) -> Result<String, SecretStoreError>;
  fn retrieve_secret(&self, secret_ref: &str) -> Result<Option<String>, SecretStoreError>;
  fn delete_secret(&self, secret_ref: &str) -> Result<(), SecretStoreError>;
  fn backend_name(&self) -> &'static str;
}

#[derive(Debug, Error)]
pub enum SecretStoreError {
  #[error("Secret storage write failed: {detail}")]
  Store { detail: String },
  #[error("Secret storage read failed: {detail}")]
  Retrieve { detail: String },
  #[error("Secret storage delete failed: {detail}")]
  Delete { detail: String },
  #[error("Invalid secret reference format: {detail}")]
  InvalidReference { detail: String },
}

#[derive(Clone)]
pub struct SecureStorage {
  inner: Arc<dyn SecretStore>,
}

impl SecureStorage {
  pub fn new() -> Self {
    #[cfg(target_os = "windows")]
    {
      Self {
        inner: Arc::new(WindowsCredentialStore::new()),
      }
    }

    #[cfg(not(target_os = "windows"))]
    {
      Self {
        inner: Arc::new(MockSecretStore::default()),
      }
    }
  }

  pub fn from_store(store: Arc<dyn SecretStore>) -> Self {
    Self { inner: store }
  }

  pub fn dev_mock() -> Self {
    Self {
      inner: Arc::new(MockSecretStore::default()),
    }
  }

  pub fn backend_name(&self) -> &'static str {
    self.inner.backend_name()
  }

  pub fn store_digitalocean_api_token(
    &self,
    provider_id: &str,
    token: &str,
  ) -> Result<String, SecretStoreError> {
    self
      .inner
      .store_secret(SecretKind::DigitalOceanApiToken, provider_id, token)
  }

  pub fn get_digitalocean_api_token(
    &self,
    token_ref: &str,
  ) -> Result<Option<String>, SecretStoreError> {
    self.inner.retrieve_secret(token_ref)
  }

  pub fn delete_digitalocean_api_token(&self, token_ref: &str) -> Result<(), SecretStoreError> {
    self.inner.delete_secret(token_ref)
  }

  pub fn store_client_private_key(
    &self,
    node_id: &str,
    private_key: &str,
  ) -> Result<String, SecretStoreError> {
    self
      .inner
      .store_secret(SecretKind::ClientPrivateKey, node_id, private_key)
  }

  pub fn get_client_private_key(
    &self,
    key_ref: &str,
  ) -> Result<Option<String>, SecretStoreError> {
    self.inner.retrieve_secret(key_ref)
  }

  pub fn delete_client_private_key(&self, key_ref: &str) -> Result<(), SecretStoreError> {
    self.inner.delete_secret(key_ref)
  }
}

#[cfg(target_os = "windows")]
#[derive(Clone)]
pub struct WindowsCredentialStore {
  service_name: String,
}

#[cfg(target_os = "windows")]
impl WindowsCredentialStore {
  pub fn new() -> Self {
    Self {
      service_name: "com.goatstare.vpn.phase1".to_string(),
    }
  }

  fn build_secret_ref(&self, kind: SecretKind, owner_id: &str) -> String {
    let owner = sanitize_segment(owner_id);
    format!("{}:{}:{}", kind.prefix(), owner, Uuid::new_v4())
  }

  fn parse_secret_ref(secret_ref: &str) -> Result<(), SecretStoreError> {
    let parts = secret_ref.split(':').collect::<Vec<_>>();
    if parts.len() != 3 {
      return Err(SecretStoreError::InvalidReference {
        detail: "Expected secret_ref format kind:owner:uuid.".to_string(),
      });
    }
    Ok(())
  }
}

#[cfg(target_os = "windows")]
impl SecretStore for WindowsCredentialStore {
  fn store_secret(
    &self,
    kind: SecretKind,
    owner_id: &str,
    secret: &str,
  ) -> Result<String, SecretStoreError> {
    let secret_ref = self.build_secret_ref(kind, owner_id);
    let entry = keyring::Entry::new(&self.service_name, &secret_ref).map_err(|source| {
      SecretStoreError::Store {
        detail: format!(
          "Could not initialize Windows credential entry. Ensure Credential Manager is available. ({source})"
        ),
      }
    })?;

    entry
      .set_password(secret)
      .map_err(|source| SecretStoreError::Store {
        detail: format!(
          "Could not persist secret to Windows protected storage. ({source})"
        ),
      })?;

    Ok(secret_ref)
  }

  fn retrieve_secret(&self, secret_ref: &str) -> Result<Option<String>, SecretStoreError> {
    Self::parse_secret_ref(secret_ref)?;
    let entry = keyring::Entry::new(&self.service_name, secret_ref).map_err(|source| {
      SecretStoreError::Retrieve {
        detail: format!(
          "Could not initialize credential lookup. ({source})"
        ),
      }
    })?;

    match entry.get_password() {
      Ok(secret) => Ok(Some(secret)),
      Err(keyring::Error::NoEntry) => Ok(None),
      Err(source) => Err(SecretStoreError::Retrieve {
        detail: format!(
          "Could not read secret from Windows protected storage. ({source})"
        ),
      }),
    }
  }

  fn delete_secret(&self, secret_ref: &str) -> Result<(), SecretStoreError> {
    Self::parse_secret_ref(secret_ref)?;
    let entry = keyring::Entry::new(&self.service_name, secret_ref).map_err(|source| {
      SecretStoreError::Delete {
        detail: format!(
          "Could not initialize credential delete operation. ({source})"
        ),
      }
    })?;

    match entry.delete_credential() {
      Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
      Err(source) => Err(SecretStoreError::Delete {
        detail: format!(
          "Could not delete secret from Windows protected storage. ({source})"
        ),
      }),
    }
  }

  fn backend_name(&self) -> &'static str {
    "windows-credential-manager"
  }
}

#[derive(Clone, Default)]
pub struct MockSecretStore {
  secrets: Arc<Mutex<HashMap<String, String>>>,
}

impl MockSecretStore {
  fn build_secret_ref(kind: SecretKind, owner_id: &str) -> String {
    let owner = sanitize_segment(owner_id);
    format!("{}:{}:{}", kind.prefix(), owner, Uuid::new_v4())
  }
}

impl SecretStore for MockSecretStore {
  fn store_secret(
    &self,
    kind: SecretKind,
    owner_id: &str,
    secret: &str,
  ) -> Result<String, SecretStoreError> {
    let secret_ref = Self::build_secret_ref(kind, owner_id);
    let mut guard = self.secrets.lock().map_err(|_| SecretStoreError::Store {
      detail: "Could not lock mock secret storage map.".to_string(),
    })?;
    guard.insert(secret_ref.clone(), secret.to_string());
    Ok(secret_ref)
  }

  fn retrieve_secret(&self, secret_ref: &str) -> Result<Option<String>, SecretStoreError> {
    let guard = self.secrets.lock().map_err(|_| SecretStoreError::Retrieve {
      detail: "Could not lock mock secret storage map.".to_string(),
    })?;
    Ok(guard.get(secret_ref).cloned())
  }

  fn delete_secret(&self, secret_ref: &str) -> Result<(), SecretStoreError> {
    let mut guard = self.secrets.lock().map_err(|_| SecretStoreError::Delete {
      detail: "Could not lock mock secret storage map.".to_string(),
    })?;
    guard.remove(secret_ref);
    Ok(())
  }

  fn backend_name(&self) -> &'static str {
    "mock-in-memory"
  }
}

fn sanitize_segment(value: &str) -> String {
  value
    .chars()
    .map(|ch| {
      if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
        ch
      } else {
        '_'
      }
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::SecureStorage;

  #[test]
  fn mock_secret_store_roundtrip_for_api_token_and_private_key() {
    let storage = SecureStorage::dev_mock();

    let token_ref = storage
      .store_digitalocean_api_token("provider-1", "dop_v1_secret")
      .expect("token should store in mock backend");

    let key_ref = storage
      .store_client_private_key("node-1", "wg-private-key")
      .expect("private key should store in mock backend");

    let token = storage
      .get_digitalocean_api_token(&token_ref)
      .expect("token lookup should succeed")
      .expect("token should exist");
    let key = storage
      .get_client_private_key(&key_ref)
      .expect("private key lookup should succeed")
      .expect("private key should exist");

    assert_eq!(token, "dop_v1_secret");
    assert_eq!(key, "wg-private-key");

    storage
      .delete_digitalocean_api_token(&token_ref)
      .expect("token delete should succeed");
    storage
      .delete_client_private_key(&key_ref)
      .expect("key delete should succeed");

    assert!(
      storage
        .get_digitalocean_api_token(&token_ref)
        .expect("token lookup should succeed")
        .is_none()
    );
    assert!(
      storage
        .get_client_private_key(&key_ref)
        .expect("private key lookup should succeed")
        .is_none()
    );
  }
}