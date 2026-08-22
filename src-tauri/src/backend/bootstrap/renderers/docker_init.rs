use super::super::contract::{subnet_of, BootstrapError, BootstrapParams, READINESS_MARKER_PATH};
use super::shared::{
  peer_public_keys_shell_list, peer_specs_shell_list, render_wg_interface_config,
  FIREWALL_RULES_FRAGMENT,
};

/// Docker's equivalent of the cloud-init payload.
///
/// On DigitalOcean, cloud-init `write_files` stages the server keys and the sysctl drop-in before
/// running the bootstrap script. A container has no cloud-init, so this script performs those same
/// writes itself, up front, and then continues with logic identical to the cloud path. Both
/// providers therefore reach the same state from one self-contained payload delivered at create
/// time — no `docker exec`, no staging round-trip.
pub(crate) fn build_docker_init_script(
  params: &BootstrapParams,
  server_private_key: &str,
  server_public_key: &str,
) -> Result<String, BootstrapError> {
  let script_body = render_docker_bootstrap_body(params)?;

  Ok(format!(
    r#"#!/usr/bin/env bash
set -euo pipefail

umask 077
mkdir -p /etc/wireguard /var/lib/goatstare

# Equivalent of cloud-init write_files: stage server key material before bootstrap.
cat > /etc/wireguard/server_private.key <<'GOATSTARE_EOF_PRIVATE'
{server_private_key}
GOATSTARE_EOF_PRIVATE
cat > /etc/wireguard/server_public.key <<'GOATSTARE_EOF_PUBLIC'
{server_public_key}
GOATSTARE_EOF_PUBLIC
chmod 600 /etc/wireguard/server_private.key /etc/wireguard/server_public.key

cat > /etc/sysctl.d/99-goatstare-vpn.conf <<'GOATSTARE_EOF_SYSCTL'
net.ipv4.ip_forward=1
net.ipv4.conf.all.forwarding=1
net.ipv4.conf.default.forwarding=1
GOATSTARE_EOF_SYSCTL

# The dev image ships these preinstalled; the install is a no-op there and a fallback on a bare
# base image. Tolerate failure so provisioning works offline, then verify the tools below.
export DEBIAN_FRONTEND=noninteractive
if ! command -v wg >/dev/null 2>&1; then
  apt-get update >/dev/null 2>&1 || true
  apt-get install -y --no-install-recommends wireguard-tools iproute2 iptables >/dev/null 2>&1 || true
  rm -rf /var/lib/apt/lists/*
fi

{script_body}
"#
  ))
}

fn render_docker_bootstrap_body(params: &BootstrapParams) -> Result<String, BootstrapError> {
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
PEER_SPECS="{peer_specs}"
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
  # wg-quick would have applied the [Peer] blocks from the config; the manual path must add
  # every one explicitly or the server comes up with no peers and rejects every handshake.
  for peer_spec in ${{PEER_SPECS}}; do
    wg set "${{WG_IFACE}}" peer "${{peer_spec%%|*}}" allowed-ips "${{peer_spec##*|}}"
  done
  ip link set up dev "${{WG_IFACE}}"
fi

log_stage "verify-interface" "verifying WireGuard interface state"
ip link show "${{WG_IFACE}}" >/dev/null 2>&1
ip -o link show "${{WG_IFACE}}" | grep -q 'UP'
wg show "${{WG_IFACE}}" >/dev/null 2>&1

log_stage "verify-runtime-peer" "confirming every peer is live on ${{WG_IFACE}}"
for peer_key in ${{PEER_PUBLIC_KEYS}}; do
  wg show "${{WG_IFACE}}" peers | grep -qF "${{peer_key}}" || {{
    log_stage "verify-runtime-peer" "peer ${{peer_key}} not present on running interface"
    exit 66
  }}
done

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
    nat_source_cidr = nat_source_cidr,
    peer_public_keys = peer_public_keys_shell_list(&params.peers),
    peer_specs = peer_specs_shell_list(&params.peers),
    marker = READINESS_MARKER_PATH,
    endpoint = params.server_endpoint_ip,
    interface_config_fragment = render_wg_interface_config(&params.peers),
    firewall_rules_fragment = FIREWALL_RULES_FRAGMENT,
  ))
}
