mod contract;
mod core;
mod renderers;

pub use contract::{
  BootstrapError, BootstrapOutputContract, BootstrapParams,
  DEFAULT_CLIENT_CIDR, DEFAULT_INTERFACE_NAME, DEFAULT_SERVER_INTERFACE_CIDR,
  DEFAULT_WIREGUARD_PORT, READINESS_MARKER_PATH,
};
pub use core::Bootstrap;

#[cfg(test)]
mod tests {
  use super::{
    Bootstrap, BootstrapOutputContract, BootstrapParams, DEFAULT_CLIENT_CIDR,
    DEFAULT_INTERFACE_NAME, DEFAULT_SERVER_INTERFACE_CIDR, DEFAULT_WIREGUARD_PORT,
    READINESS_MARKER_PATH,
  };
  use crate::backend::crypto::keypair::{generate_client_keypair, generate_server_keypair};

  /// Params carrying a real client public key, as every production render does.
  fn params_with_client_peer(client_public_key: &str) -> BootstrapParams {
    BootstrapParams {
      server_endpoint_ip: "198.51.100.9".to_string(),
      listen_port: DEFAULT_WIREGUARD_PORT,
      server_interface_cidr: DEFAULT_SERVER_INTERFACE_CIDR.to_string(),
      client_cidr: DEFAULT_CLIENT_CIDR.to_string(),
      interface_name: DEFAULT_INTERFACE_NAME.to_string(),
      client_public_key: client_public_key.to_string(),
    }
  }

  #[test]
  fn renders_cloud_init_with_wireguard_nat_and_marker() {
    let bootstrap = Bootstrap::new();
    let client = generate_client_keypair().expect("client keypair should generate");
    let params = params_with_client_peer(&client.public_key);
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

    // The peer must be baked in: the app never contacts the server again to add one.
    assert!(artifact.init_payload.contains("[Peer]"));
    assert!(
      artifact.init_payload.contains(&format!("CLIENT_PUBLIC_KEY=\"{}\"", client.public_key)),
      "cloud-init must carry the real client public key"
    );
    assert!(artifact.init_payload.contains("PublicKey = ${CLIENT_PUBLIC_KEY}"));
    assert!(artifact.init_payload.contains("AllowedIPs = ${CLIENT_CIDR}"));
  }

  #[test]
  fn renders_docker_init_with_ubuntu_prereqs_and_marker() {
    let bootstrap = Bootstrap::new();
    let client = generate_client_keypair().expect("client keypair should generate");
    let params = params_with_client_peer(&client.public_key);
    let server = generate_server_keypair().expect("server keypair should generate");

    let artifact = bootstrap
      .render_docker_init(&params, &server.private_key, &server.public_key)
      .expect("docker init render should succeed");

    assert!(artifact.init_payload.contains("#!/usr/bin/env bash"));
    assert!(artifact.init_payload.contains("apt-get update"));
    assert!(artifact.init_payload.contains("apt-get install -y --no-install-recommends wireguard-tools iproute2 iptables"));
    assert!(artifact.init_payload.contains("/etc/sysctl.d/99-goatstare-vpn.conf"));
    assert!(artifact.init_payload.contains("goatstare.bootstrap.stage=${stage}"));
    assert!(!artifact.init_payload.contains("wg genkey"));
    assert!(artifact.init_payload.contains("wg-quick up \"${WG_IFACE}\""));
    assert!(!artifact.init_payload.contains("systemctl enable --now"));
    assert!(artifact.init_payload.contains(READINESS_MARKER_PATH));
    assert!(!artifact.init_payload.contains("#cloud-config"));
    assert!(!artifact.init_payload.contains("apk add"));

    // The script stages its own key material, standing in for cloud-init write_files, so no
    // `docker exec` staging round-trip is needed.
    assert!(artifact.init_payload.contains(&server.private_key));
    assert!(artifact.init_payload.contains(&server.public_key));
    assert!(artifact.init_payload.contains("[Peer]"));
    assert!(artifact.init_payload.contains(&format!("CLIENT_PUBLIC_KEY=\"{}\"", client.public_key)));

    // The real public key is known locally, so it is no longer a marker-resolved placeholder.
    assert_eq!(artifact.output_contract_preview.server_public_key, server.public_key);
  }

