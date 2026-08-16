pub mod digitalocean;
pub mod digitalocean_regions;
pub mod docker;
pub mod retry;

use std::collections::HashMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use serde::Serialize;
use thiserror::Error;

use crate::backend::bootstrap::{BootstrapOutputContract, DEFAULT_WIREGUARD_PORT};
use crate::backend::crypto::keypair::WireGuardKeypair;
use crate::backend::db::{Database, DbError, ProviderRecord, ProviderStatus};
use crate::backend::secure_storage::{SecretStoreError, SecureStorage};

#[allow(unused_imports)]
pub use digitalocean::{
  Account, CreateDropletRequest, DEFAULT_IMAGE, DEFAULT_REGION, DEFAULT_SIZE, DigitalOceanConfig,
};

use digitalocean::{DigitalOceanApiClient, DigitalOceanError};
use docker::DockerProviderAdapter;
use retry::RetryPolicy;

pub trait ProviderAdapter: Send + Sync {
  fn provider_name(&self) -> &'static str;
  fn default_regions(&self) -> &'static [&'static str];
  fn validate_token(&self, token: &str) -> Result<Account, ProviderAdapterError>;
  fn list_regions(&self, token: &str) -> Result<Vec<ProviderRegion>, ProviderAdapterError>;
  /// Geography for a slug this adapter owns, without contacting the provider.
  ///
  /// Node records store only a slug, and the fleet list is meant to stay readable when a provider
  /// key goes bad — so naming an existing node's location must not require a token or a network
  /// call.
  fn describe_region(&self, slug: &str) -> ProviderRegion;
  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError>;
  fn create_droplet(
    &self,
    token: &str,
    droplet_name: &str,
    region: &str,
    size_slug: &str,
    init_payload: &str,
  ) -> Result<ProvisionedDroplet, ProviderAdapterError>;
  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError>;
  fn droplet_status(
    &self,
    token: &str,
    droplet_id: u64,
  ) -> Result<ProviderDropletStatus, ProviderAdapterError>;
  fn droplet_exists(&self, token: &str, droplet_id: u64) -> Result<bool, ProviderAdapterError>;
  fn delete_droplet(&self, token: &str, droplet_id: u64) -> Result<(), ProviderAdapterError>;
}

/// Where a region physically is, in provider-neutral terms.
///
/// Only `slug` is guaranteed. Every geography field is optional because providers add regions
/// faster than a curated table can follow, and rendering an invented city for an unrecognised
/// slug would tell the user they are exiting somewhere they are not. Absent means "not known",
/// and callers must display the slug rather than a guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderRegion {
  pub slug: String,
  pub city: Option<String>,
  /// State or province, where it disambiguates for a reader.
  pub area: Option<String>,
  pub country: Option<String>,
  /// ISO-3166 alpha-2, so the UI can render a flag without its own country table.
  pub country_code: Option<String>,
  pub continent: Option<String>,
}

impl ProviderRegion {
  /// A slug whose geography this adapter cannot name.
  pub fn unknown(slug: &str) -> Self {
    Self {
      slug: slug.trim().to_string(),
      city: None,
      area: None,
      country: None,
      country_code: None,
      continent: None,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderDropletStatus {
  New,
  Active,
  Off,
  Archive,
  Deleted,
  Unknown,
}

impl ProviderDropletStatus {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::New => "new",
      Self::Active => "active",
      Self::Off => "off",
      Self::Archive => "archive",
      Self::Deleted => "deleted",
      Self::Unknown => "unknown",
    }
  }
}

/// Maps a user's node name onto a name DigitalOcean will accept.
///
/// DigitalOcean requires a droplet name that is a valid hostname — letters, digits, dots and
/// hyphens only. A node name is free text, so the default "Primary Node" was rejected outright for
/// its space, ~1.5s into provisioning, with the reason discarded on the way back up.
///
/// The rule is DigitalOcean's, so the mapping belongs in its adapter rather than being imposed on
/// what the user may call a node (AGENTS.md §3). The label the user chose is untouched in the
/// local record; only the remote resource is renamed.
fn sanitize_droplet_name(raw: &str) -> String {
  let mut sanitized = String::with_capacity(raw.len());
  let mut last_was_separator = false;

  for character in raw.trim().chars() {
    if character.is_ascii_alphanumeric() || character == '.' {
      sanitized.push(character);
      last_was_separator = false;
    } else if !last_was_separator {
      // Collapse any run of unsupported characters into a single hyphen.
      sanitized.push('-');
      last_was_separator = true;
    }
  }

  // A hostname label may not begin or end with a separator.
  let trimmed = sanitized.trim_matches(|character| character == '-' || character == '.');
  // 63 is the DNS label limit; DigitalOcean allows more, but staying inside it keeps the name
  // usable everywhere it might be echoed.
  let bounded: String = trimmed.chars().take(63).collect();
  let bounded = bounded.trim_end_matches(|character| character == '-' || character == '.');

  if bounded.is_empty() {
    "goatstare-node".to_string()
  } else {
    bounded.to_string()
  }
}

#[derive(Clone)]
pub struct DigitalOceanProviderAdapter {
  client: DigitalOceanApiClient,
  config: DigitalOceanConfig,
}

impl DigitalOceanProviderAdapter {
  pub fn new() -> Result<Self, ProviderAdapterError> {
    let config = DigitalOceanConfig::phase1_defaults();
    let client = DigitalOceanApiClient::new_with_timeout(
      "https://api.digitalocean.com",
      RetryPolicy::digitalocean_default(),
      config.request_timeout,
    )
    .map_err(|source| ProviderAdapterError::Api {
      detail: "Unable to initialize DigitalOcean API client.".to_string(),
      source,
    })?;

    Ok(Self {
      client,
      config,
    })
  }

