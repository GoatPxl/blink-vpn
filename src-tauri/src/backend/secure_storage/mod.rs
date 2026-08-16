use std::collections::HashMap;
use std::sync::{Arc, Mutex};
#[cfg(target_os = "windows")]
use std::{ffi::c_void, ptr::null_mut};

use thiserror::Error;
use uuid::Uuid;
#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{FILETIME, GetLastError, ERROR_NOT_FOUND};
#[cfg(target_os = "windows")]
use windows_sys::Win32::Security::Credentials::{
  CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
  CRED_TYPE_GENERIC,
};

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
  session_cache: Arc<Mutex<HashMap<String, String>>>,
}

impl SecureStorage {
  pub fn new() -> Self {
    #[cfg(target_os = "windows")]
    {
      Self {
        inner: Arc::new(WindowsCredentialStore::new()),
        session_cache: Arc::new(Mutex::new(HashMap::new())),
      }
    }

    #[cfg(not(target_os = "windows"))]
    {
      Self {
        inner: Arc::new(MockSecretStore::default()),
        session_cache: Arc::new(Mutex::new(HashMap::new())),
      }
    }
  }

  /// Substitutes the backing store so tests can drive failure modes — a store that loses writes,
  /// one that cannot be read back — which Credential Manager will not do on demand.
  #[cfg(test)]
  pub fn from_store(store: Arc<dyn SecretStore>) -> Self {
    Self {
      inner: store,
      session_cache: Arc::new(Mutex::new(HashMap::new())),
    }
  }

  /// In-memory storage for tests. Never reachable from a shipped Windows build, where `new()`
  /// always resolves to Windows Credential Manager.
  #[cfg(test)]
  pub fn dev_mock() -> Self {
    Self {
      inner: Arc::new(MockSecretStore::default()),
      session_cache: Arc::new(Mutex::new(HashMap::new())),
    }
  }

  /// Write a secret and confirm it reads back before reporting success.
  ///
  /// The cleanup matters as much as the verification. The credential is already in the store by
  /// the time a mismatch is detected, so returning an error without removing it left a secret
  /// that nothing referenced — invisible to the app, undeletable through it, and re-created on
  /// every retry, leaking one more each time.
  ///
  /// Living here rather than in a backend means every store gets the same guarantee, and it can
  /// be tested without touching the real Windows credential vault.
  fn store_verified(
    &self,
    kind: SecretKind,
    owner_id: &str,
    secret: &str,
  ) -> Result<String, SecretStoreError> {
    const VERIFY_ATTEMPTS: usize = 2;

    for _ in 0..VERIFY_ATTEMPTS {
      let secret_ref = self.inner.store_secret(kind, owner_id, secret)?;

      if self.inner.retrieve_secret(&secret_ref)?.as_deref() == Some(secret) {
        if let Ok(mut guard) = self.session_cache.lock() {
          guard.insert(secret_ref.clone(), secret.to_string());
        }
        return Ok(secret_ref);
      }

      // Written but unreadable: remove it so a retry cannot leave a second orphan behind.
      let _ = self.inner.delete_secret(&secret_ref);
    }

    Err(SecretStoreError::Store {
      detail: "Secret was written to secure storage but did not read back correctly, after one retry. The incomplete entry was removed. Retry the save, and check Windows Credential Manager health if it persists.".to_string(),
    })
  }

  pub fn store_digitalocean_api_token(
    &self,
    provider_id: &str,
    token: &str,
  ) -> Result<String, SecretStoreError> {
    self.store_verified(SecretKind::DigitalOceanApiToken, provider_id, token)
  }

  pub fn store_provider_api_token(
    &self,
    provider_id: &str,
    token: &str,
  ) -> Result<String, SecretStoreError> {
    self.store_digitalocean_api_token(provider_id, token)
  }

  pub fn get_digitalocean_api_token(
    &self,
    token_ref: &str,
  ) -> Result<Option<String>, SecretStoreError> {
    let resolved = self.inner.retrieve_secret(token_ref)?;
    if let Some(value) = resolved {
      if let Ok(mut guard) = self.session_cache.lock() {
        guard.insert(token_ref.to_string(), value.clone());
      }
      return Ok(Some(value));
    }

    if let Ok(mut guard) = self.session_cache.lock() {
      guard.remove(token_ref);
    }

    Ok(None)
  }

  pub fn get_provider_api_token(&self, token_ref: &str) -> Result<Option<String>, SecretStoreError> {
    self.get_digitalocean_api_token(token_ref)
  }

