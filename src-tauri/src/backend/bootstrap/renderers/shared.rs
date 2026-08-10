use super::super::contract::{BootstrapParams, READINESS_MARKER_PATH};

pub(super) const WG_INTERFACE_CONFIG_FRAGMENT: &str = r#"[Interface]
Address = ${SERVER_CIDR}
ListenPort = ${WG_PORT}
PrivateKey = ${SERVER_PRIVATE_KEY}
SaveConfig = false

PostUp = iptables -A FORWARD -i ${WG_IFACE} -j ACCEPT; iptables -A FORWARD -o ${WG_IFACE} -j ACCEPT; iptables -t nat -A POSTROUTING -s ${CLIENT_CIDR} -j MASQUERADE
PostDown = iptables -D FORWARD -i ${WG_IFACE} -j ACCEPT; iptables -D FORWARD -o ${WG_IFACE} -j ACCEPT; iptables -t nat -D POSTROUTING -s ${CLIENT_CIDR} -j MASQUERADE"#;

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

pub(super) fn render_cloud_bootstrap_script(params: &BootstrapParams) -> String {
  let script_body = render_cloud_bootstrap_body(params);

  format!(
    r#"#!/usr/bin/env bash
set -euo pipefail

{script_body}
"#
  )
}

fn render_cloud_bootstrap_body(params: &BootstrapParams) -> String {
  format!(
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
CLIENT_CIDR="{client_cidr}"
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

log_stage "write-config" "writing /etc/wireguard/${{WG_IFACE}}.conf"

cat > /etc/wireguard/${{WG_IFACE}}.conf <<EOF
{interface_config_fragment}
EOF

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
    client_cidr = params.client_cidr,
    marker = READINESS_MARKER_PATH,
    endpoint = params.server_endpoint_ip,
    interface_config_fragment = WG_INTERFACE_CONFIG_FRAGMENT,
    firewall_rules_fragment = FIREWALL_RULES_FRAGMENT,
  )
}