  /// Construct against a specific API base URL so tests can point at a local mock server.
  #[cfg(test)]
  fn with_base_url(base_url: &str) -> Result<Self, ProviderAdapterError> {
    let config = DigitalOceanConfig::phase1_defaults();
    let client = DigitalOceanApiClient::new_with_timeout(
      base_url,
      RetryPolicy::digitalocean_default(),
      config.request_timeout,
    )
    .map_err(|source| ProviderAdapterError::Api {
      detail: "Unable to initialize DigitalOcean API client.".to_string(),
      source,
    })?;

    Ok(Self { client, config })
  }
}

impl ProviderAdapter for DigitalOceanProviderAdapter {
  fn provider_name(&self) -> &'static str {
    "digitalocean"
  }

  fn default_regions(&self) -> &'static [&'static str] {
    &["nyc1", "sfo3", "fra1"]
  }

  fn validate_token(&self, token: &str) -> Result<Account, ProviderAdapterError> {
    self
      .client
      .validate_token(token)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean token validation request failed.".to_string(),
        source,
      })
  }

  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError> {
    let mut sizes = self
      .client
      .list_sizes(token)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean size catalog request failed.".to_string(),
        source,
      })?;

    sizes.sort_by(|left, right| {
      left
        .price_monthly
        .total_cmp(&right.price_monthly)
        .then_with(|| left.slug.cmp(&right.slug))
    });

    let fetched_at_epoch_seconds = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
      Ok(value) => value.as_secs().to_string(),
      Err(_) => "0".to_string(),
    };

    let pricing: Vec<ProviderPlanPricing> = sizes
      .into_iter()
      .filter(|size| size.available && size.regions.iter().any(|candidate| candidate == region))
      .map(|size| {
        let plan_label = if size.description.trim().is_empty() {
          size.slug.to_uppercase()
        } else {
          size.description.trim().to_string()
        };

        ProviderPlanPricing {
          plan_key: size.slug.clone(),
          plan_label,
          size_slug: size.slug,
          region: region.to_string(),
          memory_mb: size.memory,
          vcpus: size.vcpus,
          disk_gb: size.disk,
          transfer_tb: size.transfer,
          transfer_note: format!("Includes up to {:.2} TB transfer before provider overage policies apply.", size.transfer),
          monthly_usd: size.price_monthly,
          hourly_usd: size.price_hourly,
          estimated: false,
          source_timestamp: fetched_at_epoch_seconds.clone(),
        }
      })
      .collect();

    Ok(pricing)
  }

  fn list_regions(&self, token: &str) -> Result<Vec<ProviderRegion>, ProviderAdapterError> {
    let sizes = self
      .client
      .list_sizes(token)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean size catalog request failed.".to_string(),
        source,
      })?;

    // The available slug set still comes from the size catalog, so regions and pricing cannot
    // disagree about what can actually be provisioned. Geography only enriches that set.
    let mut regions = BTreeSet::new();
    for size in sizes.into_iter().filter(|size| size.available) {
      for region in size.regions {
        if !region.trim().is_empty() {
          regions.insert(region);
        }
      }
    }

    Ok(regions.iter().map(|slug| self.describe_region(slug)).collect())
  }

  fn describe_region(&self, slug: &str) -> ProviderRegion {
    digitalocean_regions::describe_region(slug)
  }

  fn create_droplet(
    &self,
    token: &str,
    droplet_name: &str,
    region: &str,
    size_slug: &str,
    init_payload: &str,
  ) -> Result<ProvisionedDroplet, ProviderAdapterError> {
    let request = CreateDropletRequest {
      name: sanitize_droplet_name(droplet_name),
      region: if region.trim().is_empty() {
        self.config.region.to_string()
      } else {
        region.trim().to_string()
      },
      size: if size_slug.trim().is_empty() {
        self.config.size.to_string()
      } else {
        size_slug.trim().to_string()
      },
      image: self.config.image.to_string(),
      tags: vec!["goatstare-phase1".to_string()],
      user_data: Some(init_payload.to_string()),
    };

    let created = self
      .client
      .create_droplet(token, &request)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean droplet create call failed.".to_string(),
        source,
      })?;

    let active = self
      .client
      .wait_for_droplet_active(
        token,
        created.id,
        self.config.poll_interval,
        self.config.poll_max_attempts,
        self.config.droplet_ready_timeout,
      )
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean droplet did not become active before timeout.".to_string(),
        source,
      })?;

    let public_ipv4 = self
      .client
      .extract_public_ipv4(&active)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "Active droplet did not expose public IPv4.".to_string(),
        source,
      })?;

    Ok(ProvisionedDroplet {
      droplet_id: active.id,
      public_ipv4,
      // A droplet's WireGuard listen port is the port the client dials; there is no port
      // translation in front of it, unlike the Docker dev path.
      endpoint_port: DEFAULT_WIREGUARD_PORT,
    })
  }

  /// Readiness for DigitalOcean is "the droplet is active and has a public IPv4".
  ///
  /// This used to return `Unsupported`, because the design required reading a marker file off the
  /// droplet — which needs SSH the app does not have. That step is gone: the server's keypair is
  /// generated locally and baked into cloud-init, so its public key is already known here and
  /// there is nothing left to retrieve. `create_droplet` above has already polled to active, so
  /// this only has to confirm the droplet is still healthy and report the endpoint.
  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
    let droplet = self
      .client
      .get_droplet(token, request.droplet_id)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "Could not confirm DigitalOcean droplet state before completing provisioning."
          .to_string(),
        source,
      })?;

    if !droplet.status.trim().eq_ignore_ascii_case("active") {
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "DigitalOcean droplet is '{}' rather than active; provisioning cannot complete.",
          droplet.status
        ),
      });
    }

    let public_ipv4 = if request.endpoint_ip.trim().is_empty() {
      self
        .client
        .extract_public_ipv4(&droplet)
        .map_err(|source| ProviderAdapterError::Api {
          detail: "Active droplet did not expose public IPv4.".to_string(),
          source,
        })?
    } else {
      request.endpoint_ip.clone()
    };

    Ok(BootstrapOutputContract {
      server_endpoint_ip: public_ipv4,
      // Echoed from what the caller generated and baked into cloud-init, not read back.
      server_public_key: request.server_keypair.public_key.clone(),
      listen_port: DEFAULT_WIREGUARD_PORT,
    })
  }

  fn droplet_exists(&self, token: &str, droplet_id: u64) -> Result<bool, ProviderAdapterError> {
    match self.client.get_droplet(token, droplet_id) {
      Ok(_) => Ok(true),
      Err(DigitalOceanError::NotFound { .. }) => Ok(false),
      Err(source) => Err(ProviderAdapterError::Api {
        detail: "DigitalOcean droplet existence check failed.".to_string(),
        source,
      }),
    }
  }

  fn droplet_status(&self, token: &str, droplet_id: u64) -> Result<ProviderDropletStatus, ProviderAdapterError> {
    match self.client.get_droplet(token, droplet_id) {
      Ok(droplet) => Ok(match droplet.status.trim().to_ascii_lowercase().as_str() {
        "new" => ProviderDropletStatus::New,
        "active" => ProviderDropletStatus::Active,
        "off" => ProviderDropletStatus::Off,
        "archive" => ProviderDropletStatus::Archive,
        _ => ProviderDropletStatus::Unknown,
      }),
      Err(DigitalOceanError::NotFound { .. }) => Ok(ProviderDropletStatus::Deleted),
      Err(source) => Err(ProviderAdapterError::Api {
        detail: "DigitalOcean droplet status check failed.".to_string(),
        source,
      }),
    }
  }

  fn delete_droplet(&self, token: &str, droplet_id: u64) -> Result<(), ProviderAdapterError> {
    self
      .client
      .delete_droplet(token, droplet_id)
      .map_err(|source| ProviderAdapterError::Api {
        detail: "DigitalOcean droplet delete call failed.".to_string(),
        source,
      })
  }
}