  /// Both providers must produce the same server state from the same inputs. If these two
  /// renderers drift, the Docker lane stops being evidence about DigitalOcean.
  #[test]
  fn both_renderers_bake_the_same_wireguard_config() {
    let bootstrap = Bootstrap::new();
    let client = generate_client_keypair().expect("client keypair should generate");
    let server = generate_server_keypair().expect("server keypair should generate");
    let params = params_with_client_peer(&client.public_key);

    let cloud = bootstrap
      .render_cloud_init(&params, &server.private_key, &server.public_key)
      .expect("cloud-init render should succeed");
    let docker = bootstrap
      .render_docker_init(&params, &server.private_key, &server.public_key)
      .expect("docker init render should succeed");

    for payload in [&cloud.init_payload, &docker.init_payload] {
      assert!(payload.contains("[Interface]"));
      assert!(payload.contains("[Peer]"));
      assert!(payload.contains("PublicKey = ${CLIENT_PUBLIC_KEY}"));
      assert!(payload.contains(&format!("CLIENT_PUBLIC_KEY=\"{}\"", client.public_key)));
      assert!(payload.contains(&format!("SERVER_CIDR=\"{}\"", DEFAULT_SERVER_INTERFACE_CIDR)));
      assert!(payload.contains(&format!("CLIENT_CIDR=\"{}\"", DEFAULT_CLIENT_CIDR)));
    }

    assert_eq!(
      cloud.output_contract_preview.server_public_key,
      docker.output_contract_preview.server_public_key
    );
  }

  /// A payload without a peer produces a server that accepts no handshakes and cannot be fixed
  /// afterwards, so rendering must fail rather than provision one.
  #[test]
  fn rejects_render_without_client_public_key() {
    let bootstrap = Bootstrap::new();
    let server = generate_server_keypair().expect("server keypair should generate");
    let params = params_with_client_peer("");

    let err = bootstrap
      .render_cloud_init(&params, &server.private_key, &server.public_key)
      .expect_err("a payload with no client peer must be rejected");
    assert!(err.to_string().contains("client_public_key"));

    let err = bootstrap
      .render_docker_init(&params, &server.private_key, &server.public_key)
      .expect_err("a payload with no client peer must be rejected");
    assert!(err.to_string().contains("client_public_key"));
  }

  #[test]
  fn rejects_malformed_client_public_key() {
    let bootstrap = Bootstrap::new();
    let server = generate_server_keypair().expect("server keypair should generate");
    let params = params_with_client_peer("not-a-wireguard-key");

    let err = bootstrap
      .render_cloud_init(&params, &server.private_key, &server.public_key)
      .expect_err("malformed key material must be rejected");
    assert!(err.to_string().contains("client_public_key"));
  }

  #[test]
  fn rejects_invalid_cidr_and_port() {
    let bootstrap = Bootstrap::new();
    let client = generate_client_keypair().expect("client keypair should generate");
    let params = BootstrapParams {
      server_endpoint_ip: "198.51.100.9".to_string(),
      listen_port: 80,
      server_interface_cidr: "10.66.0.1/99".to_string(),
      client_cidr: "not-cidr".to_string(),
      interface_name: DEFAULT_INTERFACE_NAME.to_string(),
      client_public_key: client.public_key,
    };

    let err = bootstrap
      .render_cloud_init(&params, "dummy-private-key", "dummy-public-key")
      .expect_err("invalid params must be rejected");
    assert!(err.to_string().contains("Invalid bootstrap parameter"));
  }

  /// The output contract comes from what was generated locally, never from the server.
  ///
  /// This replaces two tests that parsed a readiness-marker JSON document into the contract. That
  /// machinery was the original design — the server generated its own keys and the app read them
  /// back out of `/var/lib/goatstare/bootstrap-ready.json` — and it required an SSH channel the app
  /// does not have and must not gain (AGENTS.md §2). Nothing called it any more, so the only thing
  /// keeping it alive was its own tests.
  #[test]
  fn the_output_contract_is_known_before_the_node_exists() {
    let bootstrap = Bootstrap::new();
    let server = generate_server_keypair().expect("server keygen should succeed");
    let client = generate_client_keypair().expect("client keygen should succeed");

    let artifact = bootstrap
      .render_cloud_init(
        &params_with_client_peer(&client.public_key),
        &server.private_key,
        &server.public_key,
      )
      .expect("render should succeed");

    assert_eq!(
      artifact.output_contract_preview,
      BootstrapOutputContract {
        server_endpoint_ip: "198.51.100.9".to_string(),
        server_public_key: server.public_key,
        listen_port: DEFAULT_WIREGUARD_PORT,
      },
      "every field must be derivable at render time, with nothing left to read back"
    );
  }

  /// The marker is still written by the script, as a self-check. Its path travels with the artifact
  /// so the payload and the Docker health check cannot disagree — but nothing here consumes it.
  #[test]
  fn the_readiness_marker_path_is_reported_without_being_read() {
    let bootstrap = Bootstrap::new();
    let server = generate_server_keypair().expect("server keygen should succeed");
    let client = generate_client_keypair().expect("client keygen should succeed");

    let artifact = bootstrap
      .render_cloud_init(
        &params_with_client_peer(&client.public_key),
        &server.private_key,
        &server.public_key,
      )
      .expect("render should succeed");

    assert_eq!(artifact.readiness_marker_path, READINESS_MARKER_PATH);
    assert!(artifact.init_payload.contains(READINESS_MARKER_PATH));
  }
}