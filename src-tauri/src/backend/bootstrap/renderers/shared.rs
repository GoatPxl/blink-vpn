use super::super::contract::{
  subnet_of, BootstrapError, BootstrapParams, BootstrapPeer, READINESS_MARKER_PATH,
};

/// The `[Interface]` half of the server's `wg0.conf`.
///
/// `${...}` are *shell* variables, expanded by the unquoted heredoc that writes this file on the
/// server — not Rust format arguments. Only the `[Peer]` blocks are substituted here, by
/// `render_wg_interface_config`.
///
/// The NAT rule masquerades `${NAT_SOURCE_CIDR}`, the whole tunnel subnet. It masqueraded a single
/// client `/32` while only one peer existed; under that rule a second peer completes a handshake
/// and then has no internet at all, which looks like a client fault and is not one.
const WG_INTERFACE_HEADER_FRAGMENT: &str = r#"[Interface]
Address = ${SERVER_CIDR}
ListenPort = ${WG_PORT}
PrivateKey = ${SERVER_PRIVATE_KEY}
SaveConfig = false

PostUp = iptables -A FORWARD -i ${WG_IFACE} -j ACCEPT; iptables -A FORWARD -o ${WG_IFACE} -j ACCEPT; iptables -t nat -A POSTROUTING -s ${NAT_SOURCE_CIDR} -j MASQUERADE
PostDown = iptables -D FORWARD -i ${WG_IFACE} -j ACCEPT; iptables -D FORWARD -o ${WG_IFACE} -j ACCEPT; iptables -t nat -D POSTROUTING -s ${NAT_SOURCE_CIDR} -j MASQUERADE"#;

/// Server `wg0.conf`, including one `[Peer]` block per baked peer.
///
/// Every peer ships in the payload because the app never contacts the server after create — there
/// is no second round-trip in which to register one (AGENTS.md §2, §12).
///
/// Peer keys and addresses are written by Rust directly into the heredoc. Both are validated as
/// WireGuard key material and IPv4 CIDRs by `validate_bootstrap_params` before this runs, so
/// nothing shell-significant can reach the generated file.
pub(super) fn render_wg_interface_config(peers: &[BootstrapPeer]) -> String {
  let mut config = String::from(WG_INTERFACE_HEADER_FRAGMENT);

  for peer in peers {
    config.push_str("\n\n[Peer]\nPublicKey = ");
    config.push_str(&peer.public_key);
    config.push_str("\nAllowedIPs = ");
    config.push_str(&peer.address_cidr);
  }

  config
}

/// Space-separated peer public keys, for the shell loops that verify each one landed.
pub(super) fn peer_public_keys_shell_list(peers: &[BootstrapPeer]) -> String {
  peers
    .iter()
    .map(|peer| peer.public_key.as_str())
    .collect::<Vec<_>>()
    .join(" ")
}

/// Space-separated `publickey|address` pairs, for the Docker path's manual `wg set` fallback.
///
/// `|` cannot occur in base64 key material or in a CIDR, so it is unambiguous as a separator and
/// the shell can split each pair with `%%` / `##` expansions.
pub(super) fn peer_specs_shell_list(peers: &[BootstrapPeer]) -> String {
  peers
    .iter()
    .map(|peer| format!("{}|{}", peer.public_key, peer.address_cidr))
    .collect::<Vec<_>>()
    .join(" ")
}

pub(super) const FIREWALL_RULES_FRAGMENT: &str = r#"iptables -N GOATSTARE-INPUT 2>/dev/null || true
iptables -F GOATSTARE-INPUT
iptables -A GOATSTARE-INPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -A GOATSTARE-INPUT -i lo -j ACCEPT
iptables -A GOATSTARE-INPUT -i ${WG_IFACE} -j ACCEPT
iptables -A GOATSTARE-INPUT -p tcp --dport 22 -j ACCEPT
iptables -A GOATSTARE-INPUT -p udp --dport ${WG_PORT} -j ACCEPT
iptables -A GOATSTARE-INPUT -j DROP
iptables -C INPUT -j GOATSTARE-INPUT 2>/dev/null || iptables -I INPUT 1 -j GOATSTARE-INPUT"#;

pub(super) fn indent_for_yaml_block(content: &str, spaces: usize) -> String {
  let prefix = " ".repeat(spaces);
  content
    .lines()
    .map(|line| format!("{prefix}{line}"))
    .collect::<Vec<_>>()
    .join("\n")
}