#[derive(Debug, Clone)]
pub struct ProvisionedDroplet {
  pub droplet_id: u64,
  pub public_ipv4: String,
  /// UDP port the *client* should dial to reach this node's WireGuard interface.
  ///
  /// Adapters report their own endpoint rather than the orchestrator assuming one. For
  /// DigitalOcean this is the server's listen port. For Docker it is the ephemeral host port
  /// published to the container, which differs per node so several can run at once.
  pub endpoint_port: u16,
}

#[derive(Debug, Clone)]
pub struct NodeInitRequest {
  pub node_name: String,
  pub region: String,
  pub size_slug: String,
  pub init_payload: String,
  pub endpoint_ip: String,
  pub readiness_marker_path: String,
  pub server_keypair: WireGuardKeypair,
}

#[derive(Debug, Clone)]
pub struct NodeInitResult {
  pub provisioned: ProvisionedDroplet,
}

#[derive(Debug, Clone)]
pub struct BootstrapWaitRequest {
  pub droplet_id: u64,
  pub endpoint_ip: String,
  pub readiness_marker_path: String,
  pub server_keypair: WireGuardKeypair,
}

pub trait NodeProvider: Send + Sync {
  fn provider_name(&self) -> &'static str;
  fn init(&self, token: &str, request: &NodeInitRequest) -> Result<NodeInitResult, ProviderAdapterError>;
  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError>;
  fn get_status(&self, token: &str, droplet_id: u64) -> Result<ProviderDropletStatus, ProviderAdapterError>;
  fn exists(&self, token: &str, droplet_id: u64) -> Result<bool, ProviderAdapterError>;
  fn delete(&self, token: &str, droplet_id: u64) -> Result<(), ProviderAdapterError>;
}

#[derive(Clone)]
struct AdapterNodeProvider {
  adapter: Arc<dyn ProviderAdapter>,
  token: Option<String>,
}

impl AdapterNodeProvider {
  fn new(adapter: Arc<dyn ProviderAdapter>) -> Self {
    Self { adapter, token: None }
  }

  fn with_token(adapter: Arc<dyn ProviderAdapter>, token: String) -> Self {
    Self {
      adapter,
      token: Some(token),
    }
  }
}

impl NodeProvider for AdapterNodeProvider {
  fn provider_name(&self) -> &'static str {
    self.adapter.provider_name()
  }

  fn init(&self, token: &str, request: &NodeInitRequest) -> Result<NodeInitResult, ProviderAdapterError> {
    let effective_token = self.token.as_deref().unwrap_or(token);

    let provisioned = self.adapter.create_droplet(
      effective_token,
      &request.node_name,
      &request.region,
      &request.size_slug,
      &request.init_payload,
    )?;

    Ok(NodeInitResult { provisioned })
  }

  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
    let effective_token = self.token.as_deref().unwrap_or(token);
    self.adapter.wait_for_bootstrap_ready(effective_token, request)
  }

  fn get_status(&self, token: &str, droplet_id: u64) -> Result<ProviderDropletStatus, ProviderAdapterError> {
    let effective_token = self.token.as_deref().unwrap_or(token);
    self.adapter.droplet_status(effective_token, droplet_id)
  }

  fn exists(&self, token: &str, droplet_id: u64) -> Result<bool, ProviderAdapterError> {
    let effective_token = self.token.as_deref().unwrap_or(token);
    self.adapter.droplet_exists(effective_token, droplet_id)
  }

  fn delete(&self, token: &str, droplet_id: u64) -> Result<(), ProviderAdapterError> {
    let effective_token = self.token.as_deref().unwrap_or(token);
    self.adapter.delete_droplet(effective_token, droplet_id)
  }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderPlanPricing {
  pub plan_key: String,
  pub plan_label: String,
  pub size_slug: String,
  pub region: String,
  pub memory_mb: u64,
  pub vcpus: u64,
  pub disk_gb: u64,
  pub transfer_tb: f64,
  pub transfer_note: String,
  pub monthly_usd: f64,
  pub hourly_usd: f64,
  pub estimated: bool,
  pub source_timestamp: String,
}

