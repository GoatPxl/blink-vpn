use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_WIREGUARD_PORT: u16 = 51820;
pub const DEFAULT_SERVER_INTERFACE_CIDR: &str = "10.66.0.1/24";
pub const DEFAULT_CLIENT_CIDR: &str = "10.66.0.2/32";
pub const DEFAULT_INTERFACE_NAME: &str = "wg0";
pub const READINESS_MARKER_PATH: &str = "/var/lib/goatstare/bootstrap-ready.json";

#[derive(Debug, Clone)]
pub struct BootstrapParams {
  pub server_endpoint_ip: String,
  pub listen_port: u16,
  pub server_interface_cidr: String,
  pub client_cidr: String,
  pub interface_name: String,
}

impl Default for BootstrapParams {
  fn default() -> Self {
    Self {
      server_endpoint_ip: "203.0.113.10".to_string(),
      listen_port: DEFAULT_WIREGUARD_PORT,
      server_interface_cidr: DEFAULT_SERVER_INTERFACE_CIDR.to_string(),
      client_cidr: DEFAULT_CLIENT_CIDR.to_string(),
      interface_name: DEFAULT_INTERFACE_NAME.to_string(),
    }
  }
}

#[derive(Debug, Clone)]
pub struct BootstrapArtifact {
  pub cloud_init_user_data: String,
  pub readiness_marker_path: String,
  pub server_public_key_path: String,
  pub output_contract_preview: BootstrapOutputContract,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapOutputContract {
  pub server_endpoint_ip: String,
  pub server_public_key: String,
  pub listen_port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadinessMarker {
  pub status: String,
  pub server_public_key: String,
  pub listen_port: u16,
}

#[derive(Debug, Error)]
pub enum BootstrapError {
  #[error("Invalid bootstrap parameter '{field}': {detail}")]
  InvalidParameter { field: &'static str, detail: String },
  #[error("Bootstrap marker parsing failed: {detail}")]
  MarkerDecode {
    detail: String,
    #[source]
    source: serde_json::Error,
  },
}

pub(crate) fn validate_bootstrap_params(params: &BootstrapParams) -> Result<(), BootstrapError> {
  validate_ipv4(&params.server_endpoint_ip, "server_endpoint_ip")?;
  validate_ipv4_cidr(&params.server_interface_cidr, "server_interface_cidr")?;
  validate_ipv4_cidr(&params.client_cidr, "client_cidr")?;
  validate_listen_port(params.listen_port)?;

  if params.interface_name.trim().is_empty() {
    return Err(BootstrapError::InvalidParameter {
      field: "interface_name",
      detail: "Interface name cannot be empty.".to_string(),
    });
  }

  if !params
    .interface_name
    .chars()
    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
  {
    return Err(BootstrapError::InvalidParameter {
      field: "interface_name",
      detail: "Interface name contains unsupported characters.".to_string(),
    });
  }

  Ok(())
}

pub(crate) fn build_output_contract_from_marker(
  endpoint_ip: &str,
  marker_json: &str,
) -> Result<BootstrapOutputContract, BootstrapError> {
  validate_ipv4(endpoint_ip, "server_endpoint_ip")?;

  let marker: ReadinessMarker = serde_json::from_str(marker_json).map_err(|source| {
    BootstrapError::MarkerDecode {
      detail:
        "Readiness marker JSON was invalid. Verify cloud-init completed and marker format is unchanged."
          .to_string(),
      source,
    }
  })?;

  if marker.status != "ready" {
    return Err(BootstrapError::InvalidParameter {
      field: "readiness_marker.status",
      detail:
        "Marker status is not ready yet. Wait for cloud-init completion before consuming output contract."
          .to_string(),
    });
  }

  validate_listen_port(marker.listen_port)?;

  if marker.server_public_key.trim().is_empty() {
    return Err(BootstrapError::InvalidParameter {
      field: "readiness_marker.server_public_key",
      detail: "Server public key in marker is empty.".to_string(),
    });
  }

  Ok(BootstrapOutputContract {
    server_endpoint_ip: endpoint_ip.to_string(),
    server_public_key: marker.server_public_key,
    listen_port: marker.listen_port,
  })
}

fn validate_listen_port(port: u16) -> Result<(), BootstrapError> {
  if (1024..=65535).contains(&port) {
    return Ok(());
  }

  Err(BootstrapError::InvalidParameter {
    field: "listen_port",
    detail: "Port must be in range 1024..=65535 for non-privileged WireGuard binding."
      .to_string(),
  })
}

fn validate_ipv4(value: &str, field: &'static str) -> Result<(), BootstrapError> {
  value.parse::<Ipv4Addr>().map_err(|_| BootstrapError::InvalidParameter {
    field,
    detail: "Expected a valid IPv4 address string.".to_string(),
  })?;
  Ok(())
}

fn validate_ipv4_cidr(value: &str, field: &'static str) -> Result<(), BootstrapError> {
  let parts: Vec<&str> = value.split('/').collect();
  if parts.len() != 2 {
    return Err(BootstrapError::InvalidParameter {
      field,
      detail: "Expected CIDR format a.b.c.d/prefix.".to_string(),
    });
  }

  parts[0]
    .parse::<Ipv4Addr>()
    .map_err(|_| BootstrapError::InvalidParameter {
      field,
      detail: "CIDR has an invalid IPv4 address.".to_string(),
    })?;

  let prefix = parts[1]
    .parse::<u8>()
    .map_err(|_| BootstrapError::InvalidParameter {
      field,
      detail: "CIDR prefix must be an integer.".to_string(),
    })?;

  if prefix > 32 {
    return Err(BootstrapError::InvalidParameter {
      field,
      detail: "CIDR prefix must be <= 32 for IPv4.".to_string(),
    });
  }

  Ok(())
}
