use std::collections::HashSet;
use std::net::Ipv4Addr;

use thiserror::Error;

pub const DEFAULT_WIREGUARD_PORT: u16 = 51820;
pub const DEFAULT_SERVER_INTERFACE_CIDR: &str = "10.66.0.1/24";
pub const DEFAULT_INTERFACE_NAME: &str = "wg0";
pub const READINESS_MARKER_PATH: &str = "/var/lib/goatstare/bootstrap-ready.json";

/// How many peers every new server is provisioned with.
///
/// Fixed at create time and never changeable afterwards: AGENTS.md §2 leaves no channel to register
/// a peer once the node exists, so the only way to gain one is to bake it up front. Slot 0 is the
/// Windows desktop; the rest are spare capacity for phones and other devices.
///
/// Five costs roughly 450 bytes of cloud-init against EC2's 16 KiB user-data ceiling, which is why
/// slots are simply always provisioned rather than being a question asked during node creation.
pub const DEFAULT_PEER_SLOTS: usize = 5;

/// One entry in the server's `[Peer]` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapPeer {
  pub public_key: String,
  /// The single address this peer is allowed to use, as `a.b.c.d/32`.
  pub address_cidr: String,
}

#[derive(Debug, Clone)]
pub struct BootstrapParams {
  pub server_endpoint_ip: String,
  pub listen_port: u16,
  pub server_interface_cidr: String,
  pub interface_name: String,
  /// Every peer the server will ever have, baked into its config at render time.
  ///
  /// All key material is generated on the Windows host before the node is created, so the server
  /// boots already knowing every peer. Nothing is pushed to it afterwards, and nothing can be.
  pub peers: Vec<BootstrapPeer>,
}

impl Default for BootstrapParams {
  fn default() -> Self {
    Self {
      server_endpoint_ip: "203.0.113.10".to_string(),
      listen_port: DEFAULT_WIREGUARD_PORT,
      server_interface_cidr: DEFAULT_SERVER_INTERFACE_CIDR.to_string(),
      interface_name: DEFAULT_INTERFACE_NAME.to_string(),
      // Intentionally empty: a placeholder peer would silently produce a server that nothing can
      // connect to. Callers must supply real generated peers.
      peers: Vec::new(),
    }
  }
}