#[derive(Clone)]
pub struct ProviderRegistry {
  adapters_by_slug: HashMap<String, Arc<dyn ProviderAdapter>>,
  default_provider_slug: String,
}

#[derive(Debug, Error)]
pub enum ProviderAdapterError {
  #[error("DigitalOcean provider API call failed: {detail}")]
  Api {
    detail: String,
    #[source]
    source: DigitalOceanError,
  },
  #[error("Provider capability not available yet: {detail}")]
  Unsupported { detail: String },
}

#[derive(Debug, Error)]
pub enum ProviderFlowError {
  #[error("Provider database operation failed: {detail}")]
  Database {
    detail: String,
    #[source]
    source: DbError,
  },
  #[error("Provider secret storage operation failed: {detail}")]
  SecretStore {
    detail: String,
    #[source]
    source: SecretStoreError,
  },
  #[error("Provider adapter operation failed: {detail}")]
  Adapter {
    detail: String,
    #[source]
    source: ProviderAdapterError,
  },
  #[error("Provider '{provider_id}' was not found after persistence. {detail}")]
  MissingPersistedProvider { provider_id: String, detail: String },
  #[error("Provider '{provider_id}' has no token_ref. {detail}")]
  MissingTokenReference { provider_id: String, detail: String },
  #[error("Provider token_ref '{token_ref}' has no stored secret. {detail}")]
  MissingSecret { token_ref: String, detail: String },
  #[error("Provider with slug '{slug}' is not supported by this app build. {detail}")]
  UnsupportedProviderSlug { slug: String, detail: String },
  #[error("Provider '{provider_id}' is suspended. {detail}")]
  ProviderSuspended { provider_id: String, detail: String },
}

impl ProviderRegistry {
  pub fn new() -> Self {
    let digitalocean = DigitalOceanProviderAdapter::new().unwrap_or_else(|err| {
      panic!("failed to initialize DigitalOcean adapter: {err}")
    });

    let mut adapters_by_slug: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    adapters_by_slug.insert("digitalocean".to_string(), Arc::new(digitalocean));
    adapters_by_slug.insert("docker".to_string(), Arc::new(DockerProviderAdapter::new()));

    Self {
      adapters_by_slug,
      default_provider_slug: "digitalocean".to_string(),
    }
  }

  pub fn from_adapter(adapter: Arc<dyn ProviderAdapter>) -> Self {
    let slug = adapter.provider_name().to_string();
    let mut adapters_by_slug = HashMap::new();
    adapters_by_slug.insert(slug.clone(), adapter);
    Self {
      adapters_by_slug,
      default_provider_slug: slug,
    }
  }

  pub fn active(&self) -> &dyn ProviderAdapter {
    self
      .adapters_by_slug
      .get(&self.default_provider_slug)
      .map(|adapter| adapter.as_ref())
      .unwrap_or_else(|| panic!("default provider adapter missing: {}", self.default_provider_slug))
  }

  pub fn node_provider_for_slug(&self, slug: &str) -> Option<Arc<dyn NodeProvider>> {
    self
      .adapter_arc_for_slug(slug)
      .map(|adapter| Arc::new(AdapterNodeProvider::new(adapter)) as Arc<dyn NodeProvider>)
  }

  pub fn node_provider_for_provider_id(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
  ) -> Result<Arc<dyn NodeProvider>, ProviderFlowError> {
    let (provider, _adapter) = self.load_provider_and_adapter(db, provider_id)?;
    let token = self.resolve_provider_token_for_record(db, secure_storage, &provider)?;
    let adapter = self
      .adapter_arc_for_slug(&provider.slug)
      .ok_or_else(|| ProviderFlowError::UnsupportedProviderSlug {
        slug: provider.slug.clone(),
        detail: "Provider adapter is not registered for the stored provider slug.".to_string(),
      })?;

    Ok(Arc::new(AdapterNodeProvider::with_token(adapter, token)) as Arc<dyn NodeProvider>)
  }

  pub fn node_provider_for_provider_id_allow_suspended(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
  ) -> Result<Arc<dyn NodeProvider>, ProviderFlowError> {
    let (provider, _adapter) = self.load_provider_and_adapter_allow_suspended(db, provider_id)?;
    let token = self.resolve_provider_token_for_record(db, secure_storage, &provider)?;
    let adapter = self
      .adapter_arc_for_slug(&provider.slug)
      .ok_or_else(|| ProviderFlowError::UnsupportedProviderSlug {
        slug: provider.slug.clone(),
        detail: "Provider adapter is not registered for the stored provider slug.".to_string(),
      })?;

    Ok(Arc::new(AdapterNodeProvider::with_token(adapter, token)) as Arc<dyn NodeProvider>)
  }

