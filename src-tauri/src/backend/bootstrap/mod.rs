mod contract;
mod core;
mod renderers;

pub use contract::{
  allocate_peer_addresses, subnet_of, BootstrapError, BootstrapOutputContract, BootstrapParams,
  BootstrapPeer, DEFAULT_INTERFACE_NAME, DEFAULT_PEER_SLOTS, DEFAULT_SERVER_INTERFACE_CIDR,
  DEFAULT_WIREGUARD_PORT, READINESS_MARKER_PATH,
};
pub use core::Bootstrap;

#[cfg(test)]
mod tests {
  use super::{
    allocate_peer_addresses, subnet_of, Bootstrap, BootstrapOutputContract, BootstrapParams,
    BootstrapPeer, DEFAULT_INTERFACE_NAME, DEFAULT_PEER_SLOTS, DEFAULT_SERVER_INTERFACE_CIDR,
    DEFAULT_WIREGUARD_PORT, READINESS_MARKER_PATH,
  };
  use crate::backend::crypto::keypair::{generate_client_keypair, generate_server_keypair};

  /// Params carrying real client public keys, as every production render does.
  fn params_with_peers(public_keys: &[String]) -> BootstrapParams {
    let addresses = allocate_peer_addresses(DEFAULT_SERVER_INTERFACE_CIDR, public_keys.len())
      .expect("the default subnet has room for these peers");

    BootstrapParams {
      server_endpoint_ip: "198.51.100.9".to_string(),
      listen_port: DEFAULT_WIREGUARD_PORT,
      server_interface_cidr: DEFAULT_SERVER_INTERFACE_CIDR.to_string(),
      interface_name: DEFAULT_INTERFACE_NAME.to_string(),
      peers: public_keys
        .iter()
        .zip(addresses)
        .map(|(public_key, address_cidr)| BootstrapPeer {
          public_key: public_key.clone(),
          address_cidr,
        })
        .collect(),
    }
  }

  fn params_with_client_peer(client_public_key: &str) -> BootstrapParams {
    params_with_peers(&[client_public_key.to_string()])
  }

  fn generated_public_keys(count: usize) -> Vec<String> {
    (0..count)
      .map(|_| {
        generate_client_keypair()
          .expect("client keypair should generate")
          .public_key
      })
      .collect()
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
      artifact.init_payload.contains(&format!("PublicKey = {}", client.public_key)),
      "cloud-init must carry the real client public key"
    );
    assert!(artifact.init_payload.contains("AllowedIPs = 10.66.0.2/32"));
    assert!(
      artifact.init_payload.contains(&format!("PEER_PUBLIC_KEYS=\"{}\"", client.public_key)),
      "the verify loop must know which peers to look for"
    );
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
    assert!(artifact.init_payload.contains(&format!("PublicKey = {}", client.public_key)));

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
      assert!(payload.contains(&format!("PublicKey = {}", client.public_key)));
      assert!(payload.contains(&format!("PEER_PUBLIC_KEYS=\"{}\"", client.public_key)));
      assert!(payload.contains(&format!("SERVER_CIDR=\"{}\"", DEFAULT_SERVER_INTERFACE_CIDR)));
      assert!(payload.contains("NAT_SOURCE_CIDR=\"10.66.0.0/24\""));
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
    assert!(err.to_string().contains("peers"));

