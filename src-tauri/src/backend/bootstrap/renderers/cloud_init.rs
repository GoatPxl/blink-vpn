use super::super::contract::{BootstrapError, BootstrapParams};
use super::shared;

pub(crate) fn build_cloud_init_user_data(
  params: &BootstrapParams,
  server_private_key: &str,
  server_public_key: &str,
) -> Result<String, BootstrapError> {
  let script = shared::render_cloud_bootstrap_script(params)?;
  let escaped_script = shared::indent_for_yaml_block(&script, 6);

  Ok(format!(
    r#"#cloud-config
package_update: true
package_upgrade: false
package_reboot_if_required: false
packages:
  - wireguard
  - wireguard-tools
  - iptables
  - iptables-persistent
  - resolvconf
  - kmod
write_files:
  - path: /etc/sysctl.d/99-goatstare-vpn.conf
    permissions: '0644'
    content: |
      net.ipv4.ip_forward=1
      net.ipv4.conf.all.forwarding=1
      net.ipv4.conf.default.forwarding=1
  - path: /etc/wireguard/server_private.key
    permissions: '0600'
    owner: root:root
    content: |
      {server_private_key}
  - path: /etc/wireguard/server_public.key
    permissions: '0600'
    owner: root:root
    content: |
      {server_public_key}
  - path: /usr/local/bin/goatstare-bootstrap.sh
    permissions: '0700'
    owner: root:root
    content: |
{escaped_script}
runcmd:
  - ["bash", "-lc", "/usr/local/bin/goatstare-bootstrap.sh"]
final_message: "goatstare bootstrap completed"
"#
  ))
}