pub(super) fn render_cloud_bootstrap_script(
  params: &BootstrapParams,
) -> Result<String, BootstrapError> {
  let script_body = render_cloud_bootstrap_body(params)?;

  Ok(format!(
    r#"#!/usr/bin/env bash
set -euo pipefail

{script_body}
"#
  ))
}

fn render_cloud_bootstrap_body(params: &BootstrapParams) -> Result<String, BootstrapError> {
  // Never fails in practice — the CIDR is validated before any renderer runs — but propagated
  // rather than unwrapped so a future caller that skips validation gets an error, not a panic.
  let nat_source_cidr = subnet_of(&params.server_interface_cidr)?;

  Ok(format!(
    r#"
log_stage() {{
  local stage="$1"
  shift || true
  echo "goatstare.bootstrap.stage=${{stage}} $*"
}}

trap 'log_stage "failed" "line=${{LINENO}} exit_status=$?"' ERR

WG_IFACE="{iface}"
WG_PORT="{port}"
SERVER_CIDR="{server_cidr}"
NAT_SOURCE_CIDR="{nat_source_cidr}"
PEER_PUBLIC_KEYS="{peer_public_keys}"
MARKER_PATH="{marker}"

umask 077
mkdir -p /etc/wireguard /var/lib/goatstare

log_stage "start" "iface=${{WG_IFACE}} port=${{WG_PORT}} endpoint={endpoint}"
log_stage "validate-tools" "checking WireGuard and firewall toolchain"
command -v wg >/dev/null
command -v iptables >/dev/null
command -v systemctl >/dev/null

if [ ! -f /etc/wireguard/server_private.key ] || [ ! -f /etc/wireguard/server_public.key ]; then
  log_stage "keypair-missing" "app must stage server keys before running init script"
  exit 64
fi

SERVER_PRIVATE_KEY=$(tr -d '\r\n' < /etc/wireguard/server_private.key)
SERVER_PUBLIC_KEY=$(tr -d '\r\n' < /etc/wireguard/server_public.key)

log_stage "write-config" "writing /etc/wireguard/${{WG_IFACE}}.conf with baked client peers"

cat > /etc/wireguard/${{WG_IFACE}}.conf <<EOF
{interface_config_fragment}
EOF
chmod 600 /etc/wireguard/${{WG_IFACE}}.conf

log_stage "verify-peer" "confirming every baked peer is present in ${{WG_IFACE}}.conf"
for peer_key in ${{PEER_PUBLIC_KEYS}}; do
  grep -q "^PublicKey = ${{peer_key}}$" /etc/wireguard/${{WG_IFACE}}.conf || {{
    log_stage "verify-peer" "peer ${{peer_key}} missing from generated config"
    exit 65
  }}
done

log_stage "sysctl" "enabling IPv4 forwarding"
sysctl --system

log_stage "firewall" "installing input chain rules"
{firewall_rules_fragment}

log_stage "service" "enabling wg-quick@${{WG_IFACE}}"
systemctl enable --now wg-quick@${{WG_IFACE}}

log_stage "marker" "writing readiness marker to ${{MARKER_PATH}}"
cat > ${{MARKER_PATH}} <<EOF
{{"status":"ready","server_public_key":"${{SERVER_PUBLIC_KEY}}","listen_port":${{WG_PORT}}}}
EOF
chmod 600 ${{MARKER_PATH}}

log_stage "marker-validate" "verifying readiness marker contents"
EXPECTED_MARKER="{{\"status\":\"ready\",\"server_public_key\":\"${{SERVER_PUBLIC_KEY}}\",\"listen_port\":${{WG_PORT}}}}"
ACTUAL_MARKER=$(cat ${{MARKER_PATH}})
if [ "$ACTUAL_MARKER" != "$EXPECTED_MARKER" ]; then
  log_stage "marker-validate" "expected=$EXPECTED_MARKER actual=$ACTUAL_MARKER"
  exit 1
fi

log_stage "complete" "bootstrap finished"
"#,
    iface = params.interface_name,
    port = params.listen_port,
    server_cidr = params.server_interface_cidr,
    nat_source_cidr = nat_source_cidr,
    peer_public_keys = peer_public_keys_shell_list(&params.peers),
    marker = READINESS_MARKER_PATH,
    endpoint = params.server_endpoint_ip,
    interface_config_fragment = render_wg_interface_config(&params.peers),
    firewall_rules_fragment = FIREWALL_RULES_FRAGMENT,
  ))
}
