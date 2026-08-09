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

pub fn generate_server_keypair() -> Result<WireGuardKeypair, KeygenError> {
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
}
