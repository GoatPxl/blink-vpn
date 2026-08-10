use super::super::contract::{BootstrapParams, READINESS_MARKER_PATH};
use super::shared::{FIREWALL_RULES_FRAGMENT, WG_INTERFACE_CONFIG_FRAGMENT};

pub(crate) fn build_docker_init_script(params: &BootstrapParams) -> String {
  let script_body = render_docker_bootstrap_body(params);

  format!(
    r#"#!/usr/bin/env bash
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive
apt-get update >/dev/null 2>&1
apt-get install -y --no-install-recommends wireguard-tools iproute2 iptables >/dev/null 2>&1
rm -rf /var/lib/apt/lists/*

cat > /etc/sysctl.d/99-goatstare-vpn.conf <<'EOF'
net.ipv4.ip_forward=1
net.ipv4.conf.all.forwarding=1
net.ipv4.conf.default.forwarding=1
EOF

{script_body}
"#
  )
}

fn render_docker_bootstrap_body(params: &BootstrapParams) -> String {
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
log_stage "validate-tools" "checking WireGuard and container networking toolchain"
command -v wg >/dev/null
command -v iptables >/dev/null
command -v ip >/dev/null

log_stage "validate-privileges" "checking WireGuard interface operations inside container"
probe="gspw0"
ip link delete "$probe" >/dev/null 2>&1 || true
if ! ip link add "$probe" type wireguard >/tmp/goatstare-wg-prereq.err 2>&1; then
  cat /tmp/goatstare-wg-prereq.err >&2 || true
  rm -f /tmp/goatstare-wg-prereq.err
  exit 61
fi
rm -f /tmp/goatstare-wg-prereq.err
ip link delete "$probe" >/dev/null 2>&1 || true

if [ ! -f /etc/wireguard/server_private.key ] || [ ! -f /etc/wireguard/server_public.key ]; then
  log_stage "keypair-missing" "app must stage server keys before running init script"
  exit 64
fi

SERVER_PRIVATE_KEY=$(tr -d '\r\n' < /etc/wireguard/server_private.key)
SERVER_PUBLIC_KEY=$(tr -d '\r\n' < /etc/wireguard/server_public.key)

chmod 600 /etc/wireguard/server_private.key /etc/wireguard/server_public.key

log_stage "write-config" "writing /etc/wireguard/${{WG_IFACE}}.conf"

cat > /etc/wireguard/${{WG_IFACE}}.conf <<EOF
{interface_config_fragment}
EOF
chmod 600 /etc/wireguard/${{WG_IFACE}}.conf

log_stage "sysctl" "ensuring IPv4 forwarding is enabled"
sysctl -w net.ipv4.ip_forward=1 >/dev/null
sysctl -w net.ipv4.conf.all.forwarding=1 >/dev/null 2>&1 || true
sysctl -w net.ipv4.conf.default.forwarding=1 >/dev/null 2>&1 || true

log_stage "firewall" "installing input chain rules"
{firewall_rules_fragment}

log_stage "interface" "activating WireGuard interface"
if command -v wg-quick >/dev/null 2>&1 && wg-quick up "${{WG_IFACE}}" >/tmp/goatstare-wg-up.log 2>&1; then
  rm -f /tmp/goatstare-wg-up.log
else
  cat /tmp/goatstare-wg-up.log >&2 || true
  rm -f /tmp/goatstare-wg-up.log
  ip link delete "${{WG_IFACE}}" >/dev/null 2>&1 || true
  ip link add "${{WG_IFACE}}" type wireguard
  ip addr replace "${{SERVER_CIDR}}" dev "${{WG_IFACE}}"
  wg set "${{WG_IFACE}}" private-key /etc/wireguard/server_private.key listen-port "${{WG_PORT}}"
  ip link set up dev "${{WG_IFACE}}"
fi

log_stage "verify-interface" "verifying WireGuard interface state"
ip link show "${{WG_IFACE}}" >/dev/null 2>&1
ip -o link show "${{WG_IFACE}}" | grep -q 'UP'
wg show "${{WG_IFACE}}" >/dev/null 2>&1

log_stage "verify-config" "ensuring /etc/wireguard/${{WG_IFACE}}.conf exists and is non-empty"
if [ ! -s "/etc/wireguard/${{WG_IFACE}}.conf" ]; then
  log_stage "verify-config" "missing or empty WireGuard config file"
  exit 63
fi

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
