use super::super::contract::{BootstrapError, BootstrapParams};
use super::shared;

pub(crate) fn build_cloud_init_user_data(
  params: &BootstrapParams,
  server_private_key: &str,
  server_public_key: &str,
) -> Result<String, BootstrapError> {
  let script = shared::render_cloud_bootstrap_script(params)?;
  let escaped_script = shared::indent_for_yaml_block(&script, 6);

  // The first line must be exactly `#cloud-config` — one added space and every provider silently
  // ignores the whole payload with no error anywhere (write-only means nothing would ever surface
  // it). A renderer test pins this.
  //
  // The payload must also be pure ASCII, including comments inside the embedded script.
  // DigitalOcean mis-decodes UTF-8 on its ConfigDrive path, and cloud-init then rejects the whole
  // document — `both_renderers_emit_ascii_only_payloads` pins that.
  //
  // `output` tees every cloud-init stage into one log so a failed bootstrap can be read from the
  // provider's console — the only debugging channel a write-only server has.
  //
  // Unattended-upgrades runs with Automatic-Reboot on: nobody ever logs into these boxes, so a
  // kernel patch that waits for a manual reboot would wait forever. The tunnel and firewall come
  // back on boot (wg-quick@ is enabled; the ingress chain is restored by netfilter-persistent).
  Ok(format!(
    r#"#cloud-config
output: {{all: '| tee -a /var/log/cloud-init-output.log'}}
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
  - unattended-upgrades
write_files:
  - path: /etc/sysctl.d/99-goatstare-vpn.conf
    permissions: '0644'
    content: |
      net.ipv4.ip_forward=1
      net.ipv4.conf.all.forwarding=1
      net.ipv4.conf.default.forwarding=1
  - path: /etc/apt/apt.conf.d/20auto-upgrades
    permissions: '0644'
    content: |
      APT::Periodic::Update-Package-Lists "1";
      APT::Periodic::Unattended-Upgrade "1";
  - path: /etc/apt/apt.conf.d/52goatstare-unattended-upgrades
    permissions: '0644'
    content: |
      Unattended-Upgrade::Allowed-Origins {{
        "${{distro_id}}:${{distro_codename}}-security";
        "${{distro_id}}ESMApps:${{distro_codename}}-apps-security";
        "${{distro_id}}ESM:${{distro_codename}}-infra-security";
        "${{distro_id}}:${{distro_codename}}-updates";
      }};
      Unattended-Upgrade::Remove-Unused-Kernel-Packages "true";
      Unattended-Upgrade::Remove-Unused-Dependencies "true";
      Unattended-Upgrade::MinimalSteps "true";
      Unattended-Upgrade::Automatic-Reboot "true";
      Unattended-Upgrade::Automatic-Reboot-Time "04:00";
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