  pub fn create_digitalocean_provider_with_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    provider_name: &str,
    api_token: &str,
  ) -> Result<ProviderRecord, ProviderFlowError> {
    self.upsert_digitalocean_provider_with_token(db, secure_storage, provider_id, provider_name, api_token)
  }

  pub fn upsert_digitalocean_provider_with_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    provider_name: &str,
    api_token: &str,
  ) -> Result<ProviderRecord, ProviderFlowError> {
    self.upsert_provider_with_token(
      db,
      secure_storage,
      provider_id,
      provider_name,
      "digitalocean",
      api_token,
    )
  }

  pub fn create_docker_provider_with_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    provider_name: &str,
    api_token: &str,
  ) -> Result<ProviderRecord, ProviderFlowError> {
    self.upsert_docker_provider_with_token(db, secure_storage, provider_id, provider_name, api_token)
  }

  pub fn upsert_docker_provider_with_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    provider_name: &str,
    api_token: &str,
  ) -> Result<ProviderRecord, ProviderFlowError> {
    self.upsert_provider_with_token(db, secure_storage, provider_id, provider_name, "docker", api_token)
  }

  /// Register a credential against whichever adapter the provider's slug names.
  ///
  /// The slug is resolved from the stored provider record by the caller, so adding a provider
  /// does not mean adding another command.
  pub fn upsert_provider_token_for_slug(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    provider_name: &str,
    provider_slug: &str,
    api_token: &str,
  ) -> Result<ProviderRecord, ProviderFlowError> {
    self.upsert_provider_with_token(
      db,
      secure_storage,
      provider_id,
      provider_name,
      provider_slug,
      api_token,
    )
  }

  fn upsert_provider_with_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    provider_name: &str,
    provider_slug: &str,
    api_token: &str,
  ) -> Result<ProviderRecord, ProviderFlowError> {
    self.adapter_for_slug(provider_slug)?;

    let existing = db
      .get_provider(provider_id)
      .map_err(|source| ProviderFlowError::Database {
        detail: "Failed to inspect provider before token upsert.".to_string(),
        source,
      })?;
    let previous_token_ref = existing.as_ref().and_then(|record| record.token_ref.clone());

    let token_ref = secure_storage
      .store_provider_api_token(provider_id, api_token)
      .map_err(|source| ProviderFlowError::SecretStore {
        detail: "Unable to save provider token in protected storage."
          .to_string(),
        source,
      })?;

    let update_result = match existing {
      Some(_) => db.update_provider(
        provider_id,
        provider_name,
        provider_slug,
        Some(&token_ref),
      ),
      None => db.create_provider(
        provider_id,
        provider_name,
        provider_slug,
        Some(&token_ref),
      ),
    };

    if let Err(source) = update_result {
      let _ = secure_storage.delete_provider_api_token(&token_ref);
      return Err(ProviderFlowError::Database {
        detail: "Failed to persist provider row with updated token reference.".to_string(),
        source,
      });
    }

    if let Some(previous_token_ref) = previous_token_ref {
      let _ = secure_storage.delete_provider_api_token(&previous_token_ref);
    }

    db
      .get_provider(provider_id)
      .map_err(|source| ProviderFlowError::Database {
        detail: "Failed to read back provider row after insert."
          .to_string(),
        source,
      })?
      .ok_or_else(|| ProviderFlowError::MissingPersistedProvider {
        provider_id: provider_id.to_string(),
        detail: "Expected provider row to exist after upsert call.".to_string(),
      })
  }

  pub fn validate_provider_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
  ) -> Result<Account, ProviderFlowError> {
    let (provider, adapter) = self.load_provider_and_adapter(db, provider_id)?;
    let token = self.resolve_provider_token_for_record(db, secure_storage, &provider)?;
    adapter
      .validate_token(&token)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Token validation through provider adapter failed.".to_string(),
        source,
      })
  }

  pub fn init_provider_node(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    request: &NodeInitRequest,
  ) -> Result<NodeInitResult, ProviderFlowError> {
    let node_provider = self.node_provider_for_provider_id(db, secure_storage, provider_id)?;
    let token = self.resolve_provider_token_for_orchestrator(db, secure_storage, provider_id)?;
    node_provider
      .init(&token, request)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Node initialization through NodeProvider failed.".to_string(),
        source,
      })
  }

  pub fn list_provider_plan_pricing(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderFlowError> {
    let (provider, adapter) = self.load_provider_and_adapter(db, provider_id)?;
    let token = self.resolve_provider_token_for_record(db, secure_storage, &provider)?;
    adapter
      .list_plan_pricing(&token, region)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Provider pricing lookup failed through adapter.".to_string(),
        source,
      })
  }

  pub fn list_provider_regions(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
  ) -> Result<Vec<ProviderRegion>, ProviderFlowError> {
    let (provider, adapter) = self.load_provider_and_adapter(db, provider_id)?;
    let token = self.resolve_provider_token_for_record(db, secure_storage, &provider)?;
    adapter
      .list_regions(&token)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Provider region lookup failed through adapter.".to_string(),
        source,
      })
  }

  /// Geography for a slug, without a token or a network call.
  ///
  /// Returns `None` only when the provider row itself cannot be resolved; an adapter that does not
  /// recognise the slug still answers, with the geography fields empty.
  pub fn describe_provider_region(
    &self,
    db: &Database,
    provider_id: &str,
    slug: &str,
  ) -> Option<ProviderRegion> {
    let (_, adapter) = self.load_provider_and_adapter(db, provider_id).ok()?;
    Some(adapter.describe_region(slug))
  }

  pub fn wait_for_provider_bootstrap_ready(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderFlowError> {
    let node_provider = self.node_provider_for_provider_id(db, secure_storage, provider_id)?;
    let token = self.resolve_provider_token_for_orchestrator(db, secure_storage, provider_id)?;
    node_provider
      .wait_for_bootstrap_ready(&token, request)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Bootstrap readiness wait failed in provider adapter.".to_string(),
        source,
      })
  }

  pub fn delete_provider_droplet(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    droplet_id: u64,
  ) -> Result<(), ProviderFlowError> {
    let node_provider = self.node_provider_for_provider_id(db, secure_storage, provider_id)?;
    node_provider
      .delete("", droplet_id)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Droplet deletion through provider adapter failed.".to_string(),
        source,
      })
  }

  pub fn provider_droplet_exists(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    droplet_id: u64,
  ) -> Result<bool, ProviderFlowError> {
    let node_provider = self.node_provider_for_provider_id(db, secure_storage, provider_id)?;
    node_provider
      .exists("", droplet_id)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Droplet existence check through provider adapter failed.".to_string(),
        source,
      })
  }

  pub fn provider_droplet_status(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    droplet_id: u64,
  ) -> Result<ProviderDropletStatus, ProviderFlowError> {
    let node_provider = self.node_provider_for_provider_id_allow_suspended(db, secure_storage, provider_id)?;
    node_provider
      .get_status("", droplet_id)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Droplet status check through provider adapter failed.".to_string(),
        source,
      })
  }

  fn load_provider_and_adapter(
    &self,
    db: &Database,
    provider_id: &str,
  ) -> Result<(ProviderRecord, &dyn ProviderAdapter), ProviderFlowError> {
    let (provider, adapter) = self.load_provider_and_adapter_allow_suspended(db, provider_id)?;

    if provider.status == ProviderStatus::Suspended {
      return Err(ProviderFlowError::ProviderSuspended {
        provider_id: provider.id.clone(),
        detail: "Resume the provider from Settings before running this operation.".to_string(),
      });
    }

    Ok((provider, adapter))
  }

  fn load_provider_and_adapter_allow_suspended(
    &self,
    db: &Database,
    provider_id: &str,
  ) -> Result<(ProviderRecord, &dyn ProviderAdapter), ProviderFlowError> {
    let provider = db
      .get_provider(provider_id)
      .map_err(|source| ProviderFlowError::Database {
        detail: "Failed loading provider row for adapter dispatch.".to_string(),
        source,
      })?
      .ok_or_else(|| ProviderFlowError::MissingPersistedProvider {
        provider_id: provider_id.to_string(),
        detail: "Cannot use provider because no DB row exists for this id.".to_string(),
      })?;

    let adapter = self.adapter_for_slug(&provider.slug)?;
    Ok((provider, adapter))
  }

  fn adapter_for_slug(&self, slug: &str) -> Result<&dyn ProviderAdapter, ProviderFlowError> {
    self
      .adapters_by_slug
      .get(slug)
      .map(|adapter| adapter.as_ref())
      .ok_or_else(|| ProviderFlowError::UnsupportedProviderSlug {
        slug: slug.to_string(),
        detail: "Add provider adapter support or update provider configuration.".to_string(),
      })
  }

  fn adapter_arc_for_slug(&self, slug: &str) -> Option<Arc<dyn ProviderAdapter>> {
    self.adapters_by_slug.get(slug).cloned()
  }

  fn resolve_provider_token(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
  ) -> Result<String, ProviderFlowError> {
    let (provider, _adapter) = self.load_provider_and_adapter(db, provider_id)?;
    self.resolve_provider_token_for_record(db, secure_storage, &provider)
  }

  fn resolve_provider_token_for_record(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider: &ProviderRecord,
  ) -> Result<String, ProviderFlowError> {
    let token_ref = provider
      .token_ref
      .clone()
      .ok_or_else(|| ProviderFlowError::MissingTokenReference {
        provider_id: provider.id.clone(),
        detail: "Provider must be initialized with a token before adapter operations."
          .to_string(),
      })?;

    let resolved = secure_storage
      .get_provider_api_token(&token_ref)
      .map_err(|source| ProviderFlowError::SecretStore {
        detail: "Failed retrieving provider token from secure storage.".to_string(),
        source,
      })?;

    if let Some(token) = resolved {
      return Ok(token);
    }

    // Self-heal stale DB pointer so UI can re-register token cleanly.
  let _ = db.update_provider_token_ref(&provider.id, None);

    Err(ProviderFlowError::MissingSecret {
        token_ref,
    detail: "Stored token reference is stale: secure storage entry is missing. Re-save your provider token in onboarding Step 2."
          .to_string(),
      })
  }

  pub fn resolve_provider_token_for_orchestrator(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
  ) -> Result<String, ProviderFlowError> {
    self.resolve_provider_token(db, secure_storage, provider_id)
  }
}


