use defguard_wireguard_rs::key::Key;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct WireGuardKeypair {
  pub private_key: String,
  pub public_key: String,
}

#[derive(Debug, Error)]
pub enum KeygenError {
  #[error("WireGuard private key material is invalid")]
  InvalidPrivateKey,
  #[error("WireGuard key derivation produced an empty public key")]
  EmptyPublicKey,
}

#[derive(Debug, Error)]
pub enum WireGuardConfigError {
  #[error("WireGuard private key cannot be empty")]
  EmptyPrivateKey,
  #[error("WireGuard public key cannot be empty")]
  EmptyPublicKey,
  #[error("WireGuard public key '{value}' is invalid")]
  InvalidPublicKey { value: String },
  #[error("WireGuard interface name cannot be empty")]
  EmptyInterfaceName,
  #[error("WireGuard server interface CIDR cannot be empty")]
  EmptyServerCidr,
  #[error("WireGuard client CIDR cannot be empty")]
  EmptyClientCidr,
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

pub fn normalize_wireguard_key_material(value: &str) -> String {
  value.trim().replace('\r', "").replace('\n', "")
}

pub fn derive_public_key(private_key: &str) -> Result<String, KeygenError> {
  let private_key = normalize_wireguard_key_material(private_key);
  if private_key.trim().is_empty() {
    return Err(KeygenError::InvalidPrivateKey);
  }

  let key = Key::try_from(private_key.as_str()).map_err(|_| KeygenError::InvalidPrivateKey)?;
  let public_key = key.public_key().to_string();

  if public_key.trim().is_empty() {
    return Err(KeygenError::EmptyPublicKey);
  }

  Ok(public_key)
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

pub fn build_server_interface_config(
  private_key: &str,
  interface_name: &str,
  server_interface_cidr: &str,
  listen_port: u16,
  client_cidr: &str,
) -> Result<String, WireGuardConfigError> {
  let private_key = normalize_wireguard_key_material(private_key);
  if private_key.trim().is_empty() {
    return Err(WireGuardConfigError::EmptyPrivateKey);
  }

  let interface_name = interface_name.trim();
  if interface_name.is_empty() {
    return Err(WireGuardConfigError::EmptyInterfaceName);
  }

  let server_interface_cidr = server_interface_cidr.trim();
  if server_interface_cidr.is_empty() {
    return Err(WireGuardConfigError::EmptyServerCidr);
  }

  let client_cidr = client_cidr.trim();
  if client_cidr.is_empty() {
    return Err(WireGuardConfigError::EmptyClientCidr);
  }

  Ok(format!(
    "[Interface]\nAddress = {}\nListenPort = {}\nPrivateKey = {}\nSaveConfig = false\n\nPostUp = iptables -A FORWARD -i {} -j ACCEPT; iptables -A FORWARD -o {} -j ACCEPT; iptables -t nat -A POSTROUTING -s {} -j MASQUERADE\nPostDown = iptables -D FORWARD -i {} -j ACCEPT; iptables -D FORWARD -o {} -j ACCEPT; iptables -t nat -D POSTROUTING -s {} -j MASQUERADE\n",
    server_interface_cidr,
    listen_port,
    private_key,
    interface_name,
    interface_name,
    client_cidr,
    interface_name,
    interface_name,
    client_cidr,
  ))
}

pub fn build_client_peer_config_fragment(
  public_key: &str,
  client_address_cidr: &str,
) -> Result<String, WireGuardConfigError> {
  let public_key = validate_public_key(public_key)?;
  let client_address_cidr = client_address_cidr.trim();
  if client_address_cidr.is_empty() {
    return Err(WireGuardConfigError::EmptyClientCidr);
  }

  Ok(format!(
    "[Peer]\nPublicKey = {}\nAllowedIPs = {}\n",
    public_key, client_address_cidr,
  ))
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

  #[test]
  fn build_server_interface_config_contains_valid_wireguard_blocks() {
    let keypair = generate_server_keypair().expect("keypair generation should succeed");
    let config = build_server_interface_config(
      &format!("\n{}\n", keypair.private_key),
      "wg0",
      "10.66.0.1/24",
      51820,
      "10.66.0.2/32",
    )
    .expect("server config should render");

    assert!(config.contains("[Interface]"));
    assert!(config.contains("Address = 10.66.0.1/24"));
    assert!(config.contains("ListenPort = 51820"));
    assert!(config.contains(&format!("PrivateKey = {}", keypair.private_key)));
    assert!(config.contains("PostUp ="));
    assert!(config.contains("PostDown ="));
  }

  #[test]
  fn build_client_peer_config_fragment_uses_valid_public_key() {
    let keypair = generate_server_keypair().expect("keypair generation should succeed");
    let fragment = build_client_peer_config_fragment(&keypair.public_key, "10.66.0.2/32")
      .expect("peer config should render");

    assert!(fragment.contains("[Peer]"));
    assert!(fragment.contains(&format!("PublicKey = {}", keypair.public_key)));
    assert!(fragment.contains("AllowedIPs = 10.66.0.2/32"));
  }
}
