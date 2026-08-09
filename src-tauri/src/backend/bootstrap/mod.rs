mod contract;
mod core;
mod renderers;

pub use contract::{
  BootstrapArtifact, BootstrapError, BootstrapOutputContract, BootstrapParams,
  DEFAULT_CLIENT_CIDR, DEFAULT_INTERFACE_NAME, DEFAULT_SERVER_INTERFACE_CIDR,
  DEFAULT_WIREGUARD_PORT, READINESS_MARKER_PATH,
};
pub use contract::ReadinessMarker;
pub use core::Bootstrap;

#[cfg(test)]
mod tests {
  use super::{
    Bootstrap, BootstrapOutputContract, BootstrapParams, DEFAULT_CLIENT_CIDR,
    DEFAULT_INTERFACE_NAME, DEFAULT_SERVER_INTERFACE_CIDR, DEFAULT_WIREGUARD_PORT,
    READINESS_MARKER_PATH,
  };
  use crate::backend::crypto::keypair::generate_server_keypair;

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
    let keypair = generate_server_keypair().expect("keypair should generate");

    let artifact = bootstrap
      .render_cloud_init(&params, &keypair.private_key, &keypair.public_key)
      .expect("cloud-init render should succeed");

    assert!(artifact.init_payload.contains("#cloud-config"));
    assert!(artifact.init_payload.contains("- wireguard"));
    assert!(artifact.init_payload.contains("- wireguard-tools"));
    assert!(artifact.init_payload.contains("- iptables"));
    assert!(artifact.init_payload.contains("net.ipv4.ip_forward=1"));
    assert!(artifact.init_payload.contains("net.ipv4.conf.all.forwarding=1"));
    assert!(artifact.init_payload.contains("net.ipv4.conf.default.forwarding=1"));
    assert!(artifact.init_payload.contains("goatstare.bootstrap.stage=${stage}"));
    assert!(!artifact.init_payload.contains("wg genkey"));
    assert!(artifact.init_payload.contains("keypair-missing"));
    assert!(artifact.init_payload.contains(&keypair.public_key));
    assert!(artifact.init_payload.contains("wg-quick@${WG_IFACE}"));
    assert!(artifact.init_payload.contains(READINESS_MARKER_PATH));
    assert!(artifact.init_payload.contains("--dport ${WG_PORT}"));
    assert!(artifact.init_payload.contains("package_reboot_if_required: false"));
  }

  #[test]
  fn renders_docker_init_with_ubuntu_prereqs_and_marker() {
    let bootstrap = Bootstrap::new();
    let params = BootstrapParams {
      server_endpoint_ip: "198.51.100.9".to_string(),
      listen_port: DEFAULT_WIREGUARD_PORT,
      server_interface_cidr: DEFAULT_SERVER_INTERFACE_CIDR.to_string(),
      client_cidr: DEFAULT_CLIENT_CIDR.to_string(),
      interface_name: DEFAULT_INTERFACE_NAME.to_string(),
    };

    let artifact = bootstrap
      .render_docker_init(&params)
      .expect("docker init render should succeed");

    assert!(artifact.init_payload.contains("#!/usr/bin/env bash"));
    assert!(artifact.init_payload.contains("apt-get update"));
    assert!(artifact.init_payload.contains("apt-get install -y --no-install-recommends wireguard-tools iproute2 iptables"));
    assert!(artifact.init_payload.contains("/etc/sysctl.d/99-goatstare-vpn.conf"));
    assert!(artifact.init_payload.contains("goatstare.bootstrap.stage=${stage}"));
    assert!(!artifact.init_payload.contains("wg genkey"));
    assert!(artifact.init_payload.contains("keypair-missing"));
    assert!(artifact.init_payload.contains("wg-quick up \"${WG_IFACE}\""));
    assert!(!artifact.init_payload.contains("systemctl enable --now"));
    assert!(artifact.init_payload.contains(READINESS_MARKER_PATH));
    assert!(!artifact.init_payload.contains("#cloud-config"));
    assert!(!artifact.init_payload.contains("apk add"));
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
      .render_cloud_init(&params, "dummy-private-key", "dummy-public-key")
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