  pub fn delete_digitalocean_api_token(&self, token_ref: &str) -> Result<(), SecretStoreError> {
    self.inner.delete_secret(token_ref)?;
    if let Ok(mut guard) = self.session_cache.lock() {
      guard.remove(token_ref);
    }
    Ok(())
  }

  pub fn delete_provider_api_token(&self, token_ref: &str) -> Result<(), SecretStoreError> {
    self.delete_digitalocean_api_token(token_ref)
  }

  pub fn store_client_private_key(
    &self,
    node_id: &str,
    private_key: &str,
  ) -> Result<String, SecretStoreError> {
    self.store_verified(SecretKind::ClientPrivateKey, node_id, private_key)
  }

  /// Read a client private key from the credential store.
  ///
  /// A miss is reported as a miss. This used to fall back to an in-process cache when the vault
  /// returned nothing, which meant a wiped or unreadable credential store kept appearing to work
  /// for the rest of the session and only failed after a restart — hiding the very failure the
  /// caller needs to see. The token path already behaved this way; the two are now consistent.
  pub fn get_client_private_key(
    &self,
    key_ref: &str,
  ) -> Result<Option<String>, SecretStoreError> {
    let resolved = self.inner.retrieve_secret(key_ref)?;
    if let Some(value) = resolved {
      if let Ok(mut guard) = self.session_cache.lock() {
        guard.insert(key_ref.to_string(), value.clone());
      }
      return Ok(Some(value));
    }

    if let Ok(mut guard) = self.session_cache.lock() {
      guard.remove(key_ref);
    }

    Ok(None)
  }

  pub fn delete_client_private_key(&self, key_ref: &str) -> Result<(), SecretStoreError> {
    self.inner.delete_secret(key_ref)?;
    if let Ok(mut guard) = self.session_cache.lock() {
      guard.remove(key_ref);
    }
    Ok(())
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

  /// The only reference format written: `kind__owner__uuid`.
  ///
  /// A second `kind:owner:uuid` builder existed alongside this one but was never called, so it
  /// only made the format look ambiguous. Reads still accept the colon form because references
  /// created by earlier versions are recorded in existing databases — rejecting them would
  /// strand credentials that are perfectly readable.
  fn build_secret_ref(&self, kind: SecretKind, owner_id: &str) -> String {
    let owner = sanitize_segment(owner_id);
    format!("{}__{}__{}", kind.prefix(), owner, Uuid::new_v4())
  }

  /// Reject obviously malformed references before touching the credential API.
  fn validate_secret_ref(secret_ref: &str) -> Result<(), SecretStoreError> {
    let trimmed = secret_ref.trim();
    if trimmed.is_empty() {
      return Err(SecretStoreError::InvalidReference {
        detail: "Secret reference must not be empty.".to_string(),
      });
    }

    // Both the current `kind__owner__uuid` form and the older `kind:owner:uuid` form begin with a
    // known kind prefix, so checking the prefix covers both without branching on shape.
    let has_known_kind = [
      SecretKind::DigitalOceanApiToken.prefix(),
      SecretKind::ClientPrivateKey.prefix(),
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix));

    if !has_known_kind {
      return Err(SecretStoreError::InvalidReference {
        detail:
          "Secret reference does not start with a known secret kind. Expected kind__owner__uuid."
            .to_string(),
      });
    }

    Ok(())
  }

  fn make_target_name(&self, secret_ref: &str) -> String {
    format!("{}:{}", self.service_name, secret_ref)
  }

  fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
  }

  fn write_secret_ref(&self, secret_ref: &str, secret: &str) -> Result<(), SecretStoreError> {
    let target = Self::to_wide(&self.make_target_name(secret_ref));
    let username = Self::to_wide(secret_ref);
    let mut blob = secret.as_bytes().to_vec();

    let mut credential = CREDENTIALW {
      Flags: 0,
      Type: CRED_TYPE_GENERIC,
      TargetName: target.as_ptr() as *mut u16,
      Comment: null_mut(),
      LastWritten: FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
      },
      CredentialBlobSize: blob.len() as u32,
      CredentialBlob: blob.as_mut_ptr(),
      Persist: CRED_PERSIST_LOCAL_MACHINE,
      AttributeCount: 0,
      Attributes: null_mut(),
      TargetAlias: null_mut(),
      UserName: username.as_ptr() as *mut u16,
    };

    let write_ok = unsafe { CredWriteW(&mut credential as *mut CREDENTIALW, 0) };
    if write_ok == 0 {
      let code = unsafe { GetLastError() };
      return Err(SecretStoreError::Store {
        detail: format!(
          "Could not persist secret to Windows protected storage (Win32 error {code})."
        ),
      });
    }

    Ok(())
  }
}

