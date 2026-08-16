use defguard_wireguard_rs::key::Key;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct WireGuardKeypair {
  pub private_key: String,
  pub public_key: String,
}

#[derive(Debug, Error)]
pub enum KeygenError {
  #[error("WireGuard key derivation produced an empty public key")]
  EmptyPublicKey,
}

#[derive(Debug, Error)]
pub enum WireGuardConfigError {
  #[error("WireGuard public key cannot be empty")]
  EmptyPublicKey,
  #[error("WireGuard public key '{value}' is invalid")]
  InvalidPublicKey { value: String },
}

fn generate_keypair() -> Result<WireGuardKeypair, KeygenError> {
  let private = Key::generate();
  let public = private.public_key();
  let public_str = public.to_string();

  if public_str.trim().is_empty() {
    return Err(KeygenError::EmptyPublicKey);
  }

  Ok(WireGuardKeypair {
    private_key: private.to_string(),
    public_key: public_str,
  })
}

/// Server keypair, generated on the Windows host before the node is created.
///
/// The private key is baked into the bootstrap payload; it is never generated remotely.
pub fn generate_server_keypair() -> Result<WireGuardKeypair, KeygenError> {
  generate_keypair()
}

/// Client keypair, generated on the Windows host before the node is created.
///
/// This must happen *before* the bootstrap render, because the client's public key ships inside
/// the server's `[Peer]` block. The private key never leaves this machine.
pub fn generate_client_keypair() -> Result<WireGuardKeypair, KeygenError> {
  generate_keypair()
}

pub fn normalize_wireguard_key_material(value: &str) -> String {
  value.trim().replace('\r', "").replace('\n', "")
}

pub fn validate_public_key(public_key: &str) -> Result<String, WireGuardConfigError> {
  let public_key = normalize_wireguard_key_material(public_key);
  if public_key.trim().is_empty() {
    return Err(WireGuardConfigError::EmptyPublicKey);
  }

  let _ = Key::try_from(public_key.as_str()).map_err(|_| WireGuardConfigError::InvalidPublicKey {
    value: public_key.clone(),
  })?;

  Ok(public_key)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn generate_server_keypair_produces_valid_base64_keys() {
    let keypair = generate_server_keypair().expect("keypair generation should succeed");

    assert!(!keypair.private_key.trim().is_empty());
    assert!(!keypair.public_key.trim().is_empty());
    // WireGuard Curve25519 keys are 32 bytes = 44 base64 chars including padding
    assert_eq!(keypair.private_key.len(), 44, "private key should be 44 base64 chars");
    assert_eq!(keypair.public_key.len(), 44, "public key should be 44 base64 chars");
  }

  #[test]
  fn each_call_produces_distinct_keypairs() {
    let a = generate_server_keypair().expect("first keypair should succeed");
    let b = generate_server_keypair().expect("second keypair should succeed");
    assert_ne!(a.private_key, b.private_key);
    assert_ne!(a.public_key, b.public_key);
  }

  /// Guards the one consumer this module still has: bootstrap rejects a malformed client public
  /// key before it can reach a provider and produce a server nothing can hand-shake with.
  #[test]
  fn validate_public_key_normalizes_wrapping_and_rejects_junk() {
    let keypair = generate_server_keypair().expect("keypair generation should succeed");

    assert_eq!(
      validate_public_key(&format!("\r\n{}\n", keypair.public_key))
        .expect("a wrapped key should normalize"),
      keypair.public_key
    );

    assert!(matches!(
      validate_public_key("   "),
      Err(WireGuardConfigError::EmptyPublicKey)
    ));
    assert!(matches!(
      validate_public_key("not-a-key"),
      Err(WireGuardConfigError::InvalidPublicKey { .. })
    ));
  }
}