#[cfg(test)]
mod tests {
  use std::path::PathBuf;
  use std::sync::Arc;
  use std::time::{SystemTime, UNIX_EPOCH};

  use rusqlite::{Connection, params};

  use crate::backend::bootstrap::{BootstrapOutputContract, DEFAULT_WIREGUARD_PORT};
  use crate::backend::db::Database;
  use crate::backend::secure_storage::SecureStorage;

  use crate::backend::crypto::keypair::generate_server_keypair;

  use super::{
    Account, BootstrapWaitRequest, NodeInitRequest, ProviderAdapter, ProviderAdapterError,
    ProviderDropletStatus, ProviderPlanPricing, ProviderRegion, ProviderRegistry, ProvisionedDroplet,
    sanitize_droplet_name,
  };

  fn test_db_path(test_name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("clock must be after unix epoch")
      .as_nanos();
    path.push(format!("goatstare_provider_{test_name}_{nanos}.sqlite3"));
    path
  }

  /// DigitalOcean readiness must work without any channel into the droplet.
  ///
  /// This previously returned `Unsupported` unconditionally, because the design needed to read a
  /// marker file off the droplet — which requires SSH the app does not have. Every production
  /// provision therefore created a billable droplet, failed here, and rolled back. The server
  /// keypair is now generated locally and baked into cloud-init, so the public key is already
  /// known and readiness is just "the droplet is active with an IP".
  /// The exact failure this fixes: the default node name contains a space, and DigitalOcean
  /// rejects any droplet name that is not a valid hostname.
  #[test]
  fn droplet_names_with_spaces_are_made_valid_hostnames() {
    assert_eq!(sanitize_droplet_name("Primary Node"), "Primary-Node");
    assert_eq!(sanitize_droplet_name("  Amsterdam  Exit  "), "Amsterdam-Exit");
  }