#[cfg(target_os = "windows")]
impl SecretStore for WindowsCredentialStore {
  /// Write a secret, verifying it reads back before reporting success.
  ///
  /// The failure path matters as much as the happy path. A mismatched read-back used to return an
  /// error while leaving the written credential in place: the caller never learned the reference,
  /// so the secret existed in the vault with nothing pointing at it — invisible, undeletable
  /// through the app, and re-created on every retry. Each failed attempt leaked another one.
  ///
  /// Write the secret and return its reference.
  ///
  /// Verification and cleanup are the wrapper's job — see [`SecureStorage::store_verified`] — so
  /// the same policy applies to every backend rather than only this one.
  fn store_secret(
    &self,
    kind: SecretKind,
    owner_id: &str,
    secret: &str,
  ) -> Result<String, SecretStoreError> {
    let secret_ref = self.build_secret_ref(kind, owner_id);
    self.write_secret_ref(&secret_ref, secret)?;
    Ok(secret_ref)
  }

  fn retrieve_secret(&self, secret_ref: &str) -> Result<Option<String>, SecretStoreError> {
    Self::validate_secret_ref(secret_ref)?;
    let target = Self::to_wide(&self.make_target_name(secret_ref));
    let mut raw_credential: *mut CREDENTIALW = null_mut();

    let read_ok = unsafe {
      CredReadW(
        target.as_ptr(),
        CRED_TYPE_GENERIC,
        0,
        &mut raw_credential as *mut *mut CREDENTIALW,
      )
    };

    if read_ok == 0 {
      let code = unsafe { GetLastError() };
      if code == ERROR_NOT_FOUND {
        return Ok(None);
      }

      return Err(SecretStoreError::Retrieve {
        detail: format!(
          "Could not read secret from Windows protected storage (Win32 error {code})."
        ),
      });
    }

    let result = unsafe {
      if raw_credential.is_null() {
        None
      } else {
        let blob_ptr = (*raw_credential).CredentialBlob;
        let blob_len = (*raw_credential).CredentialBlobSize as usize;

        if blob_ptr.is_null() || blob_len == 0 {
          Some(String::new())
        } else {
          let bytes = std::slice::from_raw_parts(blob_ptr as *const u8, blob_len);
          Some(String::from_utf8_lossy(bytes).to_string())
        }
      }
    };

    if !raw_credential.is_null() {
      unsafe { CredFree(raw_credential as *mut c_void) };
    }

    Ok(result)
  }

  fn delete_secret(&self, secret_ref: &str) -> Result<(), SecretStoreError> {
    Self::validate_secret_ref(secret_ref)?;
    let target = Self::to_wide(&self.make_target_name(secret_ref));
    let delete_ok = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };

    if delete_ok == 0 {
      let code = unsafe { GetLastError() };
      if code == ERROR_NOT_FOUND {
        return Ok(());
      }

      return Err(SecretStoreError::Delete {
        detail: format!(
          "Could not delete secret from Windows protected storage (Win32 error {code})."
        ),
      });
    }

    Ok(())
  }

}

/// Plain in-memory storage, with none of Credential Manager's protection.
///
/// Reachable in exactly two situations: a test, and a non-Windows build — which Phase 1 does not
/// ship (AGENTS.md §1). Gated so a Windows release cannot contain it at all, rather than merely
/// not calling it.
#[cfg(any(test, not(target_os = "windows")))]
#[derive(Clone, Default)]
pub struct MockSecretStore {
  secrets: Arc<Mutex<HashMap<String, String>>>,
}

#[cfg(any(test, not(target_os = "windows")))]
impl MockSecretStore {
  fn build_secret_ref(kind: SecretKind, owner_id: &str) -> String {
    let owner = sanitize_segment(owner_id);
    format!("{}:{}:{}", kind.prefix(), owner, Uuid::new_v4())
  }
}

