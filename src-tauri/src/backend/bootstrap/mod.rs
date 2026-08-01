use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_WIREGUARD_PORT: u16 = 51820;
pub const DEFAULT_SERVER_INTERFACE_CIDR: &str = "10.66.0.1/24";
pub const DEFAULT_CLIENT_CIDR: &str = "10.66.0.2/32";
pub const DEFAULT_INTERFACE_NAME: &str = "wg0";
pub const READINESS_MARKER_PATH: &str = "/var/lib/goatstare/bootstrap-ready.json";

#[derive(Clone)]
pub struct Bootstrap;

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

impl Bootstrap {
  pub fn new() -> Self {
    Self
  }

  pub fn validate_environment(&self) -> Result<(), String> {
    Ok(())
  }

  pub fn render_cloud_init(
    &self,
    params: &BootstrapParams,
  ) -> Result<BootstrapArtifact, BootstrapError> {
    validate_bootstrap_params(params)?;

    let cloud_init_user_data = build_cloud_init_user_data(params);
    let output_contract_preview = BootstrapOutputContract {
      server_endpoint_ip: params.server_endpoint_ip.clone(),
      server_public_key: "<resolved-from-readiness-marker>".to_string(),
      listen_port: params.listen_port,
    };

    Ok(BootstrapArtifact {
      cloud_init_user_data,
      readiness_marker_path: READINESS_MARKER_PATH.to_string(),
      server_public_key_path: "/etc/wireguard/server_public.key".to_string(),
      output_contract_preview,
    })
  }