    let err = bootstrap
      .render_docker_init(&params, &server.private_key, &server.public_key)
      .expect_err("a payload with no client peer must be rejected");
    assert!(err.to_string().contains("peers"));
  }

  #[test]
  fn rejects_malformed_client_public_key() {
    let bootstrap = Bootstrap::new();
    let server = generate_server_keypair().expect("server keypair should generate");
    let params = params_with_client_peer("not-a-wireguard-key");

    let err = bootstrap
      .render_cloud_init(&params, &server.private_key, &server.public_key)
      .expect_err("malformed key material must be rejected");
    assert!(err.to_string().contains("peers"));
  }

  #[test]
  fn rejects_invalid_cidr_and_port() {
    let bootstrap = Bootstrap::new();
    let client = generate_client_keypair().expect("client keypair should generate");
    let params = BootstrapParams {
      server_endpoint_ip: "198.51.100.9".to_string(),
      listen_port: 80,
      server_interface_cidr: "10.66.0.1/99".to_string(),
      interface_name: DEFAULT_INTERFACE_NAME.to_string(),
      peers: vec![BootstrapPeer {
        public_key: client.public_key,
        address_cidr: "not-cidr".to_string(),
      }],
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

  /// Every slot has to reach the server config. A slot that is generated, recorded locally and
  /// then not baked is unusable forever, because nothing can add it afterwards (AGENTS.md §2).
  #[test]
  fn bakes_every_peer_slot_into_the_server_config() {
    let bootstrap = Bootstrap::new();
    let server = generate_server_keypair().expect("server keypair should generate");
    let public_keys = generated_public_keys(DEFAULT_PEER_SLOTS);
    let params = params_with_peers(&public_keys);

    for artifact in [
      bootstrap
        .render_cloud_init(&params, &server.private_key, &server.public_key)
        .expect("cloud-init render should succeed"),
      bootstrap
        .render_docker_init(&params, &server.private_key, &server.public_key)
        .expect("docker init render should succeed"),
    ] {
      // Counted by the per-peer AllowedIPs line rather than the [Peer] marker: cloud-init indents
      // the whole script into a YAML block, and the Docker payload mentions "[Peer]" once more in
      // a comment about its manual fallback.
      assert_eq!(
        artifact.init_payload.matches("AllowedIPs = 10.66.0.").count(),
        DEFAULT_PEER_SLOTS,
        "every slot needs its own [Peer] block"
      );

      for public_key in &public_keys {
        assert!(
          artifact.init_payload.contains(&format!("PublicKey = {public_key}")),
          "peer {public_key} is missing from the payload"
        );
      }

      // Distinct addresses, or two devices would answer to the same tunnel IP.
      for address in ["10.66.0.2/32", "10.66.0.3/32", "10.66.0.4/32", "10.66.0.5/32", "10.66.0.6/32"] {
        assert!(
          artifact.init_payload.contains(&format!("AllowedIPs = {address}")),
          "address {address} is missing from the payload"
        );
      }
    }
  }

  /// NAT must cover the tunnel subnet, not one peer.
  ///
  /// The rule masqueraded `${CLIENT_CIDR}` — a single /32 — while only one peer existed. Under
  /// that rule a second device completes a handshake and then has no internet at all, which reads
  /// as a broken client rather than a server misconfiguration.
  #[test]
  fn nat_masquerades_the_tunnel_subnet_not_a_single_peer() {
    let bootstrap = Bootstrap::new();
    let server = generate_server_keypair().expect("server keypair should generate");
    let params = params_with_peers(&generated_public_keys(DEFAULT_PEER_SLOTS));

    let payload = bootstrap
      .render_cloud_init(&params, &server.private_key, &server.public_key)
      .expect("cloud-init render should succeed")
      .init_payload;

    assert!(payload.contains("NAT_SOURCE_CIDR=\"10.66.0.0/24\""));
    assert!(payload.contains("POSTROUTING -s ${NAT_SOURCE_CIDR} -j MASQUERADE"));
    assert!(
      !payload.contains("POSTROUTING -s ${CLIENT_CIDR}"),
      "the single-peer NAT scope must be gone"
    );
  }

  /// The self-check has to cover every peer, not just the first.
  #[test]
  fn the_bootstrap_self_check_loops_over_every_peer() {
    let bootstrap = Bootstrap::new();
    let server = generate_server_keypair().expect("server keypair should generate");
    let public_keys = generated_public_keys(3);
    let params = params_with_peers(&public_keys);

    let cloud = bootstrap
      .render_cloud_init(&params, &server.private_key, &server.public_key)
      .expect("cloud-init render should succeed")
      .init_payload;
    let docker = bootstrap
      .render_docker_init(&params, &server.private_key, &server.public_key)
      .expect("docker init render should succeed")
      .init_payload;

    let expected_list = format!("PEER_PUBLIC_KEYS=\"{}\"", public_keys.join(" "));

    for payload in [&cloud, &docker] {
      assert!(payload.contains(&expected_list), "the verify loop needs every key");
      assert!(payload.contains("for peer_key in ${PEER_PUBLIC_KEYS}; do"));
    }

    // Docker also re-adds peers by hand when wg-quick is unavailable, and that path is per-peer.
    assert!(docker.contains("for peer_spec in ${PEER_SPECS}; do"));
    assert!(docker.contains("allowed-ips \"${peer_spec##*|}\""));
  }

  #[test]
  fn allocates_distinct_host_addresses_inside_the_server_subnet() {
    let addresses = allocate_peer_addresses("10.66.0.1/24", 5).expect("a /24 has ample room");

    assert_eq!(
      addresses,
      vec![
        "10.66.0.2/32".to_string(),
        "10.66.0.3/32".to_string(),
        "10.66.0.4/32".to_string(),
        "10.66.0.5/32".to_string(),
        "10.66.0.6/32".to_string(),
      ]
    );
  }

  /// The server keeps the address it already holds, wherever in the range that is.
  #[test]
  fn allocation_skips_the_address_the_server_occupies() {
    let addresses = allocate_peer_addresses("10.66.0.3/24", 3).expect("a /24 has ample room");

    assert!(
      !addresses.contains(&"10.66.0.3/32".to_string()),
      "the server address must not be handed to a peer: {addresses:?}"
    );
    // Allocation starts at the bottom of the host range and steps over the server, so the
    // addresses below it are perfectly usable.
    assert_eq!(
      addresses,
      vec![
        "10.66.0.1/32".to_string(),
        "10.66.0.2/32".to_string(),
        "10.66.0.4/32".to_string(),
      ]
    );
  }

  /// A /30 holds two hosts, one of which is the server. Asking for more must fail rather than
  /// silently hand out the network or broadcast address.
  #[test]
  fn allocation_refuses_to_exceed_the_subnet() {
    let single = allocate_peer_addresses("10.66.0.1/30", 1).expect("one peer fits in a /30");
    assert_eq!(single, vec!["10.66.0.2/32".to_string()]);

    let error = allocate_peer_addresses("10.66.0.1/30", 2)
      .expect_err("a /30 cannot hold the server plus two peers");
    assert!(error.to_string().contains("room for 1 peers"), "got: {error}");

    let error = allocate_peer_addresses("10.66.0.1/31", 1)
      .expect_err("a /31 has no usable host range");
    assert!(error.to_string().contains("no room for peers"), "got: {error}");
  }

  #[test]
  fn allocation_rejects_a_malformed_subnet() {
    assert!(allocate_peer_addresses("not-a-cidr", 1).is_err());
    assert!(allocate_peer_addresses("10.66.0.1", 1).is_err());
    assert!(allocate_peer_addresses("10.66.0.1/33", 1).is_err());
  }

  #[test]
  fn subnet_of_masks_the_host_bits_away() {
    assert_eq!(subnet_of("10.66.0.1/24").expect("valid"), "10.66.0.0/24");
    assert_eq!(subnet_of("10.66.5.200/16").expect("valid"), "10.66.0.0/16");
    assert_eq!(subnet_of("192.168.1.1/32").expect("valid"), "192.168.1.1/32");
    assert!(subnet_of("nonsense").is_err());
  }
}