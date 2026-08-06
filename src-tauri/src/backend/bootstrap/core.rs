use super::contract::{
  BootstrapArtifact, BootstrapError, BootstrapOutputContract, BootstrapParams,
  READINESS_MARKER_PATH,
};
use super::renderers::{cloud_init, docker_init};

#[derive(Clone)]
pub struct Bootstrap;

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
    super::contract::validate_bootstrap_params(params)?;

    let cloud_init_user_data = cloud_init::build_cloud_init_user_data(params);
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

  pub fn render_docker_init(
    &self,
    params: &BootstrapParams,
  ) -> Result<BootstrapArtifact, BootstrapError> {
    super::contract::validate_bootstrap_params(params)?;

    let cloud_init_user_data = docker_init::build_docker_init_script(params);
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
    super::contract::build_output_contract_from_marker(endpoint_ip, marker_json)
  }
}