  pub fn build_output_contract_from_marker(
    &self,
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
}

fn validate_bootstrap_params(params: &BootstrapParams) -> Result<(), BootstrapError> {
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

fn build_cloud_init_user_data(params: &BootstrapParams) -> String {
  let script = render_bootstrap_script(params);
  let escaped_script = indent_for_yaml_block(&script, 6);

  format!(
    r#"#cloud-config
package_update: true
package_upgrade: false
packages:
  - wireguard
  - iptables-persistent
write_files:
  - path: /etc/sysctl.d/99-goatstare-vpn.conf
    permissions: '0644'
    content: |
      net.ipv4.ip_forward=1
  - path: /usr/local/bin/goatstare-bootstrap.sh
    permissions: '0700'
    owner: root:root
    content: |
{escaped_script}
runcmd:
  - ["bash", "-lc", "/usr/local/bin/goatstare-bootstrap.sh"]
final_message: "goatstare bootstrap completed"
"#
  )
}

fn indent_for_yaml_block(content: &str, spaces: usize) -> String {
  let prefix = " ".repeat(spaces);
  content
    .lines()
    .map(|line| format!("{prefix}{line}"))
    .collect::<Vec<_>>()
    .join("\n")
}

fn render_bootstrap_script(params: &BootstrapParams) -> String {
  format!(
    r#"#!/usr/bin/env bash
set -euo pipefail

WG_IFACE=\"{iface}\"
WG_PORT=\"{port}\"
SERVER_CIDR=\"{server_cidr}\"
CLIENT_CIDR=\"{client_cidr}\"
MARKER_PATH=\"{marker}\"

umask 077
mkdir -p /etc/wireguard /var/lib/goatstare

if [ ! -f /etc/wireguard/server_private.key ] || [ ! -f /etc/wireguard/server_public.key ]; then
  wg genkey | tee /etc/wireguard/server_private.key | wg pubkey > /etc/wireguard/server_public.key
fi

SERVER_PRIVATE_KEY=$(cat /etc/wireguard/server_private.key)
SERVER_PUBLIC_KEY=$(cat /etc/wireguard/server_public.key)

cat > /etc/wireguard/${{WG_IFACE}}.conf <<EOF
[Interface]
Address = ${{SERVER_CIDR}}
ListenPort = ${{WG_PORT}}
PrivateKey = ${{SERVER_PRIVATE_KEY}}
SaveConfig = false

PostUp = iptables -A FORWARD -i ${{WG_IFACE}} -j ACCEPT; iptables -A FORWARD -o ${{WG_IFACE}} -j ACCEPT; iptables -t nat -A POSTROUTING -s ${{CLIENT_CIDR}} -j MASQUERADE
PostDown = iptables -D FORWARD -i ${{WG_IFACE}} -j ACCEPT; iptables -D FORWARD -o ${{WG_IFACE}} -j ACCEPT; iptables -t nat -D POSTROUTING -s ${{CLIENT_CIDR}} -j MASQUERADE
EOF

sysctl --system

iptables -N GOATSTARE-INPUT 2>/dev/null || true
iptables -F GOATSTARE-INPUT
iptables -A GOATSTARE-INPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -A GOATSTARE-INPUT -i lo -j ACCEPT
iptables -A GOATSTARE-INPUT -p tcp --dport 22 -j ACCEPT
iptables -A GOATSTARE-INPUT -p udp --dport ${{WG_PORT}} -j ACCEPT
iptables -A GOATSTARE-INPUT -j DROP
iptables -C INPUT -j GOATSTARE-INPUT 2>/dev/null || iptables -I INPUT 1 -j GOATSTARE-INPUT

systemctl enable --now wg-quick@${{WG_IFACE}}

cat > ${{MARKER_PATH}} <<EOF
{{"status":"ready","server_public_key":"${{SERVER_PUBLIC_KEY}}","listen_port":${{WG_PORT}}}}
EOF
chmod 600 ${{MARKER_PATH}}
"#,
    iface = params.interface_name,
    port = params.listen_port,
    server_cidr = params.server_interface_cidr,
    client_cidr = params.client_cidr,
    marker = READINESS_MARKER_PATH,
  )
}

#[cfg(test)]
mod tests {
  use super::{
    Bootstrap, BootstrapOutputContract, BootstrapParams, DEFAULT_CLIENT_CIDR,
    DEFAULT_INTERFACE_NAME, DEFAULT_SERVER_INTERFACE_CIDR, DEFAULT_WIREGUARD_PORT,
    READINESS_MARKER_PATH,
  };

  #[test]
  fn renders_cloud_init_with_wireguard_nat_and_marker() {
    let bootstrap = Bootstrap::new();
    let params = BootstrapParams {
      server_endpoint_ip: "198.51.100.9".to_string(),
      listen_port: DEFAULT_WIREGUARD_PORT,
      server_interface_cidr: DEFAULT_SERVER_INTERFACE_CIDR.to_string(),
      client_cidr: DEFAULT_CLIENT_CIDR.to_string(),
      interface_name: DEFAULT_INTERFACE_NAME.to_string(),
    };

    let artifact = bootstrap
      .render_cloud_init(&params)
      .expect("cloud-init render should succeed");

    assert!(artifact.cloud_init_user_data.contains("#cloud-config"));
    assert!(artifact.cloud_init_user_data.contains("- wireguard"));
    assert!(artifact.cloud_init_user_data.contains("net.ipv4.ip_forward=1"));
    assert!(artifact.cloud_init_user_data.contains("iptables -t nat -A POSTROUTING"));
    assert!(artifact.cloud_init_user_data.contains("wg genkey"));
    assert!(artifact.cloud_init_user_data.contains("wg-quick@${WG_IFACE}"));
    assert!(artifact.cloud_init_user_data.contains(READINESS_MARKER_PATH));
    assert!(artifact.cloud_init_user_data.contains("--dport ${WG_PORT}"));
  }

  #[test]
  fn rejects_invalid_cidr_and_port() {
    let bootstrap = Bootstrap::new();
    let params = BootstrapParams {
      server_endpoint_ip: "198.51.100.9".to_string(),
      listen_port: 80,
      server_interface_cidr: "10.66.0.1/99".to_string(),
      client_cidr: "not-cidr".to_string(),
      interface_name: DEFAULT_INTERFACE_NAME.to_string(),
    };

    let err = bootstrap
      .render_cloud_init(&params)
      .expect_err("invalid params must be rejected");
    assert!(err.to_string().contains("Invalid bootstrap parameter"));
  }

  #[test]
  fn builds_output_contract_from_readiness_marker() {
    let bootstrap = Bootstrap::new();
    let marker_json =
      "{\"status\":\"ready\",\"server_public_key\":\"pubkey123\",\"listen_port\":51820}";

    let contract = bootstrap
      .build_output_contract_from_marker("203.0.113.8", marker_json)
      .expect("marker parsing should succeed");

    assert_eq!(
      contract,
      BootstrapOutputContract {
        server_endpoint_ip: "203.0.113.8".to_string(),
        server_public_key: "pubkey123".to_string(),
        listen_port: 51820,
      }
    );
  }

  #[test]
  fn readiness_status_must_be_ready() {
    let bootstrap = Bootstrap::new();
    let marker_json =
      "{\"status\":\"running\",\"server_public_key\":\"pubkey123\",\"listen_port\":51820}";

    let err = bootstrap
      .build_output_contract_from_marker("203.0.113.8", marker_json)
      .expect_err("non-ready status should fail");

    assert!(err.to_string().contains("not ready"));
  }
}