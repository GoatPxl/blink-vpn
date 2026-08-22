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
    server_private_key: &str,
    server_public_key: &str,
  ) -> Result<BootstrapArtifact, BootstrapError> {
    super::contract::validate_bootstrap_params(params)?;

    let init_payload =
      cloud_init::build_cloud_init_user_data(params, server_private_key, server_public_key)?;
    let output_contract_preview = BootstrapOutputContract {
      server_endpoint_ip: params.server_endpoint_ip.clone(),
      server_public_key: server_public_key.to_string(),
      listen_port: params.listen_port,
    };

    Ok(BootstrapArtifact {
      init_payload,
      readiness_marker_path: READINESS_MARKER_PATH.to_string(),
      output_contract_preview,
    })
  }

  pub fn render_docker_init(
    &self,
    params: &BootstrapParams,
    server_private_key: &str,
    server_public_key: &str,
  ) -> Result<BootstrapArtifact, BootstrapError> {
    super::contract::validate_bootstrap_params(params)?;

    let init_payload =
      docker_init::build_docker_init_script(params, server_private_key, server_public_key)?;
    // The server public key is known locally at render time, so this is the real value rather
    // than a placeholder resolved from a readiness marker the app would have to read back.
    let output_contract_preview = BootstrapOutputContract {
      server_endpoint_ip: params.server_endpoint_ip.clone(),
      server_public_key: server_public_key.to_string(),
      listen_port: params.listen_port,
    };

    Ok(BootstrapArtifact {
      init_payload,
      readiness_marker_path: READINESS_MARKER_PATH.to_string(),
      output_contract_preview,
    })
  }
}