#[derive(Debug, Clone)]
pub struct BootstrapArtifact {
  pub init_payload: String,
  /// Where the bootstrap script writes its own completion marker.
  ///
  /// Recorded so the payload and the Docker health check agree on the path. **Nothing in the app
  /// reads this file** — see AGENTS.md §2. It is a self-check and a debugging aid on the server.
  pub readiness_marker_path: String,
  pub output_contract_preview: BootstrapOutputContract,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapOutputContract {
  pub server_endpoint_ip: String,
  pub server_public_key: String,
  pub listen_port: u16,
}

#[derive(Debug, Error)]
pub enum BootstrapError {
  #[error("Invalid bootstrap parameter '{field}': {detail}")]
  InvalidParameter { field: &'static str, detail: String },
}

/// Assign one `/32` per peer inside the server's own subnet.
///
/// Derived from the server's interface CIDR rather than hardcoded, so the subnet stays a single
/// source of truth. The network and broadcast addresses are never handed out, and the server keeps
/// the address it already holds.
pub fn allocate_peer_addresses(
  server_interface_cidr: &str,
  count: usize,
) -> Result<Vec<String>, BootstrapError> {
  let (server_addr, prefix) = parse_ipv4_cidr(server_interface_cidr, "server_interface_cidr")?;

  if prefix > 30 {
    return Err(BootstrapError::InvalidParameter {
      field: "server_interface_cidr",
      detail: format!("A /{prefix} subnet has no room for peers. Use /30 or wider."),
    });
  }

  let mask = mask_for_prefix(prefix);
  let server = u32::from(server_addr);
  let network = server & mask;
  let broadcast = network | !mask;

  // Hosts between the network and broadcast addresses, less the one the server occupies. Computed
  // before allocating so an impossible request fails immediately rather than after a long walk.
  let mut capacity = (broadcast - network - 1) as usize;
  if server > network && server < broadcast {
    capacity -= 1;
  }

  if capacity < count {
    return Err(BootstrapError::InvalidParameter {
      field: "server_interface_cidr",
      detail: format!(
        "{server_interface_cidr} has room for {capacity} peers, but {count} were requested."
      ),
    });
  }

  let mut addresses = Vec::with_capacity(count);
  let mut host = network + 1;
  while addresses.len() < count && host < broadcast {
    if host != server {
      addresses.push(format!("{}/32", Ipv4Addr::from(host)));
    }
    host += 1;
  }

  Ok(addresses)
}

/// `10.66.0.1/24` -> `10.66.0.0/24`.
///
/// The server's NAT rule masquerades this range. It used to masquerade a single client `/32`, which
/// silently denied every peer but the first any internet access at all (AGENTS.md §12).
pub fn subnet_of(interface_cidr: &str) -> Result<String, BootstrapError> {
  let (addr, prefix) = parse_ipv4_cidr(interface_cidr, "server_interface_cidr")?;
  let network = u32::from(addr) & mask_for_prefix(prefix);
  Ok(format!("{}/{prefix}", Ipv4Addr::from(network)))
}

fn mask_for_prefix(prefix: u8) -> u32 {
  if prefix == 0 {
    0
  } else {
    u32::MAX << (32 - prefix)
  }
}

pub(crate) fn validate_bootstrap_params(params: &BootstrapParams) -> Result<(), BootstrapError> {
  validate_ipv4(&params.server_endpoint_ip, "server_endpoint_ip")?;
  validate_ipv4_cidr(&params.server_interface_cidr, "server_interface_cidr")?;
  validate_peers(&params.peers)?;
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

/// Every peer must be present and well-formed *before* the node is created.
///
/// A server rendered without a `[Peer]` block accepts no handshakes, and because the app never
/// contacts the server after create there is no later opportunity to add one. Failing here is what
/// keeps that from reaching a provider.
///
/// Duplicates are rejected rather than deduplicated: two peers sharing a key or an address means a
/// caller lost track of which device is which, and quietly collapsing them would hand two devices
/// the same identity.
fn validate_peers(peers: &[BootstrapPeer]) -> Result<(), BootstrapError> {
  if peers.is_empty() {
    return Err(BootstrapError::InvalidParameter {
      field: "peers",
      detail:
        "At least one peer is required so the server boots with a [Peer] block. Generate the client keypairs before rendering bootstrap."
          .to_string(),
    });
  }

  let mut seen_keys = HashSet::new();
  let mut seen_addresses = HashSet::new();

  for peer in peers {
    if peer.public_key.trim().is_empty() {
      return Err(BootstrapError::InvalidParameter {
        field: "peers",
        detail: "A peer has an empty public key.".to_string(),
      });
    }

    crate::backend::crypto::keypair::validate_public_key(&peer.public_key).map_err(|source| {
      BootstrapError::InvalidParameter {
        field: "peers",
        detail: format!("A peer public key is not valid WireGuard key material: {source}"),
      }
    })?;

    validate_ipv4_cidr(&peer.address_cidr, "peers")?;

    if !seen_keys.insert(peer.public_key.trim().to_string()) {
      return Err(BootstrapError::InvalidParameter {
        field: "peers",
        detail: "Two peers share a public key.".to_string(),
      });
    }

    if !seen_addresses.insert(peer.address_cidr.trim().to_string()) {
      return Err(BootstrapError::InvalidParameter {
        field: "peers",
        detail: format!("Two peers share the address {}.", peer.address_cidr),
      });
    }
  }

  Ok(())
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
  parse_ipv4_cidr(value, field).map(|_| ())
}

/// The one place a CIDR string is taken apart, so validation and address arithmetic cannot disagree.
fn parse_ipv4_cidr(value: &str, field: &'static str) -> Result<(Ipv4Addr, u8), BootstrapError> {
  let parts: Vec<&str> = value.split('/').collect();
  if parts.len() != 2 {
    return Err(BootstrapError::InvalidParameter {
      field,
      detail: "Expected CIDR format a.b.c.d/prefix.".to_string(),
    });
  }

  let address = parts[0]
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

  Ok((address, prefix))
}