  #[test]
  fn droplet_names_keep_characters_digitalocean_accepts() {
    assert_eq!(sanitize_droplet_name("DigitalOceanLive"), "DigitalOceanLive");
    assert_eq!(sanitize_droplet_name("node-1.example"), "node-1.example");
  }

  #[test]
  fn droplet_names_collapse_unsupported_runs_and_never_end_on_a_separator() {
    assert_eq!(sanitize_droplet_name("my___node!!!"), "my-node");
    assert_eq!(sanitize_droplet_name("--edge--"), "edge");
    assert_eq!(sanitize_droplet_name("emoji 🚀 node"), "emoji-node");
  }

  #[test]
  fn droplet_names_are_bounded_and_never_empty() {
    let long = sanitize_droplet_name(&"a".repeat(200));
    assert_eq!(long.len(), 63);

    // A name of nothing but punctuation still has to produce something DigitalOcean accepts.
    assert_eq!(sanitize_droplet_name("!!!"), "goatstare-node");
    assert_eq!(sanitize_droplet_name(""), "goatstare-node");

    // Truncation must not leave a trailing separator, which is itself invalid.
    let truncated = sanitize_droplet_name(&format!("{}-tail", "b".repeat(62)));
    assert!(!truncated.ends_with('-'), "truncated name must not end on a separator");
  }

  #[test]
  fn digitalocean_bootstrap_ready_uses_locally_known_key_and_droplet_status() {
    let mut server = mockito::Server::new();
    let keypair = generate_server_keypair().expect("keypair should generate");

    let droplet = server
      .mock("GET", "/v2/droplets/4242")
      .with_status(200)
      .with_header("content-type", "application/json")
      .with_body(
        r#"{"droplet":{"id":4242,"name":"gs-node","status":"active","networks":{"v4":[{"ip_address":"203.0.113.77","type":"public"}]}}}"#,
      )
      .create();

    let adapter = super::DigitalOceanProviderAdapter::with_base_url(&server.url())
      .expect("adapter should construct against mock server");

    let contract = adapter
      .wait_for_bootstrap_ready(
        "dop_v1_test_token",
        &BootstrapWaitRequest {
          droplet_id: 4242,
          endpoint_ip: String::new(),
          readiness_marker_path: "/var/lib/goatstare/bootstrap-ready.json".to_string(),
          server_keypair: keypair.clone(),
        },
      )
      .expect("readiness should resolve without any remote retrieval channel");

    droplet.assert();
    assert_eq!(contract.server_endpoint_ip, "203.0.113.77");
    assert_eq!(contract.listen_port, DEFAULT_WIREGUARD_PORT);
    // Echoed from what was baked into cloud-init, never fetched from the droplet.
    assert_eq!(contract.server_public_key, keypair.public_key);
  }

  #[test]
  fn digitalocean_bootstrap_ready_rejects_a_droplet_that_is_not_active() {
    let mut server = mockito::Server::new();

    let _droplet = server
      .mock("GET", "/v2/droplets/99")
      .with_status(200)
      .with_header("content-type", "application/json")
      .with_body(r#"{"droplet":{"id":99,"name":"gs-node","status":"new","networks":{"v4":[]}}}"#)
      .create();

    let adapter = super::DigitalOceanProviderAdapter::with_base_url(&server.url())
      .expect("adapter should construct against mock server");

    let error = adapter
      .wait_for_bootstrap_ready(
        "dop_v1_test_token",
        &BootstrapWaitRequest {
          droplet_id: 99,
          endpoint_ip: String::new(),
          readiness_marker_path: "/var/lib/goatstare/bootstrap-ready.json".to_string(),
          server_keypair: generate_server_keypair().expect("keypair should generate"),
        },
      )
      .expect_err("a droplet that is not active must not report ready");

    assert!(error.to_string().contains("active"));
  }

  #[derive(Clone)]
  struct TestProviderAdapter;

  impl ProviderAdapter for TestProviderAdapter {
    fn provider_name(&self) -> &'static str {
      "test-provider"
    }

    fn default_regions(&self) -> &'static [&'static str] {
      &["test-region"]
    }

    fn validate_token(&self, _token: &str) -> Result<Account, ProviderAdapterError> {
      Ok(Account {
        uuid: "test-account".to_string(),
        email: "test@example.com".to_string(),
        status: "active".to_string(),
      })
    }

    fn list_regions(&self, _token: &str) -> Result<Vec<ProviderRegion>, ProviderAdapterError> {
      Ok(vec![self.describe_region("test-region")])
    }


    fn describe_region(&self, slug: &str) -> ProviderRegion {
      ProviderRegion::unknown(slug)
    }

    fn list_plan_pricing(
      &self,
      _token: &str,
      _region: &str,
    ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError> {
      Ok(Vec::new())
    }

    fn create_droplet(
      &self,
      _token: &str,
      _droplet_name: &str,
      _region: &str,
      _size_slug: &str,
      _init_payload: &str,
    ) -> Result<ProvisionedDroplet, ProviderAdapterError> {
      Ok(ProvisionedDroplet {
        droplet_id: 42,
        public_ipv4: "203.0.113.10".to_string(),
        endpoint_port: DEFAULT_WIREGUARD_PORT,
      })
    }

    fn wait_for_bootstrap_ready(
      &self,
      _token: &str,
      request: &BootstrapWaitRequest,
    ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
      Ok(BootstrapOutputContract {
        server_endpoint_ip: request.endpoint_ip.clone(),
        server_public_key: "server-pub-key-123".to_string(),
        listen_port: 51820,
      })
    }

    fn droplet_status(
      &self,
      _token: &str,
      _droplet_id: u64,
    ) -> Result<ProviderDropletStatus, ProviderAdapterError> {
      Ok(ProviderDropletStatus::Active)
    }

    fn droplet_exists(&self, _token: &str, _droplet_id: u64) -> Result<bool, ProviderAdapterError> {
      Ok(true)
    }

    fn delete_droplet(&self, _token: &str, _droplet_id: u64) -> Result<(), ProviderAdapterError> {
      Ok(())
    }
  }