#[cfg(any(test, not(target_os = "windows")))]
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
  use std::sync::Arc;

  use super::SecureStorage;
  use super::{SecretKind, SecretStore, SecretStoreError};
  use std::collections::HashMap;

  /// Store that reads back exactly once, then loses the value.
  ///
  /// Models a credential vault wiped after a successful save: the write verifies, so a reference
  /// is legitimately handed out, and only a later lookup finds nothing. That is the case where a
  /// session-cache fallback would paper over a real failure.
  #[derive(Default)]
  struct EvaporatingStore {
    secrets: Arc<std::sync::Mutex<HashMap<String, String>>>,
  }

  impl SecretStore for EvaporatingStore {
    fn store_secret(
      &self,
      kind: SecretKind,
      owner_id: &str,
      secret: &str,
    ) -> Result<String, SecretStoreError> {
      let secret_ref = format!("{}__{}__evaporating", kind.prefix(), owner_id);
      self
        .secrets
        .lock()
        .expect("lock must work")
        .insert(secret_ref.clone(), secret.to_string());
      Ok(secret_ref)
    }

    fn retrieve_secret(&self, secret_ref: &str) -> Result<Option<String>, SecretStoreError> {
      // Removing on read means the verification read succeeds and every later read misses.
      Ok(self.secrets.lock().expect("lock must work").remove(secret_ref))
    }

    fn delete_secret(&self, secret_ref: &str) -> Result<(), SecretStoreError> {
      self.secrets.lock().expect("lock must work").remove(secret_ref);
      Ok(())
    }
  }

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

  #[test]
  fn token_lookup_does_not_fallback_to_session_cache_when_backend_missing() {
    let storage = SecureStorage::from_store(Arc::new(EvaporatingStore::default()));

    let token_ref = storage
      .store_digitalocean_api_token("provider-1", "dop_v1_secret")
      .expect("token should store");

    let token = storage
      .get_digitalocean_api_token(&token_ref)
      .expect("token lookup should succeed");

    assert!(token.is_none());
  }

  /// Private keys must report a vault miss as a miss, exactly like tokens.
  ///
  /// This used to fall back to an in-process cache, so a wiped credential store kept appearing to
  /// work until the app restarted — masking the failure at the point it could still be diagnosed.
  #[test]
  fn private_key_lookup_does_not_fallback_to_session_cache_when_backend_missing() {
    let storage = SecureStorage::from_store(Arc::new(EvaporatingStore::default()));

    let key_ref = storage
      .store_client_private_key("node-1", "wg-private-key")
      .expect("private key should store");

    let key = storage
      .get_client_private_key(&key_ref)
      .expect("private key lookup should succeed");

    assert!(
      key.is_none(),
      "a missing credential must not be served from the session cache"
    );
  }

  /// Store that always writes but never reads back, and counts what it was asked to delete.
  ///
  /// Models the failure the retry logic exists for: the credential lands in the vault but the
  /// verification read does not see it.
  #[derive(Default)]
  struct UnverifiableStore {
    written: Arc<std::sync::Mutex<Vec<String>>>,
    deleted: Arc<std::sync::Mutex<Vec<String>>>,
  }

  impl SecretStore for UnverifiableStore {
    fn store_secret(
      &self,
      kind: SecretKind,
      owner_id: &str,
      _secret: &str,
    ) -> Result<String, SecretStoreError> {
      let secret_ref = format!(
        "{}__{}__{}",
        kind.prefix(),
        owner_id,
        self.written.lock().expect("lock must work").len()
      );
      self
        .written
        .lock()
        .expect("lock must work")
        .push(secret_ref.clone());
      Ok(secret_ref)
    }

    fn retrieve_secret(&self, _secret_ref: &str) -> Result<Option<String>, SecretStoreError> {
      Ok(None)
    }

    fn delete_secret(&self, secret_ref: &str) -> Result<(), SecretStoreError> {
      self
        .deleted
        .lock()
        .expect("lock must work")
        .push(secret_ref.to_string());
      Ok(())
    }
  }

  /// A write that cannot be verified must not leave an orphan behind.
  ///
  /// The credential is already in the vault by the time verification fails. Returning an error
  /// without cleaning up left a secret that nothing referenced — invisible to the app,
  /// undeletable through it, and re-created on every retry.
  #[test]
  fn unverifiable_write_is_cleaned_up_rather_than_orphaned() {
    let deleted = Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = UnverifiableStore {
      written: Arc::new(std::sync::Mutex::new(Vec::new())),
      deleted: Arc::clone(&deleted),
    };
    let written = Arc::clone(&store.written);
    let storage = SecureStorage::from_store(Arc::new(store));

    let result = storage.store_digitalocean_api_token("provider-1", "dop_v1_secret");

    assert!(result.is_err(), "an unverifiable write must not report success");

    let written = written.lock().expect("lock must work").clone();
    let deleted = deleted.lock().expect("lock must work").clone();

    assert!(!written.is_empty(), "the store should have been attempted");
    assert_eq!(
      written, deleted,
      "every reference written must also be deleted, leaving no orphan"
    );
  }
}