  #[test]
  fn node_provider_registry_can_wrap_docker_adapter() {
    let registry = ProviderRegistry::new();
    let node_provider = registry.node_provider_for_slug("docker").expect("docker node provider should resolve");

    assert_eq!(node_provider.provider_name(), "docker");
  }

  #[test]
  fn provider_flow_persists_token_reference_not_plaintext() {
    let db = Database::new(test_db_path("token_ref_only"));
    db.run_migrations().expect("migrations should succeed");

    let registry = ProviderRegistry::new();
    let secure_storage = SecureStorage::dev_mock();
    let plaintext_token = "dop_v1_sensitive_plaintext_token";

    let provider = registry
      .create_digitalocean_provider_with_token(
        &db,
        &secure_storage,
        "provider-1",
        "DigitalOcean",
        plaintext_token,
      )
      .expect("provider flow should succeed");

    let token_ref = provider.token_ref.expect("provider should store token_ref");
    assert_ne!(token_ref, plaintext_token);

    let recovered = secure_storage
      .get_digitalocean_api_token(&token_ref)
      .expect("token retrieval should succeed")
      .expect("token should exist in mock secret storage");
    assert_eq!(recovered, plaintext_token);

    let connection = Connection::open(db.path()).expect("db path should be openable");
    let row = connection
      .query_row(
        "SELECT id, name, slug, IFNULL(token_ref, '') FROM providers WHERE id = ?1",
        params!["provider-1"],
        |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
          ))
        },
      )
      .expect("provider row should be queryable");

    assert_ne!(row.0, plaintext_token);
    assert_ne!(row.1, plaintext_token);
    assert_ne!(row.2, plaintext_token);
    assert_ne!(row.3, plaintext_token);
  }

  #[test]
  fn validate_token_clears_stale_token_reference_when_secret_missing() {
    let db = Database::new(test_db_path("stale_token_ref_cleared"));
    db.run_migrations().expect("migrations should succeed");

    let registry = ProviderRegistry::new();
    let secure_storage = SecureStorage::dev_mock();

    let provider = registry
      .create_digitalocean_provider_with_token(
        &db,
        &secure_storage,
        "provider-1",
        "DigitalOcean",
        "dop_v1_stale_test_token",
      )
      .expect("provider create should succeed");

    let token_ref = provider
      .token_ref
      .clone()
      .expect("token ref should be present");

    secure_storage
      .delete_digitalocean_api_token(&token_ref)
      .expect("mock token delete should succeed");

    let result = registry.validate_provider_token(&db, &secure_storage, "provider-1");
    assert!(result.is_err());

    let updated = db
      .get_provider("provider-1")
      .expect("provider lookup should succeed")
      .expect("provider should still exist");
    assert!(updated.token_ref.is_none());
  }

  #[test]
  fn init_provider_node_uses_node_provider_interface() {
    let db = Database::new(test_db_path("init_node_provider_interface"));
    db.run_migrations().expect("migrations should succeed");

    let registry = ProviderRegistry::from_adapter(Arc::new(TestProviderAdapter));
    let secure_storage = SecureStorage::dev_mock();
    let provider_id = "provider-1";

    registry
      .upsert_provider_with_token(
        &db,
        &secure_storage,
        provider_id,
        "Test Provider",
        "test-provider",
        "test-token",
      )
      .expect("provider create should succeed");

    let request = NodeInitRequest {
      node_name: "test-node".to_string(),
      region: "test-region".to_string(),
      size_slug: "test-size".to_string(),
      init_payload: "#!/bin/sh\necho hi".to_string(),
      endpoint_ip: String::new(),
      readiness_marker_path: "/tmp/marker".to_string(),
      server_keypair: generate_server_keypair().expect("keypair generation should succeed"),
    };

    let result = registry
      .init_provider_node(&db, &secure_storage, provider_id, &request)
      .expect("node init should succeed through NodeProvider interface");

    assert_eq!(result.provisioned.droplet_id, 42);
  }

  #[cfg(target_os = "windows")]
  #[test]
  fn provider_flow_roundtrip_with_windows_secure_storage_and_db() {
    let db = Database::new(test_db_path("windows_secure_storage_roundtrip"));
    db.run_migrations().expect("migrations should succeed");

    let registry = ProviderRegistry::new();
    let secure_storage = SecureStorage::new();
    let token = "dop_v1_windows_roundtrip_test_token";

    let provider = registry
      .create_digitalocean_provider_with_token(
        &db,
        &secure_storage,
        "provider-1",
        "DigitalOcean",
        token,
      )
      .expect("provider create should succeed with Windows secure storage");

    let token_ref = provider
      .token_ref
      .clone()
      .expect("token ref should exist");

    let persisted = db
      .get_provider("provider-1")
      .expect("provider lookup should succeed")
      .expect("provider row should exist");

    assert_eq!(persisted.token_ref.as_deref(), Some(token_ref.as_str()));

    let recovered = secure_storage
      .get_digitalocean_api_token(&token_ref)
      .expect("token lookup should succeed");
    assert_eq!(recovered.as_deref(), Some(token));

    secure_storage
      .delete_digitalocean_api_token(&token_ref)
      .expect("cleanup delete should succeed");
  }
}