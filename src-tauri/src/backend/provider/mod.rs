pub mod digitalocean;
pub mod digitalocean_regions;
pub mod docker;
pub mod ec2;
pub mod ec2_regions;
pub mod gce;
pub mod gce_regions;
pub mod linode;
pub mod linode_regions;
pub mod openstack;
pub mod openstack_regions;
mod place_coordinates;
pub mod retry;

use std::collections::HashMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;

use crate::backend::bootstrap::{BootstrapOutputContract, DEFAULT_WIREGUARD_PORT};
use crate::backend::crypto::keypair::WireGuardKeypair;
use crate::backend::db::{Database, DbError, ProviderRecord, ProviderStatus};
use crate::backend::secure_storage::{SecretStoreError, SecureStorage};

// `Account` is deliberately not re-exported: it is DigitalOcean's own account response, and the
// neutral result of a credential check is `ProviderIdentity`, defined below.
#[allow(unused_imports)]
pub use digitalocean::{
  CreateDropletRequest, DEFAULT_IMAGE, DEFAULT_REGION, DEFAULT_SIZE, DigitalOceanConfig,
};

use digitalocean::{DigitalOceanApiClient, DigitalOceanError};
use docker::DockerProviderAdapter;
use ec2::{AwsCredentials, DEFAULT_EC2_REGION, Ec2ApiClient, Ec2Config};
use gce::{
  GceApiClient, GceConfig, GceError, ServiceAccountKey, GCE_CLIENT_MTU, GCE_TOKEN_URL,
  GCE_USER_DATA_LIMIT_BYTES,
};
use linode::{
  CreateLinodeRequest, LinodeApiClient, LinodeConfig, LinodeError, LinodeMetadata,
  LINODE_USER_DATA_LIMIT_BYTES,
};
use openstack::{
  OpenStackApiClient, OpenStackConfig, OpenStackCredentials, OpenStackError,
  OPENSTACK_USER_DATA_LIMIT_BYTES,
};
use retry::RetryPolicy;

pub trait ProviderAdapter: Send + Sync {
  fn provider_name(&self) -> &'static str;
  /// The credential inputs this provider needs, in the order they should be presented.
  ///
  /// DigitalOcean needs one API token; AWS needs an access key id and a secret access key. The UI
  /// renders whatever it is given rather than branching on a slug, so a provider's credential shape
  /// stays inside its adapter (AGENTS.md §3).
  fn credential_fields(&self) -> Vec<CredentialField>;
  /// Check the credential works, and report whose account it is.
  fn validate_token(&self, token: &str) -> Result<ProviderIdentity, ProviderAdapterError>;
  fn list_regions(&self, token: &str) -> Result<Vec<ProviderRegion>, ProviderAdapterError>;
  /// Geography for a slug this adapter owns, without contacting the provider.
  ///
  /// Node records store only a slug, and the fleet list is meant to stay readable when a provider
  /// key goes bad — so naming an existing node's location must not require a token or a network
  /// call.
  fn describe_region(&self, slug: &str) -> ProviderRegion;
  /// The regions this adapter can name without a credential, for showing where a provider runs
  /// before the user has a key for it. Empty for an adapter with no curated table.
  fn known_regions(&self) -> Vec<ProviderRegion> {
    Vec::new()
  }
  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError>;
  /// How this provider takes delivery of the bootstrap script.
  ///
  /// The orchestrator used to pick the renderer with `if provider_slug == "docker"`, which is the
  /// branch AGENTS.md §3 forbids and which silently decided a third provider's delivery mechanism
  /// by falling through an `else`. The adapter states it instead.
  fn init_delivery(&self) -> InitDelivery;
  fn create_node(
    &self,
    token: &str,
    node_name: &str,
    region: &str,
    size_slug: &str,
    init_payload: &str,
  ) -> Result<ProvisionedNode, ProviderAdapterError>;
  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError>;
  fn node_status(
    &self,
    token: &str,
    node_id: &str,
  ) -> Result<ProviderNodeStatus, ProviderAdapterError>;
  fn node_exists(&self, token: &str, node_id: &str) -> Result<bool, ProviderAdapterError>;
  fn delete_node(&self, token: &str, node_id: &str) -> Result<(), ProviderAdapterError>;
  /// Tunnel MTU client configs for this provider's nodes should carry, if the provider's
  /// networking calls for one.
  ///
  /// Some providers run instance NICs below 1500 (Google Compute Engine: 1460), where WireGuard's
  /// default of 1420 fragments or blackholes. The value is captured on the node row at provision
  /// time, so it survives adapter changes and stays `NULL` for nodes created before the hint
  /// existed. `None` — the default — means client configs carry no MTU line.
  fn client_mtu_hint(&self) -> Option<u16> {
    None
  }
}

/// How a provider takes delivery of the rendered bootstrap script.
///
/// The script body is identical across variants; only the wrapper differs. If these ever produce
/// different server state, the dev lane has stopped being evidence for the production one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitDelivery {
  /// cloud-init `#cloud-config`, consumed by the provider's first-boot agent.
  CloudInit,
  /// A bash script executed by a container entrypoint.
  DockerInit,
}

/// The single-field key used by providers that need only one value, and by every credential saved
/// before credentials could have more than one field.
pub const LEGACY_SINGLE_CREDENTIAL_KEY: &str = "api_token";

/// A provider credential, as stored: a set of named fields.
///
/// Serialized to a JSON object for storage. Deserialization accepts a bare string as well, because
/// every credential saved before this existed is a raw token with no JSON around it — those are
/// read as a single `api_token` field. Dropping that fallback would silently invalidate the stored
/// credential of every existing install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCredential {
  fields: std::collections::BTreeMap<String, String>,
}

impl ProviderCredential {
  pub fn from_fields(fields: impl IntoIterator<Item = (String, String)>) -> Self {
    Self {
      fields: fields.into_iter().collect(),
    }
  }

  /// Parse a stored credential value.
  ///
  /// A value that does not parse as a JSON object is treated as a legacy single token rather than
  /// as an error — that is the whole backward-compatibility guarantee.
  pub fn parse(stored: &str) -> Self {
    let trimmed = stored.trim();

    if trimmed.starts_with('{') {
      if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Self {
          fields: map
            .into_iter()
            .filter_map(|(key, value)| value.as_str().map(|text| (key, text.to_string())))
            .collect(),
        };
      }
    }

    Self::from_fields([(LEGACY_SINGLE_CREDENTIAL_KEY.to_string(), stored.to_string())])
  }

  /// Serialize for storage. Always a JSON object, so what is written is unambiguous.
  pub fn to_stored_value(&self) -> String {
    serde_json::Value::Object(
      self
        .fields
        .iter()
        .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
        .collect(),
    )
    .to_string()
  }

  pub fn field(&self, key: &str) -> Option<&str> {
    self.fields.get(key).map(String::as_str).filter(|value| !value.trim().is_empty())
  }

  /// A required field, or a capability error naming what is missing.
  pub fn require(&self, key: &str) -> Result<&str, ProviderAdapterError> {
    self.field(key).ok_or_else(|| ProviderAdapterError::Unsupported {
      detail: format!("Stored credential is missing the '{key}' field. Re-save this provider's credential in Setup."),
    })
  }

  /// The value for a provider that needs exactly one field.
  pub fn single(&self) -> Result<&str, ProviderAdapterError> {
    self.require(LEGACY_SINGLE_CREDENTIAL_KEY)
  }
}

/// One credential input a provider needs from the user.
#[derive(Debug, Clone, Serialize)]
pub struct CredentialField {
  /// Stable machine key. This is the key the adapter reads back out of the stored credential, so
  /// renaming one is a breaking change for anyone who already saved a credential.
  pub key: String,
  pub label: String,
  pub placeholder: String,
  /// Whether the UI should mask this input. An AWS access key id is an identifier, not a secret;
  /// the secret access key is.
  pub secret: bool,
  pub help: Option<String>,
  /// Where the provider's console issues this credential, so onboarding can open it in one click.
  /// Set on a provider's first field only. Opened by the backend, never handed to the webview to
  /// open, so the page can never be pointed anywhere else.
  pub token_page_url: Option<String>,
  /// What to tick on that page, in one line: "Linodes: Read/Write."
  pub scope_hint: Option<String>,
}

/// Where a region physically is, in provider-neutral terms.
///
/// Only `slug` is guaranteed. Every geography field is optional because providers add regions
/// faster than a curated table can follow, and rendering an invented city for an unrecognised
/// slug would tell the user they are exiting somewhere they are not. Absent means "not known",
/// and callers must display the slug rather than a guess.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderRegion {
  pub slug: String,
  pub city: Option<String>,
  /// State or province, where it disambiguates for a reader.
  pub area: Option<String>,
  pub country: Option<String>,
  /// ISO-3166 alpha-2, so the UI can render a flag without its own country table.
  pub country_code: Option<String>,
  pub continent: Option<String>,
  /// Where the city is, for the map. Absent for a city the shared table does not list, in which
  /// case the region is listed but not pinned. See `place_coordinates.rs`.
  pub latitude: Option<f64>,
  pub longitude: Option<f64>,
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
      latitude: None,
      longitude: None,
    }
  }
}

/// A provider's own view of a node, normalized.
///
/// The variants are DigitalOcean's droplet states, kept because they turned out to describe every
/// provider's node adequately — each adapter maps its own vocabulary onto them.
///
/// The names and their `as_str` values are a UI contract: `attach_live_provider_status` serializes
/// them straight to the fleet list, so renaming one would change what the dashboard receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderNodeStatus {
  New,
  Active,
  Off,
  Archive,
  Deleted,
  Unknown,
}

impl ProviderNodeStatus {
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
    "blink-node".to_string()
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
    Self::for_base_url("https://api.digitalocean.com")
  }

  /// Construct against a specific API base URL so tests can point at a local mock server.
  #[cfg(test)]
  fn with_base_url(base_url: &str) -> Result<Self, ProviderAdapterError> {
    Self::for_base_url(base_url)
  }

  fn for_base_url(base_url: &str) -> Result<Self, ProviderAdapterError> {
    let config = DigitalOceanConfig::defaults();
    let client = DigitalOceanApiClient::new_with_timeout(
      base_url,
      RetryPolicy::cloud_default(),
      config.request_timeout,
    )
    .map_err(|source| {
      ProviderAdapterError::api("Unable to initialize DigitalOcean API client.", source)
    })?;

    Ok(Self { client, config })
  }

  /// The bearer token out of the stored credential.
  ///
  /// Credentials are stored as a field set. DigitalOcean needs exactly one field, and a credential
  /// saved before field sets existed is a bare token, which `ProviderCredential::parse` maps onto
  /// the same key.
  fn api_token(stored: &str) -> Result<String, ProviderAdapterError> {
    Ok(ProviderCredential::parse(stored).single()?.to_string())
  }

  /// Delete a droplet this call created but can no longer return, and fold the outcome into the
  /// error the caller sees.
  ///
  /// A failed delete is never swallowed (AGENTS.md §8): if the droplet survives, its id goes into
  /// the message so the user can remove it, rather than being left paying for an invisible droplet.
  fn abandon_droplet(
    &self,
    api_token: &str,
    droplet_id: u64,
    detail: &str,
    cause: DigitalOceanError,
  ) -> ProviderAdapterError {
    match self.client.delete_droplet(api_token, droplet_id) {
      Ok(()) => ProviderAdapterError::api(
        format!("{detail} The droplet was deleted, so nothing is still billing."),
        cause,
      ),
      Err(cleanup_error) => ProviderAdapterError::api(
        format!(
          "{detail} Droplet {droplet_id} could NOT be deleted and may still be billing — remove it in the DigitalOcean console. Cleanup failure: {cleanup_error}"
        ),
        cause,
      ),
    }
  }

  /// DigitalOcean's API keys droplets by a numeric id, so the opaque node id is parsed back at the
  /// adapter boundary. A malformed id is a caller error, not a panic.
  fn droplet_id(node_id: &str) -> Result<u64, ProviderAdapterError> {
    node_id
      .trim()
      .parse::<u64>()
      .map_err(|_| ProviderAdapterError::Unsupported {
        detail: format!(
          "'{node_id}' is not a DigitalOcean droplet id. The node record may belong to another provider."
        ),
      })
  }
}

impl ProviderAdapter for DigitalOceanProviderAdapter {
  fn provider_name(&self) -> &'static str {
    "digitalocean"
  }

  fn credential_fields(&self) -> Vec<CredentialField> {
    vec![CredentialField {
      key: "api_token".to_string(),
      label: "API token".to_string(),
      placeholder: "dop_v1_...".to_string(),
      secret: true,
      help: Some("Needs read and write access to Droplets.".to_string()),
      token_page_url: Some("https://cloud.digitalocean.com/account/api/tokens".to_string()),
      scope_hint: Some("Full Access, or a custom token that can create, read and delete Droplets and tags.".to_string()),
    }]
  }

  fn validate_token(&self, token: &str) -> Result<ProviderIdentity, ProviderAdapterError> {
    let account = self
      .client
      .validate_token(&Self::api_token(token)?)
      .map_err(|source| ProviderAdapterError::api("DigitalOcean token validation request failed.", source))?;

    Ok(ProviderIdentity {
      account_id: account.uuid,
      label: Some(account.email),
      // DigitalOcean is the one provider that reports account standing, and it matters: a
      // `locked` account accepts this token and then refuses to create a droplet.
      status: Some(account.status),
    })
  }

  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError> {
    let mut sizes = self
      .client
      .list_sizes(&Self::api_token(token)?)
      .map_err(|source| ProviderAdapterError::api("DigitalOcean size catalog request failed.", source))?;

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
          monthly_usd: Some(size.price_monthly),
          hourly_usd: Some(size.price_hourly),
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
      .list_sizes(&Self::api_token(token)?)
      .map_err(|source| ProviderAdapterError::api("DigitalOcean size catalog request failed.", source))?;

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

  fn known_regions(&self) -> Vec<ProviderRegion> {
    digitalocean_regions::known_regions()
  }

  fn init_delivery(&self) -> InitDelivery {
    InitDelivery::CloudInit
  }

  fn create_node(
    &self,
    token: &str,
    node_name: &str,
    region: &str,
    size_slug: &str,
    init_payload: &str,
  ) -> Result<ProvisionedNode, ProviderAdapterError> {
    let request = CreateDropletRequest {
      name: sanitize_droplet_name(node_name),
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
      tags: vec!["blink".to_string()],
      user_data: Some(init_payload.to_string()),
    };

    // Resolved once and reused for every call below. Passing the raw stored value to the client
    // sends the whole JSON credential object as the bearer token, which DigitalOcean answers with
    // 401 — see `abandon_droplet` for what that cost when it happened mid-poll.
    let api_token = Self::api_token(token)?;

    let created = self
      .client
      .create_droplet(&api_token, &request)
      .map_err(|source| ProviderAdapterError::api("DigitalOcean droplet create call failed.", source))?;

    // Past this line a billable droplet exists. The orchestrator's rollback only fires once
    // `create_node` has *returned* a node id, so a failure from here on has to delete the droplet
    // itself or leave the user paying for one the app never recorded.
    let active = match self.client.wait_for_droplet_active(
      &api_token,
      created.id,
      self.config.poll_interval,
      self.config.poll_max_attempts,
      self.config.droplet_ready_timeout,
    ) {
      Ok(active) => active,
      Err(source) => {
        return Err(self.abandon_droplet(
          &api_token,
          created.id,
          "DigitalOcean droplet never reached active state.",
          source,
        ));
      }
    };

    let public_ipv4 = match self.client.extract_public_ipv4(&active) {
      Ok(public_ipv4) => public_ipv4,
      Err(source) => {
        return Err(self.abandon_droplet(
          &api_token,
          created.id,
          "Active droplet did not expose public IPv4.",
          source,
        ));
      }
    };

    Ok(ProvisionedNode {
      node_id: active.id.to_string(),
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
      .get_droplet(&Self::api_token(token)?, Self::droplet_id(&request.node_id)?)
      .map_err(|source| {
        ProviderAdapterError::api(
          "Could not confirm DigitalOcean droplet state before completing provisioning.",
          source,
        )
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
        .map_err(|source| ProviderAdapterError::api("Active droplet did not expose public IPv4.", source))?
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

  fn node_exists(&self, token: &str, node_id: &str) -> Result<bool, ProviderAdapterError> {
    match self.client.get_droplet(&Self::api_token(token)?, Self::droplet_id(node_id)?) {
      Ok(_) => Ok(true),
      Err(DigitalOceanError::NotFound { .. }) => Ok(false),
      Err(source) => Err(ProviderAdapterError::api(
        "DigitalOcean droplet existence check failed.",
        source,
      )),
    }
  }

  fn node_status(&self, token: &str, node_id: &str) -> Result<ProviderNodeStatus, ProviderAdapterError> {
    match self.client.get_droplet(&Self::api_token(token)?, Self::droplet_id(node_id)?) {
      Ok(droplet) => Ok(match droplet.status.trim().to_ascii_lowercase().as_str() {
        "new" => ProviderNodeStatus::New,
        "active" => ProviderNodeStatus::Active,
        "off" => ProviderNodeStatus::Off,
        "archive" => ProviderNodeStatus::Archive,
        _ => ProviderNodeStatus::Unknown,
      }),
      Err(DigitalOceanError::NotFound { .. }) => Ok(ProviderNodeStatus::Deleted),
      Err(source) => Err(ProviderAdapterError::api(
        "DigitalOcean droplet status check failed.",
        source,
      )),
    }
  }

  fn delete_node(&self, token: &str, node_id: &str) -> Result<(), ProviderAdapterError> {
    self
      .client
      .delete_droplet(&Self::api_token(token)?, Self::droplet_id(node_id)?)
      .map_err(|source| ProviderAdapterError::api("DigitalOcean droplet delete call failed.", source))
  }
}

/// Order two optional prices cheapest first, sorting unpriced plans last.
///
/// A plan whose provider publishes no price is not free, so it must not lead a list sorted by
/// cost — it simply has no place in that ordering, and the end is the least misleading one.
fn cheapest_first(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
  match (left, right) {
    (Some(left), Some(right)) => left.total_cmp(&right),
    (Some(_), None) => std::cmp::Ordering::Less,
    (None, Some(_)) => std::cmp::Ordering::Greater,
    (None, None) => std::cmp::Ordering::Equal,
  }
}

/// Linode labels: 3–64 characters, alphanumerics plus `-`, `_` and `.`, and no leading or
/// trailing separator. Distinct from `sanitize_droplet_name` because the rule is Linode's, and a
/// shared sanitizer would encode one provider's constraints for every other (AGENTS.md §3).
fn sanitize_linode_label(raw: &str) -> String {
  let mut sanitized = String::new();
  let mut last_was_separator = false;

  for character in raw.trim().chars() {
    if character.is_ascii_alphanumeric() {
      sanitized.push(character);
      last_was_separator = false;
    } else if !last_was_separator {
      sanitized.push('-');
      last_was_separator = true;
    }
  }

  let trimmed = sanitized.trim_matches(|character| character == '-' || character == '.');
  let bounded: String = trimmed.chars().take(64).collect();
  let bounded = bounded.trim_end_matches(|character| character == '-' || character == '.');

  // Also covers the 3-character minimum: anything shorter would be rejected at create.
  if bounded.len() < 3 {
    "blink-node".to_string()
  } else {
    bounded.to_string()
  }
}

/// A single-use root password for a Linode deploy.
///
/// The create call requires either `root_pass` or `authorized_keys`, and the app has no SSH key to
/// offer (AGENTS.md §2). This value is generated, sent once, and dropped — never stored, never
/// logged, and never usable for login, because the bootstrap disables sshd on first boot.
fn throwaway_root_password() -> String {
  format!(
    "Gs1!{}{}",
    uuid::Uuid::new_v4().simple(),
    uuid::Uuid::new_v4().simple()
  )
}

pub struct LinodeProviderAdapter {
  client: LinodeApiClient,
  config: LinodeConfig,
}

impl LinodeProviderAdapter {
  /// Infallible by construction, following `Ec2ProviderAdapter::new` rather than
  /// `DigitalOceanProviderAdapter::new`, whose `Result` the registry unwraps with a panic.
  pub fn new() -> Self {
    Self::for_base_url("https://api.linode.com")
  }

  /// Construct against a specific API base URL so tests can point at a local mock server.
  #[cfg(test)]
  fn with_base_url(base_url: &str) -> Self {
    Self::for_base_url(base_url)
  }

  fn for_base_url(base_url: &str) -> Self {
    let config = LinodeConfig::defaults();
    let client =
      LinodeApiClient::new_with_timeout(base_url, RetryPolicy::cloud_default(), config.request_timeout)
        .unwrap_or_else(|_| {
          // A blocking client build failure is not recoverable here and must not panic at
          // registry construction; the retry-free client surfaces it as a normal API error.
          LinodeApiClient::new_with_timeout(base_url, RetryPolicy::cloud_default(), Duration::from_secs(20))
            .expect("linode client should build with default timeout")
        });

    Self { client, config }
  }

  /// The bearer token out of the stored credential. Linode needs exactly one field, so a
  /// credential saved before field sets existed maps onto the same key.
  fn api_token(stored: &str) -> Result<String, ProviderAdapterError> {
    Ok(ProviderCredential::parse(stored).single()?.to_string())
  }

  /// Linode keys instances by a numeric id, so the opaque node id is parsed back at the adapter
  /// boundary. A malformed id is a caller error, not a panic.
  fn instance_id(node_id: &str) -> Result<u64, ProviderAdapterError> {
    node_id
      .trim()
      .parse::<u64>()
      .map_err(|_| ProviderAdapterError::Unsupported {
        detail: format!(
          "'{node_id}' is not a Linode instance id. The node record may belong to another provider."
        ),
      })
  }

  /// Delete an instance this call created but can no longer return, folding the outcome into the
  /// error the caller sees.
  ///
  /// A failed delete is never swallowed (AGENTS.md §8): if the instance survives, its id goes into
  /// the message so the user can remove it rather than paying for one the app never recorded.
  fn abandon_instance(
    &self,
    api_token: &str,
    instance_id: u64,
    detail: &str,
    cause: LinodeError,
  ) -> ProviderAdapterError {
    match self.client.delete_instance(api_token, instance_id) {
      Ok(()) => ProviderAdapterError::api(
        format!("{detail} The Linode was deleted, so nothing is still billing."),
        cause,
      ),
      Err(cleanup_error) => ProviderAdapterError::api(
        format!(
          "{detail} Linode {instance_id} could NOT be deleted and may still be billing — remove it in the Linode console. Cleanup failure: {cleanup_error}"
        ),
        cause,
      ),
    }
  }
}

impl ProviderAdapter for LinodeProviderAdapter {
  fn provider_name(&self) -> &'static str {
    "linode"
  }

  fn credential_fields(&self) -> Vec<CredentialField> {
    vec![CredentialField {
      key: LEGACY_SINGLE_CREDENTIAL_KEY.to_string(),
      label: "Personal access token".to_string(),
      placeholder: "linode_...".to_string(),
      secret: true,
      help: Some("Needs read/write access to Linodes.".to_string()),
      token_page_url: Some("https://cloud.linode.com/profile/tokens".to_string()),
      scope_hint: Some("Linodes: Read/Write.".to_string()),
    }]
  }

  fn validate_token(&self, token: &str) -> Result<ProviderIdentity, ProviderAdapterError> {
    let account = self
      .client
      .get_account(&Self::api_token(token)?)
      .map_err(|source| ProviderAdapterError::api("Linode token validation request failed.", source))?;

    Ok(ProviderIdentity {
      account_id: account.euuid,
      label: Some(account.email),
      // Linode's account endpoint reports no standing, so none is claimed.
      status: None,
    })
  }

  fn list_regions(&self, token: &str) -> Result<Vec<ProviderRegion>, ProviderAdapterError> {
    let regions = self
      .client
      .list_regions(&Self::api_token(token)?)
      .map_err(|source| ProviderAdapterError::api("Linode region catalog request failed.", source))?;

    // Only regions running the Metadata service can be offered: elsewhere `user_data` is accepted
    // and silently ignored, which provisions a healthy-looking server that never bootstrapped.
    let mut slugs = BTreeSet::new();
    for region in regions
      .into_iter()
      .filter(|region| region.supports_metadata() && !region.id.trim().is_empty())
    {
      slugs.insert(region.id);
    }

    Ok(slugs.iter().map(|slug| self.describe_region(slug)).collect())
  }

  fn describe_region(&self, slug: &str) -> ProviderRegion {
    linode_regions::describe_region(slug)
  }

  fn known_regions(&self) -> Vec<ProviderRegion> {
    linode_regions::known_regions()
  }

  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError> {
    let types = self
      .client
      .list_types(&Self::api_token(token)?)
      .map_err(|source| ProviderAdapterError::api("Linode type catalog request failed.", source))?;

    let fetched_at_epoch_seconds =
      match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(value) => value.as_secs().to_string(),
        Err(_) => "0".to_string(),
      };

    // Shared-CPU plans only. Dedicated/GPU/premium classes exist but are far past what a personal
    // WireGuard endpoint needs, and listing them invites an expensive mistake.
    let mut pricing: Vec<ProviderPlanPricing> = types
      .into_iter()
      .filter(|entry| entry.class == "nanode" || entry.class == "standard")
      .map(|entry| {
        let (monthly_usd, hourly_usd) = entry.price_for_region(region);
        let transfer_tb = entry.transfer as f64 / 1000.0;
        let plan_label = if entry.label.trim().is_empty() {
          entry.id.to_uppercase()
        } else {
          entry.label.trim().to_string()
        };

        ProviderPlanPricing {
          plan_key: entry.id.clone(),
          plan_label,
          size_slug: entry.id,
          region: region.to_string(),
          memory_mb: entry.memory,
          vcpus: entry.vcpus,
          // Linode reports disk in MB; every other adapter reports GB.
          disk_gb: entry.disk / 1024,
          transfer_tb,
          transfer_note: format!(
            "Includes up to {transfer_tb:.2} TB transfer before provider overage policies apply."
          ),
          monthly_usd: Some(monthly_usd),
          hourly_usd: Some(hourly_usd),
          // Live prices, including any regional override.
          estimated: false,
          source_timestamp: fetched_at_epoch_seconds.clone(),
        }
      })
      .collect();

    pricing.sort_by(|left, right| {
      cheapest_first(left.monthly_usd, right.monthly_usd)
        .then_with(|| left.size_slug.cmp(&right.size_slug))
    });

    Ok(pricing)
  }

  fn init_delivery(&self) -> InitDelivery {
    InitDelivery::CloudInit
  }

  fn create_node(
    &self,
    token: &str,
    node_name: &str,
    region: &str,
    size_slug: &str,
    init_payload: &str,
  ) -> Result<ProvisionedNode, ProviderAdapterError> {
    // The API takes user-data base64-encoded, and documents its limit on the encoded form — so
    // the check has to happen after encoding, not before.
    let user_data = base64_encode(init_payload.as_bytes());
    if user_data.len() > LINODE_USER_DATA_LIMIT_BYTES {
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "The bootstrap payload is {} bytes once encoded, over Linode's {LINODE_USER_DATA_LIMIT_BYTES}-byte user-data limit.",
          user_data.len()
        ),
      });
    }

    let request = CreateLinodeRequest {
      label: sanitize_linode_label(node_name),
      region: if region.trim().is_empty() {
        self.config.region.to_string()
      } else {
        region.trim().to_string()
      },
      instance_type: if size_slug.trim().is_empty() {
        self.config.instance_type.to_string()
      } else {
        size_slug.trim().to_string()
      },
      image: self.config.image.to_string(),
      root_pass: throwaway_root_password(),
      booted: true,
      tags: vec!["blink".to_string()],
      metadata: LinodeMetadata { user_data },
    };

    let api_token = Self::api_token(token)?;

    let created = self
      .client
      .create_instance(&api_token, &request)
      .map_err(|source| ProviderAdapterError::api("Linode instance create call failed.", source))?;

    // Past this line a billable instance exists. The orchestrator's rollback only fires once
    // `create_node` has *returned* a node id, so a failure from here on must delete the instance
    // itself or leave the user paying for one the app never recorded.
    let running = match self.client.wait_for_instance_running(
      &api_token,
      created.id,
      self.config.poll_interval,
      self.config.poll_max_attempts,
      self.config.instance_ready_timeout,
    ) {
      Ok(running) => running,
      Err(source) => {
        return Err(self.abandon_instance(
          &api_token,
          created.id,
          "Linode instance never reached running state.",
          source,
        ));
      }
    };

    let public_ipv4 = match self.client.extract_public_ipv4(&running) {
      Ok(public_ipv4) => public_ipv4,
      Err(source) => {
        return Err(self.abandon_instance(
          &api_token,
          created.id,
          "Running Linode did not expose a public IPv4.",
          source,
        ));
      }
    };

    Ok(ProvisionedNode {
      node_id: created.id.to_string(),
      public_ipv4,
      endpoint_port: DEFAULT_WIREGUARD_PORT,
    })
  }

  /// Readiness is "the instance is running and has a public IPv4".
  ///
  /// Nothing is read off the server: the server keypair was generated locally and baked into the
  /// payload, so its public key is already known here (AGENTS.md §2).
  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
    let instance = self
      .client
      .get_instance(&Self::api_token(token)?, Self::instance_id(&request.node_id)?)
      .map_err(|source| {
        ProviderAdapterError::api(
          "Could not confirm Linode instance state before completing provisioning.",
          source,
        )
      })?;

    if !instance.status.trim().eq_ignore_ascii_case("running") {
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "Linode instance is '{}' rather than running; provisioning cannot complete.",
          instance.status
        ),
      });
    }

    let public_ipv4 = if request.endpoint_ip.trim().is_empty() {
      self
        .client
        .extract_public_ipv4(&instance)
        .map_err(|source| {
          ProviderAdapterError::api("Running Linode did not expose a public IPv4.", source)
        })?
    } else {
      request.endpoint_ip.clone()
    };

    Ok(BootstrapOutputContract {
      server_endpoint_ip: public_ipv4,
      // Echoed from what the caller generated and baked into the payload, not read back.
      server_public_key: request.server_keypair.public_key.clone(),
      listen_port: DEFAULT_WIREGUARD_PORT,
    })
  }

  fn node_status(
    &self,
    token: &str,
    node_id: &str,
  ) -> Result<ProviderNodeStatus, ProviderAdapterError> {
    match self
      .client
      .get_instance(&Self::api_token(token)?, Self::instance_id(node_id)?)
    {
      Ok(instance) => Ok(match instance.status.trim().to_ascii_lowercase().as_str() {
        "provisioning" | "booting" | "migrating" | "rebuilding" | "cloning" | "restoring" => {
          ProviderNodeStatus::New
        }
        "running" => ProviderNodeStatus::Active,
        "offline" | "stopped" | "shutting_down" | "rebooting" => ProviderNodeStatus::Off,
        "deleting" => ProviderNodeStatus::Archive,
        _ => ProviderNodeStatus::Unknown,
      }),
      Err(LinodeError::NotFound { .. }) => Ok(ProviderNodeStatus::Deleted),
      Err(source) => Err(ProviderAdapterError::api(
        "Linode instance status check failed.",
        source,
      )),
    }
  }

  fn node_exists(&self, token: &str, node_id: &str) -> Result<bool, ProviderAdapterError> {
    match self
      .client
      .get_instance(&Self::api_token(token)?, Self::instance_id(node_id)?)
    {
      Ok(_) => Ok(true),
      Err(LinodeError::NotFound { .. }) => Ok(false),
      Err(source) => Err(ProviderAdapterError::api(
        "Linode instance existence check failed.",
        source,
      )),
    }
  }

  fn delete_node(&self, token: &str, node_id: &str) -> Result<(), ProviderAdapterError> {
    self
      .client
      .delete_instance(&Self::api_token(token)?, Self::instance_id(node_id)?)
      .map_err(|source| ProviderAdapterError::api("Linode instance delete call failed.", source))
  }
}

/// Credential field keys for OpenStack. The secret is the only one redacted: an auth URL and a
/// credential id name which cloud and which key are in use, which is a diagnostic (AGENTS.md §6).
const OPENSTACK_AUTH_URL_KEY: &str = "auth_url";
const OPENSTACK_CREDENTIAL_ID_KEY: &str = "application_credential_id";
const OPENSTACK_CREDENTIAL_SECRET_KEY: &str = "application_credential_secret";

/// Nova server names are permissive — 255 characters, and no character class it rejects outright.
/// Control characters are still stripped, because they would travel into logs and console output.
fn sanitize_openstack_server_name(raw: &str) -> String {
  let cleaned: String = raw
    .trim()
    .chars()
    .filter(|character| !character.is_control())
    .take(200)
    .collect();

  let cleaned = cleaned.trim();
  let stem = if cleaned.is_empty() { "blink-node" } else { cleaned };
  let suffix = uuid::Uuid::new_v4().simple().to_string();

  format!("{stem}-{}", &suffix[..8])
}

/// Security group name for a node. Uniqueness matters: a group is addressed by name at create, and
/// two nodes sharing one would make teardown of either strand the other.
fn openstack_security_group_name(server_name: &str) -> String {
  let bounded: String = server_name.chars().take(200).collect();
  format!("blink-{bounded}")
}

pub struct OpenStackProviderAdapter {
  client: OpenStackApiClient,
  config: OpenStackConfig,
}

impl OpenStackProviderAdapter {
  /// Infallible by construction, following the EC2, Linode and GCE adapters.
  pub fn new() -> Self {
    let config = OpenStackConfig::defaults();
    let client =
      OpenStackApiClient::new_with_timeout(RetryPolicy::cloud_default(), config.request_timeout)
        .unwrap_or_else(|_| {
          OpenStackApiClient::new_with_timeout(RetryPolicy::cloud_default(), Duration::from_secs(20))
            .expect("openstack client should build with default timeout")
        });

    Self { client, config }
  }

  /// The adapter with polling intervals short enough for a mock server.
  ///
  /// There is no `with_base_url` here, unlike every other adapter: OpenStack has no fixed endpoint
  /// to override, because the auth URL is part of the credential and every other address comes
  /// from the catalogue. What *does* need overriding is the timing — teardown waits for a port to
  /// be released, and against a mock that never stops reporting the server, the real 180-second
  /// dependency timeout turns each test into a three-minute wait.
  #[cfg(test)]
  fn with_fast_polling() -> Self {
    let mut adapter = Self::new();
    adapter.config.poll_interval = Duration::from_millis(1);
    adapter.config.poll_max_attempts = 3;
    adapter.config.server_ready_timeout = Duration::from_millis(50);
    adapter.config.dependency_release_timeout = Duration::from_millis(50);
    adapter
  }

  fn credentials(stored: &str) -> Result<OpenStackCredentials, ProviderAdapterError> {
    let credential = ProviderCredential::parse(stored);

    Ok(OpenStackCredentials {
      auth_url: credential.require(OPENSTACK_AUTH_URL_KEY)?.to_string(),
      application_credential_id: credential.require(OPENSTACK_CREDENTIAL_ID_KEY)?.to_string(),
      application_credential_secret: credential
        .require(OPENSTACK_CREDENTIAL_SECRET_KEY)?
        .to_string(),
    })
  }

  /// Nova scopes a server to a region, and the trait's status, exists and delete methods carry
  /// none — so it travels inside the node id, as EC2 and GCE do.
  ///
  /// The security group and any floating IP are deliberately *not* encoded. They are rediscovered
  /// at delete time, so the id stays honest if an operator removes one by hand.
  fn join_node_id(region: &str, server_id: &str) -> String {
    format!("{region}/{server_id}")
  }

  fn split_node_id(node_id: &str) -> Result<(String, String), ProviderAdapterError> {
    node_id
      .trim()
      .split_once('/')
      .filter(|(region, id)| !region.trim().is_empty() && !id.trim().is_empty())
      .map(|(region, id)| (region.trim().to_string(), id.trim().to_string()))
      .ok_or_else(|| ProviderAdapterError::Unsupported {
        detail: format!(
          "'{node_id}' is not an OpenStack node reference. Expected '<region>/<server-id>'; the node record may belong to another provider."
        ),
      })
  }

  /// Map a client error onto the adapter's own error type, keeping capability gaps distinct.
  ///
  /// A cloud that simply does not offer something is not a failure to retry — it is a fact the
  /// user has to act on, and the UI keys recovery off `Unsupported` rather than message text.
  fn adapt_error(detail: &str, source: OpenStackError) -> ProviderAdapterError {
    match source {
      OpenStackError::Capability { detail } => ProviderAdapterError::Unsupported { detail },
      other => ProviderAdapterError::api(detail.to_string(), other),
    }
  }

  /// Remove what this call created but can no longer return, folding every outcome into the error
  /// the caller sees.
  ///
  /// Ordered: the server first, because it is the billable thing, then its floating IP, then the
  /// security group — which cannot be deleted until the port referencing it is gone. Anything that
  /// survives is named (AGENTS.md §8).
  fn abandon_server(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    server_id: Option<&str>,
    floating_ip_id: Option<&str>,
    security_group_id: Option<&str>,
    detail: &str,
    cause: OpenStackError,
  ) -> ProviderAdapterError {
    let mut leftovers = Vec::new();

    if let Some(server_id) = server_id {
      match self.client.delete_server(credentials, region, server_id) {
        Ok(()) => {
          // The group cannot go until the server's port is released, so the wait is not optional.
          let _ = self.client.wait_for_server_gone(
            credentials,
            region,
            server_id,
            self.config.poll_interval,
            self.config.dependency_release_timeout,
          );
        }
        Err(error) => leftovers.push(format!(
          "server '{server_id}' could NOT be deleted and may still be billing ({error})"
        )),
      }
    }

    if let Some(floating_ip_id) = floating_ip_id {
      if let Err(error) = self.client.release_floating_ip(credentials, region, floating_ip_id) {
        leftovers.push(format!(
          "floating IP '{floating_ip_id}' could NOT be released and may still be billing ({error})"
        ));
      }
    }

    if let Some(security_group_id) = security_group_id {
      if let Err(error) = self.delete_security_group_when_free(credentials, region, security_group_id)
      {
        leftovers.push(format!(
          "security group '{security_group_id}' could NOT be removed ({error})"
        ));
      }
    }

    if leftovers.is_empty() {
      ProviderAdapterError::api(
        format!("{detail} Everything created was removed, so nothing is still billing."),
        cause,
      )
    } else {
      ProviderAdapterError::api(
        format!("{detail} Clean up in your cloud's dashboard: {}.", leftovers.join("; ")),
        cause,
      )
    }
  }

  /// Delete a security group, retrying while its port is still being released.
  ///
  /// Neutron answers 409 until the server's port is gone, which is the same dependency dance EC2
  /// runs for a security group and its network interface.
  fn delete_security_group_when_free(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    security_group_id: &str,
  ) -> Result<(), OpenStackError> {
    let policy = self.client.retry_policy();
    let started = std::time::Instant::now();

    for attempt in 0..=policy.max_retries {
      match self.client.delete_security_group(credentials, region, security_group_id) {
        Ok(()) => return Ok(()),
        Err(OpenStackError::Conflict { .. })
          if started.elapsed() < self.config.dependency_release_timeout =>
        {
          std::thread::sleep(policy.backoff_for_attempt(attempt));
        }
        Err(error) => return Err(error),
      }
    }

    self.client.delete_security_group(credentials, region, security_group_id)
  }
}

impl ProviderAdapter for OpenStackProviderAdapter {
  fn provider_name(&self) -> &'static str {
    "openstack"
  }

  fn credential_fields(&self) -> Vec<CredentialField> {
    vec![
      CredentialField {
        key: OPENSTACK_AUTH_URL_KEY.to_string(),
        label: "Keystone auth URL".to_string(),
        placeholder: "https://identity.example.com/v3".to_string(),
        // Names which cloud is in use — a diagnostic, not a secret.
        secret: false,
        help: Some("The identity endpoint for your cloud, ending in /v3.".to_string()),
        token_page_url: None,
        scope_hint: None,
      },
      CredentialField {
        key: OPENSTACK_CREDENTIAL_ID_KEY.to_string(),
        label: "Application credential ID".to_string(),
        placeholder: "8ab1...".to_string(),
        secret: false,
        help: Some(
          "Create an application credential in your cloud's dashboard rather than using your own password — it is scoped and can be revoked on its own."
            .to_string(),
        ),
        token_page_url: None,
        scope_hint: None,
      },
      CredentialField {
        key: OPENSTACK_CREDENTIAL_SECRET_KEY.to_string(),
        label: "Application credential secret".to_string(),
        placeholder: String::new(),
        secret: true,
        help: Some("Shown once when the credential is created.".to_string()),
        token_page_url: None,
        scope_hint: None,
      },
    ]
  }

  fn validate_token(&self, token: &str) -> Result<ProviderIdentity, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;

    // Authenticating *is* the validation: it proves the credential works and that the catalogue
    // carries the services this app needs.
    let project_id = self
      .client
      .project_id(&credentials)
      .map_err(|source| Self::adapt_error("OpenStack authentication failed.", source))?;

    Ok(ProviderIdentity {
      account_id: project_id,
      // The credential id, not a person: an application credential belongs to a project rather
      // than to a named user, so there is no email to report.
      label: Some(credentials.application_credential_id.clone()),
      status: None,
    })
  }

  fn list_regions(&self, token: &str) -> Result<Vec<ProviderRegion>, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let regions = self
      .client
      .regions(&credentials)
      .map_err(|source| Self::adapt_error("OpenStack region lookup failed.", source))?;

    Ok(regions.iter().map(|slug| self.describe_region(slug)).collect())
  }

  fn describe_region(&self, slug: &str) -> ProviderRegion {
    openstack_regions::describe_region(slug)
  }

  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let mut flavors = self
      .client
      .list_flavors(&credentials, region)
      .map_err(|source| Self::adapt_error("OpenStack flavor list request failed.", source))?;

    flavors.retain(|flavor| !flavor.disabled && flavor.is_public);
    flavors.sort_by(|left, right| left.ram.cmp(&right.ram).then_with(|| left.name.cmp(&right.name)));

    let fetched_at_epoch_seconds =
      match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(value) => value.as_secs().to_string(),
        Err(_) => "0".to_string(),
      };

    Ok(
      flavors
        .into_iter()
        .map(|flavor| ProviderPlanPricing {
          plan_key: flavor.id.clone(),
          plan_label: if flavor.name.trim().is_empty() {
            flavor.id.clone()
          } else {
            flavor.name.clone()
          },
          // Nova launches by flavor *id*; the name is a label and is not guaranteed unique.
          size_slug: flavor.id,
          region: region.to_string(),
          memory_mb: flavor.ram,
          vcpus: flavor.vcpus,
          disk_gb: flavor.disk,
          transfer_tb: 0.0,
          transfer_note:
            "Your cloud operator sets the price and any transfer allowance for this flavor — OpenStack itself publishes neither."
              .to_string(),
          // Not zero: nothing in the API knows what an operator charges, and a rendered 0.00 would
          // say this flavor is free.
          monthly_usd: None,
          hourly_usd: None,
          estimated: true,
          source_timestamp: fetched_at_epoch_seconds.clone(),
        })
        .collect(),
    )
  }

  fn init_delivery(&self) -> InitDelivery {
    InitDelivery::CloudInit
  }

  fn create_node(
    &self,
    token: &str,
    node_name: &str,
    region: &str,
    size_slug: &str,
    init_payload: &str,
  ) -> Result<ProvisionedNode, ProviderAdapterError> {
    let user_data = base64_encode(init_payload.as_bytes());
    if user_data.len() > OPENSTACK_USER_DATA_LIMIT_BYTES {
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "The bootstrap payload is {} bytes once encoded, over Nova's {OPENSTACK_USER_DATA_LIMIT_BYTES}-byte user-data limit.",
          user_data.len()
        ),
      });
    }

    // There is no default flavor to fall back on: flavor ids are operator-defined, so any guess
    // would be wrong on most clouds.
    if size_slug.trim().is_empty() {
      return Err(ProviderAdapterError::Unsupported {
        detail: "Choose a plan before provisioning. OpenStack clouds define their own flavors, so there is no default to fall back on."
          .to_string(),
      });
    }

    let credentials = Self::credentials(token)?;
    let region = region.trim().to_string();

    let image_id = self
      .client
      .resolve_ubuntu_image(&credentials, &region)
      .map_err(|source| Self::adapt_error("OpenStack image lookup failed.", source))?;

    let (network_id, direct_public) = self
      .client
      .resolve_network(&credentials, &region)
      .map_err(|source| Self::adapt_error("OpenStack network lookup failed.", source))?;

    let server_name = sanitize_openstack_server_name(node_name);
    let group_name = openstack_security_group_name(&server_name);

    let security_group_id = self
      .client
      .create_security_group(&credentials, &region, &group_name)
      .map_err(|source| {
        Self::adapt_error("OpenStack security group create call failed.", source)
      })?;

    if let Err(source) =
      self
        .client
        .authorize_udp_ingress(&credentials, &region, &security_group_id, DEFAULT_WIREGUARD_PORT)
    {
      return Err(self.abandon_server(
        &credentials,
        &region,
        None,
        None,
        Some(&security_group_id),
        "Could not open the WireGuard port on the new security group.",
        source,
      ));
    }

    let server_id = match self.client.create_server(
      &credentials,
      &region,
      &server_name,
      &image_id,
      size_slug.trim(),
      &network_id,
      &group_name,
      &user_data,
    ) {
      Ok(server_id) => server_id,
      Err(source) => {
        return Err(self.abandon_server(
          &credentials,
          &region,
          None,
          None,
          Some(&security_group_id),
          "OpenStack server create call failed.",
          source,
        ));
      }
    };

    // Past this line a billable server exists. The orchestrator's rollback only fires once
    // `create_node` has *returned* a node id, so a failure from here on must clean up itself.
    let server = match self.client.wait_for_server_active(
      &credentials,
      &region,
      &server_id,
      self.config.poll_interval,
      self.config.poll_max_attempts,
      self.config.server_ready_timeout,
    ) {
      Ok(server) => server,
      Err(source) => {
        return Err(self.abandon_server(
          &credentials,
          &region,
          Some(&server_id),
          None,
          Some(&security_group_id),
          "The OpenStack server never became ACTIVE.",
          source,
        ));
      }
    };

    // Two topologies: some clouds put a routable address straight on the server, others hand out
    // RFC1918 and expect a floating IP allocated afterwards.
    let (public_ipv4, floating_ip_id) = match server.reachable_address() {
      Some(address) => (address, None),
      None if direct_public => {
        return Err(self.abandon_server(
          &credentials,
          &region,
          Some(&server_id),
          None,
          Some(&security_group_id),
          "The server was attached directly to an external network but received no routable address.",
          OpenStackError::Capability {
            detail: "No routable address was assigned.".to_string(),
          },
        ));
      }
      None => {
        let ports = match self.client.ports_for_server(&credentials, &region, &server_id) {
          Ok(ports) => ports,
          Err(source) => {
            return Err(self.abandon_server(
              &credentials,
              &region,
              Some(&server_id),
              None,
              Some(&security_group_id),
              "Could not find the new server's network port to attach a floating IP to.",
              source,
            ));
          }
        };

        let Some(port_id) = ports.first() else {
          return Err(self.abandon_server(
            &credentials,
            &region,
            Some(&server_id),
            None,
            Some(&security_group_id),
            "The new server has no network port, so no address can be attached to it.",
            OpenStackError::Capability {
              detail: "Nova reported no port for this server.".to_string(),
            },
          ));
        };

        match self.client.associate_floating_ip(&credentials, &region, port_id) {
          Ok(floating) => (floating.floating_ip_address, Some(floating.id)),
          Err(source) => {
            return Err(self.abandon_server(
              &credentials,
              &region,
              Some(&server_id),
              None,
              Some(&security_group_id),
              "Could not give the server a publicly reachable address.",
              source,
            ));
          }
        }
      }
    };

    if public_ipv4.trim().is_empty() {
      return Err(self.abandon_server(
        &credentials,
        &region,
        Some(&server_id),
        floating_ip_id.as_deref(),
        Some(&security_group_id),
        "The OpenStack server exposed no usable address.",
        OpenStackError::Capability {
          detail: "The resolved address was empty.".to_string(),
        },
      ));
    }

    Ok(ProvisionedNode {
      node_id: Self::join_node_id(&region, &server_id),
      public_ipv4,
      endpoint_port: DEFAULT_WIREGUARD_PORT,
    })
  }

  /// Readiness is "the server is ACTIVE and has a reachable address".
  ///
  /// Nothing is read off the server: the server keypair was generated locally and baked into the
  /// payload, so its public key is already known here (AGENTS.md §2).
  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let (region, server_id) = Self::split_node_id(&request.node_id)?;

    let server = self
      .client
      .get_server(&credentials, &region, &server_id)
      .map_err(|source| {
        Self::adapt_error(
          "Could not confirm the OpenStack server's state before completing provisioning.",
          source,
        )
      })?;

    if !server.status.eq_ignore_ascii_case("ACTIVE") {
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "The OpenStack server is '{}' rather than ACTIVE; provisioning cannot complete.",
          server.status
        ),
      });
    }

    let public_ipv4 = if request.endpoint_ip.trim().is_empty() {
      server
        .reachable_address()
        .ok_or_else(|| ProviderAdapterError::Unsupported {
          detail: "The OpenStack server exposes no reachable address.".to_string(),
        })?
    } else {
      request.endpoint_ip.clone()
    };

    Ok(BootstrapOutputContract {
      server_endpoint_ip: public_ipv4,
      // Echoed from what the caller generated and baked into the payload, not read back.
      server_public_key: request.server_keypair.public_key.clone(),
      listen_port: DEFAULT_WIREGUARD_PORT,
    })
  }

  fn node_status(
    &self,
    token: &str,
    node_id: &str,
  ) -> Result<ProviderNodeStatus, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let (region, server_id) = Self::split_node_id(node_id)?;

    match self.client.get_server(&credentials, &region, &server_id) {
      Ok(server) => Ok(match server.status.to_ascii_uppercase().as_str() {
        "BUILD" | "REBUILD" | "REBUILD_SPAWNING" => ProviderNodeStatus::New,
        "ACTIVE" => ProviderNodeStatus::Active,
        "SHUTOFF" | "PAUSED" | "SUSPENDED" | "STOPPED" => ProviderNodeStatus::Off,
        "SHELVED" | "SHELVED_OFFLOADED" => ProviderNodeStatus::Archive,
        "DELETED" | "SOFT_DELETED" => ProviderNodeStatus::Deleted,
        _ => ProviderNodeStatus::Unknown,
      }),
      Err(OpenStackError::NotFound { .. }) => Ok(ProviderNodeStatus::Deleted),
      Err(source) => Err(Self::adapt_error("OpenStack server status check failed.", source)),
    }
  }

  fn node_exists(&self, token: &str, node_id: &str) -> Result<bool, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let (region, server_id) = Self::split_node_id(node_id)?;

    match self.client.get_server(&credentials, &region, &server_id) {
      Ok(server) => Ok(!server.status.eq_ignore_ascii_case("DELETED")),
      Err(OpenStackError::NotFound { .. }) => Ok(false),
      Err(source) => Err(Self::adapt_error(
        "OpenStack server existence check failed.",
        source,
      )),
    }
  }

  /// Delete the server, then its floating IP, then its security group.
  ///
  /// The order is forced by Neutron: a security group cannot go while a port references it, and
  /// the port goes with the server. The floating IP and group ids are **rediscovered** rather than
  /// stored, so this still does the right thing if an operator removed one by hand.
  ///
  /// A server that will not delete aborts immediately — it is the billable thing. Anything else
  /// that survives is accumulated and reported by id (AGENTS.md §8).
  fn delete_node(&self, token: &str, node_id: &str) -> Result<(), ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let (region, server_id) = Self::split_node_id(node_id)?;

    // Read what is attached *before* deleting: the server's ports disappear with it, the floating
    // IPs are only findable through them, and its security groups are only named on the server.
    let ports = self
      .client
      .ports_for_server(&credentials, &region, &server_id)
      .unwrap_or_default();
    let floating_ips = self
      .client
      .floating_ips_for_ports(&credentials, &region, &ports)
      .unwrap_or_default();

    // Only groups this app created are candidates — a group the operator attached by hand is
    // theirs, and deleting it would break whatever else uses it.
    let group_names: Vec<String> = self
      .client
      .get_server(&credentials, &region, &server_id)
      .map(|server| {
        server
          .security_groups
          .into_iter()
          .map(|group| group.name)
          .filter(|name| name.starts_with("blink-"))
          .collect()
      })
      .unwrap_or_default();
    let groups = self
      .client
      .security_group_ids_by_name(&credentials, &region, &group_names)
      .unwrap_or_default();

    self
      .client
      .delete_server(&credentials, &region, &server_id)
      .map_err(|source| Self::adapt_error("OpenStack server delete call failed.", source))?;

    // The group cannot be removed until the server's port is released, so this wait is load
    // bearing rather than cosmetic.
    let _ = self.client.wait_for_server_gone(
      &credentials,
      &region,
      &server_id,
      self.config.poll_interval,
      self.config.dependency_release_timeout,
    );

    let mut leftovers = Vec::new();

    for floating in &floating_ips {
      if let Err(error) = self.client.release_floating_ip(&credentials, &region, &floating.id) {
        leftovers.push(format!(
          "floating IP {} ({}) could NOT be released and may still be billing: {error}",
          floating.floating_ip_address, floating.id
        ));
      }
    }

    for (group_id, group_name) in &groups {
      if let Err(error) = self.delete_security_group_when_free(&credentials, &region, group_id) {
        leftovers.push(format!(
          "security group '{group_name}' ({group_id}) could NOT be removed: {error}"
        ));
      }
    }

    if leftovers.is_empty() {
      Ok(())
    } else {
      Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "The server was deleted, but some resources remain — remove them in your cloud's dashboard: {}.",
          leftovers.join("; ")
        ),
      })
    }
  }
}

/// Machine types offered for a Blink node on GCE, cheapest first.
///
/// Google's Cloud Billing Catalog API prices compute by SKU — a large catalogue that has to be
/// reassembled into a per-machine-type figure, which is a heavyweight dependency for three rows.
/// These are on-demand Linux list prices in `us-central1`, reported with `estimated: true` — what
/// that flag exists for. Availability is still checked live per zone, so a type a zone cannot
/// launch is never offered.
///
/// `monthly_usd` includes the external IPv4 charge (~$2.90/mo at $0.004/hour, billed since
/// February 2024), because an instance without one is unreachable and quoting the machine alone
/// would understate the real bill.
const GCE_PLAN_SPECS: &[GcePlanSpec] = &[
  GcePlanSpec {
    machine_type: "e2-micro",
    label: "e2-micro (2 shared vCPU, 1 GB)",
    memory_mb: 1024,
    vcpus: 2,
    monthly_usd: 9.75,
    hourly_usd: 0.013,
  },
  GcePlanSpec {
    machine_type: "e2-small",
    label: "e2-small (2 shared vCPU, 2 GB)",
    memory_mb: 2048,
    vcpus: 2,
    monthly_usd: 16.55,
    hourly_usd: 0.0227,
  },
  GcePlanSpec {
    machine_type: "e2-medium",
    label: "e2-medium (2 shared vCPU, 4 GB)",
    memory_mb: 4096,
    vcpus: 2,
    monthly_usd: 30.14,
    hourly_usd: 0.0413,
  },
];

struct GcePlanSpec {
  machine_type: &'static str,
  label: &'static str,
  memory_mb: u64,
  vcpus: u64,
  monthly_usd: f64,
  hourly_usd: f64,
}

/// GCE instance names: lowercase, `[a-z]([-a-z0-9]{0,61}[a-z0-9])?`.
///
/// A uuid suffix is appended because the name *is* the instance's identity within a project and
/// zone — two nodes a user calls "Primary Node" must not collide, and GCE rejects the second
/// rather than disambiguating it.
fn sanitize_gce_instance_name(raw: &str) -> String {
  let mut sanitized = String::new();
  let mut last_was_separator = false;

  for character in raw.trim().chars() {
    if character.is_ascii_alphanumeric() {
      sanitized.push(character.to_ascii_lowercase());
      last_was_separator = false;
    } else if !last_was_separator {
      sanitized.push('-');
      last_was_separator = true;
    }
  }

  let trimmed = sanitized.trim_matches('-');
  // A name must start with a letter, so a leading digit gets a stem rather than being dropped.
  let stem = if trimmed.is_empty() || trimmed.starts_with(|c: char| c.is_ascii_digit()) {
    format!("blink-{trimmed}")
  } else {
    trimmed.to_string()
  };

  let suffix = uuid::Uuid::new_v4().simple().to_string();
  // 63 total, less the 9 the suffix and its separator take.
  let bounded: String = stem.chars().take(54).collect();

  format!("{}-{}", bounded.trim_end_matches('-'), &suffix[..8])
}

/// The per-node firewall rule name, derived from the instance name so teardown can find it without
/// storing a second identifier.
fn gce_firewall_rule_name(instance_name: &str) -> String {
  let bounded: String = instance_name.chars().take(59).collect();
  format!("{bounded}-wg")
}

pub struct GceProviderAdapter {
  client: GceApiClient,
  config: GceConfig,
}

impl GceProviderAdapter {
  /// Infallible by construction, following the EC2 and Linode adapters.
  pub fn new() -> Self {
    Self::for_base_urls("https://compute.googleapis.com", GCE_TOKEN_URL)
  }

  /// Construct against specific base URLs so tests can point both the compute API and the token
  /// endpoint at one mock server.
  #[cfg(test)]
  fn with_base_url(base_url: &str) -> Self {
    Self::for_base_urls(base_url, &format!("{}/token", base_url.trim_end_matches('/')))
  }

  fn for_base_urls(compute_base_url: &str, token_url: &str) -> Self {
    let config = GceConfig::defaults();
    let client = GceApiClient::new_with_timeout(
      compute_base_url,
      token_url,
      RetryPolicy::cloud_default(),
      config.request_timeout,
    )
    .unwrap_or_else(|_| {
      GceApiClient::new_with_timeout(
        compute_base_url,
        token_url,
        RetryPolicy::cloud_default(),
        Duration::from_secs(20),
      )
      .expect("gce client should build with default timeout")
    });

    Self { client, config }
  }

  /// The service account key out of the stored credential.
  ///
  /// One field holds the whole downloaded JSON key file, and everything else — project id, client
  /// email, the signing key — is read out of it. Asking separately for a project id that is
  /// already in the file invites a mismatch that surfaces as a 404 on a project that does exist.
  fn service_account(stored: &str) -> Result<ServiceAccountKey, ProviderAdapterError> {
    let credential = ProviderCredential::parse(stored);
    let raw = credential.require(GCE_CREDENTIAL_KEY)?;

    ServiceAccountKey::parse(raw).map_err(|source| ProviderAdapterError::Unsupported {
      detail: format!("{source} Re-save the credential in Setup."),
    })
  }

  /// GCE addresses an instance by zone and name, and the trait's status, exists and delete methods
  /// carry no zone — so it travels inside the node id, as EC2 does with its region.
  fn join_node_id(zone: &str, instance_name: &str) -> String {
    format!("{zone}/{instance_name}")
  }

  fn split_node_id(node_id: &str) -> Result<(String, String), ProviderAdapterError> {
    node_id
      .trim()
      .split_once('/')
      .filter(|(zone, name)| !zone.trim().is_empty() && !name.trim().is_empty())
      .map(|(zone, name)| (zone.trim().to_string(), name.trim().to_string()))
      .ok_or_else(|| ProviderAdapterError::Unsupported {
        detail: format!(
          "'{node_id}' is not a Google Compute Engine node reference. Expected '<zone>/<instance-name>'; the node record may belong to another provider."
        ),
      })
  }

  /// Remove resources this call created but can no longer return, folding every outcome into the
  /// error the caller sees.
  ///
  /// Two resources, so two ways to leak. A failed delete is never swallowed (AGENTS.md §8): what
  /// survives is named, so the user can remove it rather than paying for something invisible.
  fn abandon_instance(
    &self,
    key: &ServiceAccountKey,
    zone: &str,
    instance_name: &str,
    firewall_rule: &str,
    detail: &str,
    cause: GceError,
  ) -> ProviderAdapterError {
    let mut leftovers = Vec::new();

    if let Err(error) = self.client.delete_instance(key, zone, instance_name) {
      leftovers.push(format!(
        "instance '{instance_name}' in {zone} could NOT be deleted and may still be billing ({error})"
      ));
    }

    if let Err(error) = self.client.delete_firewall_rule(key, firewall_rule) {
      leftovers.push(format!("firewall rule '{firewall_rule}' could NOT be removed ({error})"));
    }

    if leftovers.is_empty() {
      ProviderAdapterError::api(
        format!("{detail} The instance and its firewall rule were removed, so nothing is still billing."),
        cause,
      )
    } else {
      ProviderAdapterError::api(
        format!(
          "{detail} Clean up in the Google Cloud console: {}.",
          leftovers.join("; ")
        ),
        cause,
      )
    }
  }
}

/// The single credential field: the whole service account key file.
const GCE_CREDENTIAL_KEY: &str = "service_account_json";

impl ProviderAdapter for GceProviderAdapter {
  fn provider_name(&self) -> &'static str {
    "gce"
  }

  fn credential_fields(&self) -> Vec<CredentialField> {
    vec![CredentialField {
      key: GCE_CREDENTIAL_KEY.to_string(),
      label: "Service account key (JSON)".to_string(),
      placeholder: "{\"type\": \"service_account\", ...}".to_string(),
      secret: true,
      help: Some(
        "Paste the whole JSON key file for a service account with the Compute Admin role. The project is read from the file. The project needs billing enabled and the Compute Engine API turned on."
          .to_string(),
      ),
      token_page_url: Some("https://console.cloud.google.com/iam-admin/serviceaccounts".to_string()),
      scope_hint: Some("A JSON key for a service account with the Compute Admin role.".to_string()),
    }]
  }

  fn validate_token(&self, token: &str) -> Result<ProviderIdentity, ProviderAdapterError> {
    let key = Self::service_account(token)?;

    // Reading the project proves three things at once: the key signs, the Compute Engine API is
    // enabled, and the service account can actually see the project it names.
    self
      .client
      .get_project(&key)
      .map_err(|source| ProviderAdapterError::api("Google project lookup failed.", source))?;

    Ok(ProviderIdentity {
      // The project is what Google bills, and what the instance will live in.
      account_id: key.project_id.clone(),
      label: Some(key.client_email.clone()),
      status: None,
    })
  }

  fn list_regions(&self, token: &str) -> Result<Vec<ProviderRegion>, ProviderAdapterError> {
    let key = Self::service_account(token)?;
    let zones = self
      .client
      .list_zones(&key)
      .map_err(|source| ProviderAdapterError::api("Google zone catalog request failed.", source))?;

    // The trait's "region" is a GCE *zone*: an instance is created in one, and mapping regions here
    // would leave the adapter picking a zone on the user's behalf.
    let mut slugs = BTreeSet::new();
    for zone in zones
      .into_iter()
      .filter(|zone| zone.status.eq_ignore_ascii_case("UP") && !zone.name.trim().is_empty())
    {
      slugs.insert(zone.name);
    }

    Ok(slugs.iter().map(|slug| self.describe_region(slug)).collect())
  }

  fn describe_region(&self, slug: &str) -> ProviderRegion {
    gce_regions::describe_region(slug)
  }

  fn known_regions(&self) -> Vec<ProviderRegion> {
    gce_regions::known_regions()
  }

  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError> {
    let key = Self::service_account(token)?;
    let zone = if region.trim().is_empty() {
      self.config.zone.to_string()
    } else {
      region.trim().to_string()
    };

    // Prices are curated, but availability is not: a zone that cannot launch a type must never
    // offer it, and which types a zone has is genuinely per-zone.
    let offered = self
      .client
      .list_machine_types(&key, &zone)
      .map_err(|source| {
        ProviderAdapterError::api("Google machine type availability request failed.", source)
      })?
      .into_iter()
      .map(|machine_type| machine_type.name)
      .collect::<BTreeSet<_>>();

    let fetched_at_epoch_seconds =
      match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(value) => value.as_secs().to_string(),
        Err(_) => "0".to_string(),
      };

    Ok(
      GCE_PLAN_SPECS
        .iter()
        .filter(|spec| offered.contains(spec.machine_type))
        .map(|spec| ProviderPlanPricing {
          plan_key: spec.machine_type.to_string(),
          plan_label: spec.label.to_string(),
          size_slug: spec.machine_type.to_string(),
          region: zone.clone(),
          memory_mb: spec.memory_mb,
          vcpus: spec.vcpus,
          disk_gb: self.config.boot_disk_gb,
          // Google bills egress per GB with no bundled allowance, so there is no transfer figure
          // to quote. Reporting 0 rather than inventing one keeps the note the honest part.
          transfer_tb: 0.0,
          transfer_note:
            "Egress is billed per GB with no included allowance, and the external IPv4 is billed hourly. Both are already in this estimate's monthly figure."
              .to_string(),
          monthly_usd: Some(spec.monthly_usd),
          hourly_usd: Some(spec.hourly_usd),
          estimated: true,
          source_timestamp: fetched_at_epoch_seconds.clone(),
        })
        .collect(),
    )
  }

  fn init_delivery(&self) -> InitDelivery {
    InitDelivery::CloudInit
  }

  fn client_mtu_hint(&self) -> Option<u16> {
    // GCE NICs run at 1460, so WireGuard's default 1420 tunnel MTU is 40 bytes too large.
    Some(GCE_CLIENT_MTU)
  }

  fn create_node(
    &self,
    token: &str,
    node_name: &str,
    region: &str,
    size_slug: &str,
    init_payload: &str,
  ) -> Result<ProvisionedNode, ProviderAdapterError> {
    // GCE takes metadata values verbatim — no base64 step — so the limit applies to the rendered
    // script as it stands.
    if init_payload.len() > GCE_USER_DATA_LIMIT_BYTES {
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "The bootstrap payload is {} bytes, over Google's {GCE_USER_DATA_LIMIT_BYTES}-byte metadata value limit.",
          init_payload.len()
        ),
      });
    }

    let key = Self::service_account(token)?;
    let zone = if region.trim().is_empty() {
      self.config.zone.to_string()
    } else {
      region.trim().to_string()
    };
    let machine_type = if size_slug.trim().is_empty() {
      self.config.machine_type.to_string()
    } else {
      size_slug.trim().to_string()
    };

    // A project whose default network was deleted cannot launch into one. Finding that out from a
    // failed insert produces a far worse message than saying so here.
    if !self
      .client
      .default_network_exists(&key)
      .map_err(|source| ProviderAdapterError::api("Google default network lookup failed.", source))?
    {
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "Project '{}' has no 'default' VPC network, so this app cannot launch an instance into it. Recreate the default network, or use a different project.",
          key.project_id
        ),
      });
    }

    let instance_name = sanitize_gce_instance_name(node_name);
    let firewall_rule = gce_firewall_rule_name(&instance_name);

    // The default network passes no UDP 51820, so without this rule the instance boots, bootstraps
    // correctly, and refuses every handshake. Created first: nothing is billing yet if it fails.
    self
      .client
      .create_firewall_rule(&key, &firewall_rule, &instance_name, DEFAULT_WIREGUARD_PORT)
      .map_err(|source| {
        ProviderAdapterError::api("Google firewall rule create call failed.", source)
      })?;

    let inserted = self.client.insert_instance(
      &key,
      &zone,
      &instance_name,
      &machine_type,
      self.config.image_family,
      self.config.boot_disk_gb,
      &instance_name,
      init_payload,
    );

    if let Err(source) = inserted {
      // Nothing is billing yet, but the rule would linger and block a retry using the same name.
      let cleanup = self.client.delete_firewall_rule(&key, &firewall_rule);
      return Err(match cleanup {
        Ok(()) => ProviderAdapterError::api("Google instance insert call failed.", source),
        Err(error) => ProviderAdapterError::api(
          format!(
            "Google instance insert call failed, and firewall rule '{firewall_rule}' could NOT be removed — delete it in the Google Cloud console. Cleanup failure: {error}"
          ),
          source,
        ),
      });
    }

    // Past this line a billable instance exists. The orchestrator's rollback only fires once
    // `create_node` has *returned* a node id, so a failure from here on must clean up itself.
    let running = match self.client.wait_for_instance_running(
      &key,
      &zone,
      &instance_name,
      self.config.poll_interval,
      self.config.poll_max_attempts,
      self.config.instance_ready_timeout,
    ) {
      Ok(running) => running,
      Err(source) => {
        return Err(self.abandon_instance(
          &key,
          &zone,
          &instance_name,
          &firewall_rule,
          "Google instance never reached RUNNING with an external address.",
          source,
        ));
      }
    };

    let public_ipv4 = match running.external_ipv4() {
      Some(public_ipv4) => public_ipv4,
      None => {
        return Err(self.abandon_instance(
          &key,
          &zone,
          &instance_name,
          &firewall_rule,
          "Running Google instance exposed no external IPv4.",
          GceError::NotFound {
            action: "create_node",
            detail: "The instance's access config carried no natIP.".to_string(),
          },
        ));
      }
    };

    Ok(ProvisionedNode {
      node_id: Self::join_node_id(&zone, &instance_name),
      public_ipv4,
      endpoint_port: DEFAULT_WIREGUARD_PORT,
    })
  }

  /// Readiness is "the instance is RUNNING and has an external IPv4".
  ///
  /// Nothing is read off the server: the server keypair was generated locally and baked into the
  /// payload, so its public key is already known here (AGENTS.md §2).
  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
    let key = Self::service_account(token)?;
    let (zone, instance_name) = Self::split_node_id(&request.node_id)?;

    let instance = self
      .client
      .get_instance(&key, &zone, &instance_name)
      .map_err(|source| {
        ProviderAdapterError::api(
          "Could not confirm Google instance state before completing provisioning.",
          source,
        )
      })?;

    if !instance.status.eq_ignore_ascii_case("RUNNING") {
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "Google instance is '{}' rather than RUNNING; provisioning cannot complete.",
          instance.status
        ),
      });
    }

    let public_ipv4 = if request.endpoint_ip.trim().is_empty() {
      instance
        .external_ipv4()
        .ok_or_else(|| ProviderAdapterError::Unsupported {
          detail: "Running Google instance exposed no external IPv4.".to_string(),
        })?
    } else {
      request.endpoint_ip.clone()
    };

    Ok(BootstrapOutputContract {
      server_endpoint_ip: public_ipv4,
      // Echoed from what the caller generated and baked into the payload, not read back.
      server_public_key: request.server_keypair.public_key.clone(),
      listen_port: DEFAULT_WIREGUARD_PORT,
    })
  }

  fn node_status(
    &self,
    token: &str,
    node_id: &str,
  ) -> Result<ProviderNodeStatus, ProviderAdapterError> {
    let key = Self::service_account(token)?;
    let (zone, instance_name) = Self::split_node_id(node_id)?;

    match self.client.get_instance(&key, &zone, &instance_name) {
      Ok(instance) => Ok(match instance.status.to_ascii_uppercase().as_str() {
        "PROVISIONING" | "STAGING" => ProviderNodeStatus::New,
        "RUNNING" => ProviderNodeStatus::Active,
        "STOPPING" | "STOPPED" | "TERMINATED" | "SUSPENDING" | "SUSPENDED" => {
          ProviderNodeStatus::Off
        }
        _ => ProviderNodeStatus::Unknown,
      }),
      Err(GceError::NotFound { .. }) => Ok(ProviderNodeStatus::Deleted),
      Err(source) => Err(ProviderAdapterError::api(
        "Google instance status check failed.",
        source,
      )),
    }
  }

  fn node_exists(&self, token: &str, node_id: &str) -> Result<bool, ProviderAdapterError> {
    let key = Self::service_account(token)?;
    let (zone, instance_name) = Self::split_node_id(node_id)?;

    match self.client.get_instance(&key, &zone, &instance_name) {
      Ok(_) => Ok(true),
      Err(GceError::NotFound { .. }) => Ok(false),
      Err(source) => Err(ProviderAdapterError::api(
        "Google instance existence check failed.",
        source,
      )),
    }
  }

  /// Delete the instance, then its firewall rule.
  ///
  /// The boot disk is `autoDelete`, so it goes with the instance — a disk left behind is the
  /// classic way a "deleted" GCE node keeps costing money. A rule that cannot be removed is
  /// reported rather than swallowed; unlike the instance it is not billable, but leaving it silently
  /// blocks a later node from reusing the name.
  fn delete_node(&self, token: &str, node_id: &str) -> Result<(), ProviderAdapterError> {
    let key = Self::service_account(token)?;
    let (zone, instance_name) = Self::split_node_id(node_id)?;

    self
      .client
      .delete_instance(&key, &zone, &instance_name)
      .map_err(|source| ProviderAdapterError::api("Google instance delete call failed.", source))?;

    let firewall_rule = gce_firewall_rule_name(&instance_name);
    self
      .client
      .delete_firewall_rule(&key, &firewall_rule)
      .map_err(|source| {
        ProviderAdapterError::api(
          format!(
            "The instance was deleted, but firewall rule '{firewall_rule}' could NOT be removed — delete it in the Google Cloud console."
          ),
          source,
        )
      })
  }
}

/// Instance types offered for a Blink node, cheapest first.
///
/// AWS publishes prices through the Pricing API, which is a large region-limited catalogue and a
/// heavyweight dependency for three rows. These are list prices for on-demand Linux in `us-east-1`
/// and are reported with `estimated: true`, which is what that flag exists for — the Docker adapter
/// uses it the same way. Availability is still checked live per region, so a type a region cannot
/// launch is never offered.
///
/// `monthly_usd` includes the public IPv4 address charge (~$3.60/mo), because a node without one is
/// unreachable and quoting the instance alone would understate the real bill.
const EC2_PLAN_SPECS: &[Ec2PlanSpec] = &[
  Ec2PlanSpec {
    instance_type: "t3.micro",
    label: "t3.micro — 2 vCPU, 1 GB",
    vcpus: 2,
    memory_mb: 1024,
    disk_gb: 8,
    transfer_tb: 0.1,
    monthly_usd: 11.20,
    hourly_usd: 0.0154,
  },
  Ec2PlanSpec {
    instance_type: "t3.small",
    label: "t3.small — 2 vCPU, 2 GB",
    vcpus: 2,
    memory_mb: 2048,
    disk_gb: 8,
    transfer_tb: 0.1,
    monthly_usd: 18.80,
    hourly_usd: 0.0258,
  },
  Ec2PlanSpec {
    instance_type: "t3.medium",
    label: "t3.medium — 2 vCPU, 4 GB",
    vcpus: 2,
    memory_mb: 4096,
    disk_gb: 8,
    transfer_tb: 0.1,
    monthly_usd: 33.90,
    hourly_usd: 0.0464,
  },
];

#[derive(Debug, Clone, Copy)]
struct Ec2PlanSpec {
  instance_type: &'static str,
  label: &'static str,
  vcpus: u64,
  memory_mb: u64,
  disk_gb: u64,
  transfer_tb: f64,
  monthly_usd: f64,
  hourly_usd: f64,
}

/// Tag key marking every resource this app creates, so an operator can find them in the console.
const EC2_OWNER_TAG_KEY: &str = "blink";
const EC2_OWNER_TAG_VALUE: &str = "managed";

/// Maps a user's node name onto an AWS tag value.
///
/// AWS tag values are far more permissive than a hostname — up to 256 characters, most of Unicode —
/// so this is a different rule from `sanitize_droplet_name`, and that is exactly why it belongs in
/// this adapter rather than being shared (AGENTS.md §3). Only the characters AWS actually rejects
/// are replaced; the user's chosen label is otherwise preserved.
fn sanitize_ec2_tag_value(raw: &str) -> String {
  let cleaned: String = raw
    .trim()
    .chars()
    .filter(|character| !matches!(character, '\u{0}'..='\u{1F}' | '\u{7F}'))
    .take(255)
    .collect();

  let trimmed = cleaned.trim();
  if trimmed.is_empty() {
    "blink-node".to_string()
  } else {
    trimmed.to_string()
  }
}

/// A security-group name derived from the node name.
///
/// AWS restricts these to `a-zA-Z0-9 ._-:/()#,@[]+=&;{}!$*` and requires uniqueness within the VPC,
/// so a random suffix is appended — two nodes the user called the same thing must not collide.
fn ec2_security_group_name(node_name: &str) -> String {
  let base: String = node_name
    .trim()
    .chars()
    .map(|character| {
      if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
        character
      } else {
        '-'
      }
    })
    .collect();

  let compact = base.trim_matches('-');
  let stem = if compact.is_empty() { "blink-node" } else { compact };
  let bounded: String = stem.chars().take(200).collect();
  let suffix = uuid::Uuid::new_v4().simple().to_string();

  format!("blink-{}-{}", bounded.trim_matches('-'), &suffix[..8])
}

/// Base64 for EC2 `UserData`, which must be encoded before signing.
///
/// Written out rather than adding a dependency for one call.
fn base64_encode(input: &[u8]) -> String {
  const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let mut out = String::with_capacity(input.len().div_ceil(3) * 4);

  for chunk in input.chunks(3) {
    let bytes = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
    let triple = ((bytes[0] as u32) << 16) | ((bytes[1] as u32) << 8) | bytes[2] as u32;

    out.push(ALPHABET[(triple >> 18) as usize & 0x3F] as char);
    out.push(ALPHABET[(triple >> 12) as usize & 0x3F] as char);
    out.push(if chunk.len() > 1 {
      ALPHABET[(triple >> 6) as usize & 0x3F] as char
    } else {
      '='
    });
    out.push(if chunk.len() > 2 {
      ALPHABET[triple as usize & 0x3F] as char
    } else {
      '='
    });
  }

  out
}

/// EC2 caps `UserData` at 16 KiB before base64. The rendered cloud-init is several KB, so this is
/// checked rather than assumed — an edit to the shared bootstrap script must not silently push
/// provisioning over the limit.
const EC2_USER_DATA_LIMIT_BYTES: usize = 16 * 1024;

pub struct Ec2ProviderAdapter {
  client: Ec2ApiClient,
  config: Ec2Config,
}

impl Ec2ProviderAdapter {
  /// Infallible by construction.
  ///
  /// `DigitalOceanProviderAdapter::new` returns a `Result` that `ProviderRegistry::new` unwraps with
  /// a panic; adding a second adapter that can fail at construction would double that hazard, so
  /// the HTTP client is built lazily-safe here and a build failure surfaces as a normal API error.
  pub fn new() -> Self {
    let config = Ec2Config::defaults();
    let client = Ec2ApiClient::new_with_timeout(RetryPolicy::cloud_default(), config.request_timeout)
      .unwrap_or_else(|_| {
        // `reqwest` only fails to build a client on a malformed TLS configuration, which is not
        // input-dependent. Retrying with defaults keeps construction total.
        Ec2ApiClient::new_with_timeout(RetryPolicy::cloud_default(), Duration::from_secs(20))
          .expect("a default blocking HTTP client must be constructible")
      });

    Self { client, config }
  }

  #[cfg(test)]
  fn with_base_url(base_url: &str) -> Self {
    let config = Ec2Config::defaults();
    // A fast retry policy keeps mock-server tests from sleeping through real backoff.
    let client = Ec2ApiClient::with_base_url(
      base_url,
      RetryPolicy {
        max_retries: 1,
        base_delay_ms: 1,
        max_delay_ms: 2,
        jitter_max_ms: 0,
      },
      config.request_timeout,
    )
    .expect("test client should build");

    Self {
      client,
      config: Ec2Config {
        poll_interval: Duration::from_millis(1),
        dependency_release_timeout: Duration::from_millis(20),
        ..config
      },
    }
  }

  fn credentials(stored: &str) -> Result<AwsCredentials, ProviderAdapterError> {
    let credential = ProviderCredential::parse(stored);

    Ok(AwsCredentials {
      access_key_id: credential.require("access_key_id")?.to_string(),
      secret_access_key: credential.require("secret_access_key")?.to_string(),
      // Optional: only temporary credentials carry one.
      session_token: credential.field("session_token").map(|value| value.to_string()),
    })
  }

  fn effective_region(region: &str) -> String {
    let trimmed = region.trim();
    if trimmed.is_empty() {
      DEFAULT_EC2_REGION.to_string()
    } else {
      trimmed.to_string()
    }
  }

  /// The region an existing node lives in.
  ///
  /// EC2 scopes both the endpoint host and the signature to a region, so operating on an instance
  /// requires knowing which one it is in. The node id alone does not say, and the trait's status,
  /// exists and delete methods do not carry a region — so the region is encoded into the node id
  /// at create time and split back out here.
  fn split_node_id(node_id: &str) -> Result<(String, String), ProviderAdapterError> {
    match node_id.trim().split_once('/') {
      Some((region, instance)) if !region.is_empty() && !instance.is_empty() => {
        Ok((region.to_string(), instance.to_string()))
      }
      _ => Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "'{node_id}' is not an EC2 node reference. Expected '<region>/<instance-id>'; the node record may belong to another provider."
        ),
      }),
    }
  }

  fn join_node_id(region: &str, instance_id: &str) -> String {
    format!("{region}/{instance_id}")
  }

  /// The security group attached to a node.
  ///
  /// Answers `None` for an instance EC2 no longer reports, so destroy stays idempotent without a
  /// separate existence check first.
  fn security_group_for_node(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    instance_id: &str,
  ) -> Result<Option<String>, ProviderAdapterError> {
    self
      .client
      .security_group_for_instance(region, credentials, instance_id)
      .map_err(|source| {
        ProviderAdapterError::api("Could not find the security group attached to this node.", source)
      })
  }
}

impl ProviderAdapter for Ec2ProviderAdapter {
  fn provider_name(&self) -> &'static str {
    "aws-ec2"
  }

  fn credential_fields(&self) -> Vec<CredentialField> {
    vec![
      CredentialField {
        key: "access_key_id".to_string(),
        label: "Access key ID".to_string(),
        placeholder: "AKIA...".to_string(),
        // An access key ID is an identifier, not a secret. Masking it only makes it harder to
        // check against the AWS console.
        secret: false,
        help: Some(
          "An IAM user key with EC2 instance, security group, SSM parameter and STS identity permissions."
            .to_string(),
        ),
        token_page_url: Some("https://console.aws.amazon.com/iam/home#/security_credentials".to_string()),
        scope_hint: Some("An access key for an IAM user that can manage EC2 instances, security groups and Elastic IPs.".to_string()),
      },
      CredentialField {
        key: "secret_access_key".to_string(),
        label: "Secret access key".to_string(),
        placeholder: "wJalrXUtnFEMI/...".to_string(),
        secret: true,
        help: Some("Shown only once by AWS when the key is created.".to_string()),
        token_page_url: None,
        scope_hint: None,
      },
    ]
  }

  fn validate_token(&self, token: &str) -> Result<ProviderIdentity, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;

    let identity = self
      .client
      .get_caller_identity(DEFAULT_EC2_REGION, &credentials)
      .map_err(|source| ProviderAdapterError::api("AWS credential validation failed.", source))?;

    Ok(ProviderIdentity {
      account_id: identity.account,
      // AWS exposes no email for a key. The ARN names the principal, which is what an operator
      // would check to confirm they saved the key they meant to.
      label: Some(identity.arn),
      // STS answers who the caller is, not whether the account is in good standing.
      status: None,
    })
  }

  fn list_regions(&self, token: &str) -> Result<Vec<ProviderRegion>, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;

    let codes = self
      .client
      .describe_regions(&credentials)
      .map_err(|source| ProviderAdapterError::api("AWS region listing failed.", source))?;

    Ok(codes.iter().map(|code| self.describe_region(code)).collect())
  }

  fn describe_region(&self, slug: &str) -> ProviderRegion {
    ec2_regions::describe_region(slug)
  }

  fn known_regions(&self) -> Vec<ProviderRegion> {
    ec2_regions::known_regions()
  }

  fn list_plan_pricing(
    &self,
    token: &str,
    region: &str,
  ) -> Result<Vec<ProviderPlanPricing>, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let region = Self::effective_region(region);

    let candidates: Vec<&str> = EC2_PLAN_SPECS.iter().map(|plan| plan.instance_type).collect();

    // Prices are a curated estimate, but availability is not: offering an instance type this
    // region cannot launch would fail at RunInstances, after the security group already exists.
    let offered = self
      .client
      .filter_offered_instance_types(&region, &credentials, &candidates)
      .map_err(|source| {
        ProviderAdapterError::api("AWS instance type availability lookup failed.", source)
      })?;

    let source_timestamp = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
      Ok(value) => value.as_secs().to_string(),
      Err(_) => "0".to_string(),
    };

    Ok(
      EC2_PLAN_SPECS
        .iter()
        .filter(|plan| offered.iter().any(|candidate| candidate == plan.instance_type))
        .map(|plan| ProviderPlanPricing {
          plan_key: plan.instance_type.to_string(),
          plan_label: plan.label.to_string(),
          size_slug: plan.instance_type.to_string(),
          region: region.clone(),
          memory_mb: plan.memory_mb,
          vcpus: plan.vcpus,
          disk_gb: plan.disk_gb,
          transfer_tb: plan.transfer_tb,
          transfer_note:
            "Estimate. AWS bills data transfer out per GB and prices vary by region; includes the public IPv4 address charge."
              .to_string(),
          monthly_usd: Some(plan.monthly_usd),
          hourly_usd: Some(plan.hourly_usd),
          // List prices, not a live quote — see EC2_PLAN_SPECS.
          estimated: true,
          source_timestamp: source_timestamp.clone(),
        })
        .collect(),
    )
  }

  fn init_delivery(&self) -> InitDelivery {
    InitDelivery::CloudInit
  }

  fn create_node(
    &self,
    token: &str,
    node_name: &str,
    region: &str,
    size_slug: &str,
    init_payload: &str,
  ) -> Result<ProvisionedNode, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let region = Self::effective_region(region);

    if init_payload.as_bytes().len() > EC2_USER_DATA_LIMIT_BYTES {
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "The rendered bootstrap payload is {} bytes, over EC2's {EC2_USER_DATA_LIMIT_BYTES}-byte user-data limit.",
          init_payload.as_bytes().len()
        ),
      });
    }

    let instance_type = if size_slug.trim().is_empty() {
      EC2_PLAN_SPECS[0].instance_type.to_string()
    } else {
      size_slug.trim().to_string()
    };

    let image_id = self
      .client
      .resolve_ubuntu_ami(&region, &credentials)
      .map_err(|source| {
        ProviderAdapterError::api("Could not resolve the current Ubuntu AMI for this region.", source)
      })?;

    let vpc_id = self
      .client
      .default_vpc_id(&region, &credentials)
      .map_err(|source| ProviderAdapterError::api("Could not look up the default VPC.", source))?
      .ok_or_else(|| ProviderAdapterError::Unsupported {
        detail: format!(
          "This AWS account has no default VPC in {region}. Create one in the AWS console, or choose a region that has one."
        ),
      })?;

    let subnet_id = self
      .client
      .first_subnet_id(&region, &credentials, &vpc_id)
      .map_err(|source| ProviderAdapterError::api("Could not look up a subnet in the default VPC.", source))?
      .ok_or_else(|| ProviderAdapterError::Unsupported {
        detail: format!("The default VPC in {region} has no subnet to launch into."),
      })?;

    let tags = vec![
      ("Name".to_string(), sanitize_ec2_tag_value(node_name)),
      (EC2_OWNER_TAG_KEY.to_string(), EC2_OWNER_TAG_VALUE.to_string()),
    ];

    let group_id = self
      .client
      .create_security_group(
        &region,
        &credentials,
        &vpc_id,
        &ec2_security_group_name(node_name),
        "Blink WireGuard node — inbound UDP 51820 only.",
        &tags,
      )
      .map_err(|source| ProviderAdapterError::api("Could not create the security group.", source))?;

    // From here a real resource exists, so every later failure removes it before returning.
    let cleanup_group = |adapter: &Self| {
      let _ = adapter
        .client
        .delete_security_group(&region, &credentials, &group_id);
    };

    if let Err(source) = self
      .client
      .authorize_udp_ingress(&region, &credentials, &group_id, DEFAULT_WIREGUARD_PORT)
    {
      cleanup_group(self);
      return Err(ProviderAdapterError::api(
        "Could not open the WireGuard port on the new security group.",
        source,
      ));
    }

    let launched = match self.client.run_instance(
      &region,
      &credentials,
      &image_id,
      &instance_type,
      &subnet_id,
      &group_id,
      &base64_encode(init_payload.as_bytes()),
      &tags,
    ) {
      Ok(instance) => instance,
      Err(source) => {
        cleanup_group(self);
        return Err(ProviderAdapterError::api("EC2 instance launch failed.", source));
      }
    };

    let running = match self.client.wait_for_instance_running(
      &region,
      &credentials,
      &launched.instance_id,
      self.config.poll_interval,
      self.config.poll_max_attempts,
      self.config.instance_ready_timeout,
    ) {
      Ok(instance) => instance,
      Err(source) => {
        // Leave nothing half-provisioned and billing behind.
        let _ = self
          .client
          .terminate_instance(&region, &credentials, &launched.instance_id);
        cleanup_group(self);
        return Err(ProviderAdapterError::api(
          "EC2 instance did not reach running before timeout.",
          source,
        ));
      }
    };

    let public_ipv4 = match running.public_ip {
      Some(address) => address,
      None => {
        let _ = self
          .client
          .terminate_instance(&region, &credentials, &launched.instance_id);
        cleanup_group(self);
        return Err(ProviderAdapterError::Unsupported {
          detail: "The EC2 instance started without a public IPv4 address, so it cannot be reached."
            .to_string(),
        });
      }
    };

    Ok(ProvisionedNode {
      node_id: Self::join_node_id(&region, &running.instance_id),
      public_ipv4,
      // No port translation in front of an EC2 instance, so the client dials the listen port.
      endpoint_port: DEFAULT_WIREGUARD_PORT,
    })
  }

  /// Readiness for EC2 is "the instance is running and has a public IPv4".
  ///
  /// `create_node` has already polled to running, so this confirms the instance is still healthy
  /// and reports the endpoint. Nothing is read off the instance itself — the server's public key
  /// was generated locally and baked into user-data, so it is already known here (AGENTS.md §2).
  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let (region, instance_id) = Self::split_node_id(&request.node_id)?;

    let instance = self
      .client
      .describe_instance(&region, &credentials, &instance_id)
      .map_err(|source| {
        ProviderAdapterError::api("Could not confirm EC2 instance state before completing provisioning.", source)
      })?
      .ok_or_else(|| ProviderAdapterError::Unsupported {
        detail: format!("EC2 no longer reports instance {instance_id}; provisioning cannot complete."),
      })?;

    if instance.state != "running" {
      return Err(ProviderAdapterError::Unsupported {
        detail: format!(
          "EC2 instance is '{}' rather than running; provisioning cannot complete.",
          instance.state
        ),
      });
    }

    let server_endpoint_ip = if request.endpoint_ip.trim().is_empty() {
      instance.public_ip.ok_or_else(|| ProviderAdapterError::Unsupported {
        detail: "The running EC2 instance has no public IPv4 address.".to_string(),
      })?
    } else {
      request.endpoint_ip.clone()
    };

    Ok(BootstrapOutputContract {
      server_endpoint_ip,
      // Echoed from what the caller generated and baked into user-data, not read back.
      server_public_key: request.server_keypair.public_key.clone(),
      listen_port: DEFAULT_WIREGUARD_PORT,
    })
  }

  fn node_status(&self, token: &str, node_id: &str) -> Result<ProviderNodeStatus, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let (region, instance_id) = Self::split_node_id(node_id)?;

    let instance = self
      .client
      .describe_instance(&region, &credentials, &instance_id)
      .map_err(|source| ProviderAdapterError::api("EC2 instance status check failed.", source))?;

    Ok(match instance {
      None => ProviderNodeStatus::Deleted,
      Some(instance) => match instance.state.as_str() {
        "pending" => ProviderNodeStatus::New,
        "running" => ProviderNodeStatus::Active,
        "stopping" | "stopped" => ProviderNodeStatus::Off,
        "shutting-down" => ProviderNodeStatus::Archive,
        "terminated" => ProviderNodeStatus::Deleted,
        _ => ProviderNodeStatus::Unknown,
      },
    })
  }

  fn node_exists(&self, token: &str, node_id: &str) -> Result<bool, ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let (region, instance_id) = Self::split_node_id(node_id)?;

    let instance = self
      .client
      .describe_instance(&region, &credentials, &instance_id)
      .map_err(|source| ProviderAdapterError::api("EC2 instance existence check failed.", source))?;

    // A terminated instance lingers in DescribeInstances for about an hour. Reporting it as
    // existing would leave a destroyed node looking alive on the fleet list.
    Ok(matches!(instance, Some(instance) if instance.state != "terminated"))
  }

  /// Terminate the instance, then remove the security group created alongside it.
  ///
  /// The order is forced by AWS: a security group cannot be deleted while its network interface is
  /// still attached, and that interface is released asynchronously after termination. A failure to
  /// remove the group is reported rather than swallowed — silently succeeding here is the habit
  /// that leaves resources behind with nothing on screen to say so (AGENTS.md §8).
  fn delete_node(&self, token: &str, node_id: &str) -> Result<(), ProviderAdapterError> {
    let credentials = Self::credentials(token)?;
    let (region, instance_id) = Self::split_node_id(node_id)?;

    // Read the attached group *before* terminating; afterwards the association is gone.
    let group_id = self.security_group_for_node(&region, &credentials, &instance_id)?;

    self
      .client
      .terminate_instance(&region, &credentials, &instance_id)
      .map_err(|source| ProviderAdapterError::api("EC2 instance termination failed.", source))?;

    let Some(group_id) = group_id else {
      return Ok(());
    };

    self
      .client
      .wait_for_instance_terminated(
        &region,
        &credentials,
        &instance_id,
        self.config.poll_interval,
        self.config.dependency_release_timeout,
      )
      .map_err(|source| {
        ProviderAdapterError::api(
          "The EC2 instance was terminated but did not finish releasing its network interface, so its security group could not be removed.",
          source,
        )
      })?;

    // The interface can still be detaching after the instance reports terminated, which surfaces as
    // DependencyViolation. Retry on exactly that, and only that.
    let policy = self.client.retry_policy().clone();
    let mut last_error = None;

    for attempt in 0..=policy.max_retries {
      match self
        .client
        .delete_security_group(&region, &credentials, &group_id)
      {
        Ok(()) => return Ok(()),
        Err(ec2::Ec2Error::DependencyViolation { .. }) if attempt < policy.max_retries => {
          std::thread::sleep(policy.backoff_for_attempt(attempt));
        }
        Err(source) => {
          last_error = Some(source);
          break;
        }
      }
    }

    Err(ProviderAdapterError::api(
      format!(
        "The EC2 instance was terminated, but its security group {group_id} could not be removed. Delete it in the AWS console."
      ),
      last_error.unwrap_or(ec2::Ec2Error::Timeout {
        action: "delete the security group".to_string(),
        detail: "The group was still in use after every retry.".to_string(),
      }),
    ))
  }
}

#[derive(Debug, Clone)]
pub struct ProvisionedNode {
  /// The provider's own identifier for this node, opaque to everything above the adapter.
  ///
  /// This was a `u64` while DigitalOcean was the only production provider. A modern EC2 instance id
  /// (`i-0abc123def4567890`) does not fit in 64 bits, and the Docker adapter had to mint a synthetic
  /// number and stamp it on the container as a label purely to satisfy the type. It is stored
  /// verbatim in `vpn_nodes.external_ref`, which was always `TEXT`.
  pub node_id: String,
  pub public_ipv4: String,
  /// UDP port the *client* should dial to reach this node's WireGuard interface.
  ///
  /// Adapters report their own endpoint rather than the orchestrator assuming one. For
  /// DigitalOcean this is the server's listen port. For Docker it is the ephemeral host port
  /// published to the container, which differs per node so several can run at once.
  pub endpoint_port: u16,
}

/// Everything an adapter needs to create a node.
///
/// It once also carried `endpoint_ip` (a `0.0.0.0` placeholder to be filled in later),
/// `readiness_marker_path` and `server_keypair` — all inputs to reading state back off the server
/// after create. Nothing read them, because the payload already contains the keys and the endpoint
/// comes from provider metadata (AGENTS.md §2).
#[derive(Debug, Clone)]
pub struct NodeInitRequest {
  pub node_name: String,
  pub region: String,
  pub size_slug: String,
  pub init_payload: String,
}

#[derive(Debug, Clone)]
pub struct NodeInitResult {
  pub provisioned: ProvisionedNode,
}

/// What the adapter needs to report a node ready.
///
/// The keypair is here because the adapter builds the output contract from what was generated
/// locally; `readiness_marker_path` used to accompany it, for reading the marker back.
#[derive(Debug, Clone)]
pub struct BootstrapWaitRequest {
  pub node_id: String,
  pub endpoint_ip: String,
  pub server_keypair: WireGuardKeypair,
}

pub trait NodeProvider: Send + Sync {
  fn init(&self, token: &str, request: &NodeInitRequest) -> Result<NodeInitResult, ProviderAdapterError>;
  fn wait_for_bootstrap_ready(
    &self,
    token: &str,
    request: &BootstrapWaitRequest,
  ) -> Result<BootstrapOutputContract, ProviderAdapterError>;
  fn get_status(&self, token: &str, node_id: &str) -> Result<ProviderNodeStatus, ProviderAdapterError>;
  fn exists(&self, token: &str, node_id: &str) -> Result<bool, ProviderAdapterError>;
  fn delete(&self, token: &str, node_id: &str) -> Result<(), ProviderAdapterError>;
}

#[derive(Clone)]
struct AdapterNodeProvider {
  adapter: Arc<dyn ProviderAdapter>,
  token: Option<String>,
}

impl AdapterNodeProvider {
  fn with_token(adapter: Arc<dyn ProviderAdapter>, token: String) -> Self {
    Self {
      adapter,
      token: Some(token),
    }
  }
}

impl NodeProvider for AdapterNodeProvider {
  fn init(&self, token: &str, request: &NodeInitRequest) -> Result<NodeInitResult, ProviderAdapterError> {
    let effective_token = self.token.as_deref().unwrap_or(token);

    let provisioned = self.adapter.create_node(
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

  fn get_status(&self, token: &str, node_id: &str) -> Result<ProviderNodeStatus, ProviderAdapterError> {
    let effective_token = self.token.as_deref().unwrap_or(token);
    self.adapter.node_status(effective_token, node_id)
  }

  fn exists(&self, token: &str, node_id: &str) -> Result<bool, ProviderAdapterError> {
    let effective_token = self.token.as_deref().unwrap_or(token);
    self.adapter.node_exists(effective_token, node_id)
  }

  fn delete(&self, token: &str, node_id: &str) -> Result<(), ProviderAdapterError> {
    let effective_token = self.token.as_deref().unwrap_or(token);
    self.adapter.delete_node(effective_token, node_id)
  }
}

/// Who a validated credential belongs to.
///
/// The answer to "this key works — but whose account is it, and will it be the one billed?", which
/// is the question a user actually has after saving a credential.
///
/// This replaced `Account`, DigitalOcean's own `GET /v2/account` response type, which was used as
/// the neutral result because DigitalOcean was once the only provider. Its `uuid`/`email`/`status`
/// fields forced every later adapter to lie: EC2 put an ARN in `email`, GCE a service-account
/// address, OpenStack a credential id, and all of them hardcoded `status: "active"` — a claim none
/// of those providers had actually made. The fields here are named for what they mean, and the
/// ones a provider does not answer are `None` rather than invented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderIdentity {
  /// The account, project or tenancy this credential acts on — whatever the provider bills.
  pub account_id: String,
  /// Something a person recognises as theirs: an email, a service account address, a project name.
  /// `None` where the provider offers nothing human-readable.
  pub label: Option<String>,
  /// Account standing, where the provider reports one.
  ///
  /// DigitalOcean does, and it matters: a `locked` account accepts a valid token and then refuses
  /// to create anything, so surfacing it turns a baffling provisioning failure into an obvious
  /// one. Most providers say nothing, and `None` records that rather than asserting "active" on
  /// their behalf.
  pub status: Option<String>,
}

impl ProviderIdentity {
  /// One line naming this account, for the message shown after a credential is checked.
  pub fn describe(&self) -> String {
    let named = match self.label.as_deref().map(str::trim) {
      Some(label) if !label.is_empty() => format!("{label} ({})", self.account_id),
      _ => self.account_id.clone(),
    };

    match self.status.as_deref().map(str::trim) {
      Some(status) if !status.is_empty() => format!("{named}, status {status}"),
      _ => named,
    }
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
  /// `None` where the provider publishes no price at all.
  ///
  /// Optional rather than zero because zero is a claim: rendered, it says the plan is free, and
  /// an OpenStack cloud whose operator sets its own prices is emphatically not. The UI already
  /// renders an absent price as "n/a", and `vpn_nodes.price_monthly_usd` has always been nullable
  /// — this type was the odd one out.
  pub monthly_usd: Option<f64>,
  pub hourly_usd: Option<f64>,
  pub estimated: bool,
  pub source_timestamp: String,
}

#[derive(Clone)]
pub struct ProviderRegistry {
  adapters_by_slug: HashMap<String, Arc<dyn ProviderAdapter>>,
}

#[derive(Debug, Error)]
pub enum ProviderAdapterError {
  /// A call to the provider's API failed.
  ///
  /// The source is boxed because this enum is shared by every adapter. It was typed as
  /// `DigitalOceanError`, which meant no other adapter could report an API failure at all — the
  /// reason the Docker adapter reported even a crashed CLI as `Unsupported`. `describe_error_chain`
  /// still walks the chain, so the concrete cause reaches the user unchanged.
  #[error("Provider API call failed: {detail}")]
  Api {
    detail: String,
    #[source]
    source: Box<dyn std::error::Error + Send + Sync + 'static>,
  },
  /// This provider genuinely cannot do the thing asked of it.
  ///
  /// Not a transport failure and not a crashed subprocess — those are `Api`. This is the capability
  /// error AGENTS.md §3 calls for, e.g. an AWS account with no default VPC in the chosen region.
  #[error("Provider capability not available yet: {detail}")]
  Unsupported { detail: String },
}

impl ProviderAdapterError {
  /// Build an `Api` error from any adapter-specific error type.
  pub fn api<E>(detail: impl Into<String>, source: E) -> Self
  where
    E: std::error::Error + Send + Sync + 'static,
  {
    Self::Api {
      detail: detail.into(),
      source: Box::new(source),
    }
  }
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

    // Each adapter names itself, so the registry key and the adapter's own identity cannot drift.
    // These slugs were written out a second time here, which meant a renamed adapter would still
    // answer to its old key and `UnsupportedProviderSlug` would name a provider that exists.
    let adapters: Vec<Arc<dyn ProviderAdapter>> = vec![
      Arc::new(digitalocean),
      Arc::new(Ec2ProviderAdapter::new()),
      Arc::new(LinodeProviderAdapter::new()),
      Arc::new(GceProviderAdapter::new()),
      Arc::new(OpenStackProviderAdapter::new()),
      Arc::new(DockerProviderAdapter::new()),
    ];

    let adapters_by_slug = adapters
      .into_iter()
      .map(|adapter| (adapter.provider_name().to_string(), adapter))
      .collect();

    Self { adapters_by_slug }
  }

  /// A registry holding one mock adapter, so orchestrator tests can exercise the real flows
  /// without a provider account. Production always builds the full registry via `new`.
  #[cfg(test)]
  pub fn from_adapter(adapter: Arc<dyn ProviderAdapter>) -> Self {
    let slug = adapter.provider_name().to_string();
    let mut adapters_by_slug = HashMap::new();
    adapters_by_slug.insert(slug, adapter);
    Self { adapters_by_slug }
  }

  /// A node provider with no credential attached.
  ///
  /// Test-only: production always resolves through `node_provider_for_provider_id`, which reads the
  /// stored credential. Docker needs no real token, which is what makes this usable in tests.
  #[cfg(test)]
  pub fn node_provider_for_slug(&self, slug: &str) -> Option<Arc<dyn NodeProvider>> {
    self
      .adapter_arc_for_slug(slug)
      .map(|adapter| Arc::new(AdapterNodeProvider::with_token(adapter, String::new())) as Arc<dyn NodeProvider>)
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

  // Four provider-named wrappers used to live here — create/upsert pairs for DigitalOcean and for
  // Docker — each of which did nothing but pass a hardcoded slug to `upsert_provider_with_token`.
  // Nothing called them, and a registry method named after one provider is the shape AGENTS.md §3
  // exists to prevent. The slug is a parameter now, as it already was underneath.

  /// Register a credential against whichever adapter the provider's slug names.
  ///
  /// The slug is resolved from the stored provider record by the caller, so adding a provider
  /// does not mean adding another command.
  pub fn upsert_provider_credential_for_slug(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    provider_name: &str,
    provider_slug: &str,
    credential: &ProviderCredential,
  ) -> Result<ProviderRecord, ProviderFlowError> {
    self.upsert_provider_with_credential(
      db,
      secure_storage,
      provider_id,
      provider_name,
      provider_slug,
      credential,
    )
  }

  fn upsert_provider_with_credential(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    provider_name: &str,
    provider_slug: &str,
    credential: &ProviderCredential,
  ) -> Result<ProviderRecord, ProviderFlowError> {
    self.adapter_for_slug(provider_slug)?;
    let stored_credential = credential.to_stored_value();

    let existing = db
      .get_provider(provider_id)
      .map_err(|source| ProviderFlowError::Database {
        detail: "Failed to inspect provider before token upsert.".to_string(),
        source,
      })?;
    let previous_token_ref = existing.as_ref().and_then(|record| record.token_ref.clone());

    let token_ref = secure_storage
      .store_provider_credential(provider_id, &stored_credential)
      .map_err(|source| ProviderFlowError::SecretStore {
        detail: "Unable to save provider credential in protected storage."
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
      let _ = secure_storage.delete_provider_credential(&token_ref);
      return Err(ProviderFlowError::Database {
        detail: "Failed to persist provider row with updated token reference.".to_string(),
        source,
      });
    }

    if let Some(previous_token_ref) = previous_token_ref {
      let _ = secure_storage.delete_provider_credential(&previous_token_ref);
    }

    // The stored verdict described the key that was just replaced. Leaving it would show a brand
    // new, unchecked key as already verified.
    let _ = db.record_provider_validation(provider_id, None);

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
  ) -> Result<ProviderIdentity, ProviderFlowError> {
    let (provider, adapter) = self.load_provider_and_adapter(db, provider_id)?;
    let token = self.resolve_provider_token_for_record(db, secure_storage, &provider)?;
    let outcome = adapter
      .validate_token(&token)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Token validation through provider adapter failed.".to_string(),
        source,
      });

    // Persisted so the verdict survives a restart, rather than every stored key reading
    // "not checked" on each launch. Best-effort: failing to record it must not fail the check.
    let _ = db.record_provider_validation(provider_id, Some(outcome.is_ok()));

    outcome
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

  pub fn delete_provider_node(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    node_id: &str,
  ) -> Result<(), ProviderFlowError> {
    let node_provider = self.node_provider_for_provider_id(db, secure_storage, provider_id)?;
    node_provider
      .delete("", node_id)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Node deletion through provider adapter failed.".to_string(),
        source,
      })
  }

  pub fn provider_node_exists(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    node_id: &str,
  ) -> Result<bool, ProviderFlowError> {
    let node_provider = self.node_provider_for_provider_id(db, secure_storage, provider_id)?;
    node_provider
      .exists("", node_id)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Node existence check through provider adapter failed.".to_string(),
        source,
      })
  }

  pub fn provider_node_status(
    &self,
    db: &Database,
    secure_storage: &SecureStorage,
    provider_id: &str,
    node_id: &str,
  ) -> Result<ProviderNodeStatus, ProviderFlowError> {
    let node_provider = self.node_provider_for_provider_id_allow_suspended(db, secure_storage, provider_id)?;
    node_provider
      .get_status("", node_id)
      .map_err(|source| ProviderFlowError::Adapter {
        detail: "Node status check through provider adapter failed.".to_string(),
        source,
      })
  }

  /// The credential inputs a provider needs, for the UI to render.
  ///
  /// Resolved through the adapter, so the frontend never has to know which provider needs how many
  /// fields.
  pub fn credential_fields_for_provider(
    &self,
    db: &Database,
    provider_id: &str,
  ) -> Result<Vec<CredentialField>, ProviderFlowError> {
    let (_, adapter) = self.load_provider_and_adapter_allow_suspended(db, provider_id)?;
    Ok(adapter.credential_fields())
  }

  /// Where a provider runs, from its adapter's curated table. Offline: no token, no network call.
  pub fn known_regions_for_provider(
    &self,
    db: &Database,
    provider_id: &str,
  ) -> Result<Vec<ProviderRegion>, ProviderFlowError> {
    let (_, adapter) = self.load_provider_and_adapter_allow_suspended(db, provider_id)?;
    Ok(adapter.known_regions())
  }

  /// How this provider takes delivery of the bootstrap script.
  pub fn init_delivery_for_provider(
    &self,
    db: &Database,
    provider_id: &str,
  ) -> Result<InitDelivery, ProviderFlowError> {
    let (_, adapter) = self.load_provider_and_adapter_allow_suspended(db, provider_id)?;
    Ok(adapter.init_delivery())
  }

  /// The owning adapter's client-MTU hint, read at provision time so it can be recorded on the
  /// node row. Offline: needs the stored slug, not a token.
  pub fn client_mtu_hint_for_provider(
    &self,
    db: &Database,
    provider_id: &str,
  ) -> Result<Option<u16>, ProviderFlowError> {
    let (_, adapter) = self.load_provider_and_adapter_allow_suspended(db, provider_id)?;
    Ok(adapter.client_mtu_hint())
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
        detail: "Resume the provider from Setup before running this operation.".to_string(),
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
      .get_provider_credential(&token_ref)
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

  #[test]
  fn token_pages_are_https_and_set_once_per_provider() {
    use super::{
      CredentialField, DigitalOceanProviderAdapter, Ec2ProviderAdapter, GceProviderAdapter, LinodeProviderAdapter,
      OpenStackProviderAdapter, ProviderAdapter,
    };
    let adapters: Vec<(&str, Vec<CredentialField>, bool)> = vec![
      (
        "digitalocean",
        DigitalOceanProviderAdapter::new().expect("adapter builds").credential_fields(),
        true,
      ),
      ("linode", LinodeProviderAdapter::new().credential_fields(), true),
      ("ec2", Ec2ProviderAdapter::new().credential_fields(), true),
      ("gce", GceProviderAdapter::new().credential_fields(), true),
      // Every OpenStack cloud has its own console; there is no one page to send anyone to.
      ("openstack", OpenStackProviderAdapter::new().credential_fields(), false),
    ];
    for (name, fields, expects_page) in adapters {
      let pages: Vec<&String> = fields.iter().filter_map(|field| field.token_page_url.as_ref()).collect();
      assert_eq!(pages.len(), usize::from(expects_page), "{name}: token page count");
      for page in pages {
        assert!(page.starts_with("https://"), "{name}: {page}");
      }
      if expects_page {
        assert!(fields[0].token_page_url.is_some() && fields[0].scope_hint.is_some(), "{name}: on the first field");
      }
    }
  }

  use std::path::PathBuf;
  use std::sync::Arc;
  use std::time::{SystemTime, UNIX_EPOCH};

  use rusqlite::{Connection, params};

  use crate::backend::bootstrap::{BootstrapOutputContract, DEFAULT_WIREGUARD_PORT};
  use crate::backend::db::Database;
  use crate::backend::secure_storage::SecureStorage;

  use crate::backend::crypto::keypair::generate_server_keypair;

  use crate::backend::logging;

  use super::{
    BootstrapWaitRequest, CredentialField, EC2_USER_DATA_LIMIT_BYTES, Ec2ProviderAdapter,
    GCE_USER_DATA_LIMIT_BYTES, GceProviderAdapter, InitDelivery, LEGACY_SINGLE_CREDENTIAL_KEY,
    LINODE_USER_DATA_LIMIT_BYTES, LinodeProviderAdapter, NodeInitRequest, ProviderAdapter,
    ProviderAdapterError, ProviderCredential, ProviderIdentity, ProviderNodeStatus, ProviderPlanPricing,
    OpenStackProviderAdapter, ProviderRegion, ProviderRegistry, ProvisionedNode, base64_encode,
    ec2_security_group_name, sanitize_droplet_name, sanitize_ec2_tag_value,
    sanitize_gce_instance_name, sanitize_linode_label,
  };

  fn test_db_path(test_name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("clock must be after unix epoch")
      .as_nanos();
    path.push(format!("blink_provider_{test_name}_{nanos}.sqlite3"));
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
    assert_eq!(sanitize_droplet_name("!!!"), "blink-node");
    assert_eq!(sanitize_droplet_name(""), "blink-node");

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
          node_id: "4242".to_string(),
          endpoint_ip: String::new(),
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
          node_id: "99".to_string(),
          endpoint_ip: String::new(),
          server_keypair: generate_server_keypair().expect("keypair should generate"),
        },
      )
      .expect_err("a droplet that is not active must not report ready");

    assert!(error.to_string().contains("active"));
  }

  /// The stored credential is a JSON field set, so every DigitalOcean call has to go through
  /// `api_token`. The readiness poll used to receive the raw stored value and send the whole JSON
  /// object as the bearer token, which DigitalOcean answered with 401 — after the droplet was
  /// already created and billing. The exact-header matcher is what makes this test able to fail.
  #[test]
  fn digitalocean_create_node_polls_with_the_parsed_token_not_the_stored_credential() {
    let mut server = mockito::Server::new();
    let stored = ProviderCredential::from_fields([(
      "api_token".to_string(),
      "dop_v1_EXAMPLE_NOT_A_REAL_TOKEN".to_string(),
    )])
    .to_stored_value();

    let create = server
      .mock("POST", "/v2/droplets")
      .match_header(
        "authorization",
        mockito::Matcher::Exact("Bearer dop_v1_EXAMPLE_NOT_A_REAL_TOKEN".to_string()),
      )
      .with_status(202)
      .with_header("content-type", "application/json")
      .with_body(r#"{"droplet":{"id":7001,"name":"gs-node","status":"new","networks":{"v4":[]}}}"#)
      .create();

    let poll = server
      .mock("GET", "/v2/droplets/7001")
      .match_header(
        "authorization",
        mockito::Matcher::Exact("Bearer dop_v1_EXAMPLE_NOT_A_REAL_TOKEN".to_string()),
      )
      .with_status(200)
      .with_header("content-type", "application/json")
      .with_body(
        r#"{"droplet":{"id":7001,"name":"gs-node","status":"active","networks":{"v4":[{"ip_address":"203.0.113.9","type":"public"}]}}}"#,
      )
      .create();

    let adapter = super::DigitalOceanProviderAdapter::with_base_url(&server.url())
      .expect("adapter should construct against mock server");

    let provisioned = adapter
      .create_node(&stored, "gs-node", "nyc1", "s-1vcpu-512mb-10gb", "#cloud-config")
      .expect("create should succeed when both calls carry the parsed token");

    create.assert();
    poll.assert();
    assert_eq!(provisioned.node_id, "7001");
    assert_eq!(provisioned.public_ipv4, "203.0.113.9");
  }

  /// A failure *after* the droplet exists must not strand it. The orchestrator's rollback only
  /// fires once `create_node` has returned a node id, so the adapter owns this cleanup — otherwise
  /// the user pays for a droplet the app never recorded.
  #[test]
  fn digitalocean_create_node_deletes_the_droplet_when_readiness_fails() {
    let mut server = mockito::Server::new();

    let _create = server
      .mock("POST", "/v2/droplets")
      .with_status(202)
      .with_header("content-type", "application/json")
      .with_body(r#"{"droplet":{"id":7002,"name":"gs-node","status":"new","networks":{"v4":[]}}}"#)
      .create();

    // The exact failure seen in the field: the poll is rejected while the droplet is already live.
    let _poll = server
      .mock("GET", "/v2/droplets/7002")
      .with_status(401)
      .with_body(r#"{"id":"unauthorized","message":"Unable to authenticate you."}"#)
      .create();

    let delete = server
      .mock("DELETE", "/v2/droplets/7002")
      .with_status(204)
      .expect(1)
      .create();

    let adapter = super::DigitalOceanProviderAdapter::with_base_url(&server.url())
      .expect("adapter should construct against mock server");

    let error = adapter
      .create_node(&single_field_credential("dop_v1_EXAMPLE_NOT_A_REAL_TOKEN").to_stored_value(), "gs-node", "nyc1", "s-1vcpu-512mb-10gb", "#cloud-config")
      .expect_err("a rejected readiness poll must fail the create");

    delete.assert();
    assert!(
      error.to_string().contains("never reached active state"),
      "error should name the readiness failure, got: {error}"
    );
  }

  /// Cleanup that itself fails is reported, never swallowed — the droplet id has to reach the user
  /// so they can remove it by hand (AGENTS.md §8).
  #[test]
  fn digitalocean_create_node_reports_a_droplet_it_could_not_delete() {
    let mut server = mockito::Server::new();

    let _create = server
      .mock("POST", "/v2/droplets")
      .with_status(202)
      .with_header("content-type", "application/json")
      .with_body(r#"{"droplet":{"id":7003,"name":"gs-node","status":"new","networks":{"v4":[]}}}"#)
      .create();

    let _poll = server.mock("GET", "/v2/droplets/7003").with_status(401).create();
    let _delete = server.mock("DELETE", "/v2/droplets/7003").with_status(403).create();

    let adapter = super::DigitalOceanProviderAdapter::with_base_url(&server.url())
      .expect("adapter should construct against mock server");

    let error = adapter
      .create_node(&single_field_credential("dop_v1_EXAMPLE_NOT_A_REAL_TOKEN").to_stored_value(), "gs-node", "nyc1", "s-1vcpu-512mb-10gb", "#cloud-config")
      .expect_err("a rejected readiness poll must fail the create");

    let message = error.to_string();
    assert!(message.contains("7003"), "the surviving droplet id must reach the user, got: {message}");
    assert!(
      message.contains("could NOT be deleted"),
      "a failed cleanup must be reported, got: {message}"
    );
  }

  // --- Amazon EC2 -----------------------------------------------------------------------------

  fn aws_credential() -> String {
    ProviderCredential::from_fields([
      ("access_key_id".to_string(), "AKIAEXAMPLENOTREAL".to_string()),
      (
        "secret_access_key".to_string(),
        "wJalrXUtnFEMI-EXAMPLE-NOT-A-REAL-KEY".to_string(),
      ),
    ])
    .to_stored_value()
  }

  /// Matches one EC2 Query action within the form-encoded body.
  fn ec2_action(action: &str) -> mockito::Matcher {
    mockito::Matcher::Regex(format!("Action={action}(&|$)"))
  }

  /// A mocked EC2 action.
  ///
  /// `expect_at_least(1)` rather than an exact count: how many times a flow polls is an
  /// implementation detail, and pinning it turns a harmless change into a test failure. What
  /// matters is that the action was called at all.
  fn mock_ec2(
    server: &mut mockito::ServerGuard,
    action: &str,
    body: &str,
  ) -> mockito::Mock {
    server
      .mock("POST", "/ec2")
      .match_body(ec2_action(action))
      .with_status(200)
      .with_body(body)
      .expect_at_least(1)
      .create()
  }

  /// The AMI lookup every provision starts with.
  fn mock_ssm_ami(server: &mut mockito::ServerGuard) -> mockito::Mock {
    server
      .mock("POST", "/ssm")
      .with_status(200)
      .with_body(r#"{"Parameter":{"Value":"ami-0abcdef1234567890"}}"#)
      .expect_at_least(1)
      .create()
  }

  /// The whole provisioning path: resolve an AMI, find the default VPC and a subnet, create and
  /// open a security group, launch, and poll to running with a public address.
  #[test]
  fn ec2_create_node_launches_into_the_default_vpc_and_reports_its_endpoint() {
    let mut server = mockito::Server::new();

    let ssm = mock_ssm_ami(&mut server);
    let vpcs = mock_ec2(
      &mut server,
      "DescribeVpcs",
      r#"<DescribeVpcsResponse><vpcSet><item><vpcId>vpc-0aaa</vpcId></item></vpcSet></DescribeVpcsResponse>"#,
    );
    let subnets = mock_ec2(
      &mut server,
      "DescribeSubnets",
      r#"<DescribeSubnetsResponse><subnetSet><item><subnetId>subnet-0bbb</subnetId></item></subnetSet></DescribeSubnetsResponse>"#,
    );
    let create_group = mock_ec2(
      &mut server,
      "CreateSecurityGroup",
      r#"<CreateSecurityGroupResponse><groupId>sg-0ccc</groupId></CreateSecurityGroupResponse>"#,
    );
    let authorize = mock_ec2(
      &mut server,
      "AuthorizeSecurityGroupIngress",
      r#"<AuthorizeSecurityGroupIngressResponse><return>true</return></AuthorizeSecurityGroupIngressResponse>"#,
    );
    let run = mock_ec2(
      &mut server,
      "RunInstances",
      r#"<RunInstancesResponse><instancesSet><item>
        <instanceId>i-0abc123def4567890</instanceId>
        <instanceState><code>0</code><name>pending</name></instanceState>
      </item></instancesSet></RunInstancesResponse>"#,
    );
    let describe = mock_ec2(
      &mut server,
      "DescribeInstances",
      r#"<DescribeInstancesResponse><reservationSet><item><instancesSet><item>
        <instanceId>i-0abc123def4567890</instanceId>
        <instanceState><code>16</code><name>running</name></instanceState>
        <ipAddress>203.0.113.42</ipAddress>
      </item></instancesSet></item></reservationSet></DescribeInstancesResponse>"#,
    );

    let adapter = Ec2ProviderAdapter::with_base_url(&server.url());

    let provisioned = adapter
      .create_node(
        &aws_credential(),
        "Primary Node",
        "eu-west-2",
        "t3.micro",
        "#cloud-config\npackages: [wireguard]\n",
      )
      .expect("provisioning should succeed against the mocked AWS API");

    ssm.assert();
    vpcs.assert();
    subnets.assert();
    create_group.assert();
    authorize.assert();
    run.assert();
    describe.assert();

    assert_eq!(provisioned.public_ipv4, "203.0.113.42");
    assert_eq!(provisioned.endpoint_port, DEFAULT_WIREGUARD_PORT);
    // The region travels with the node id, because status, exists and delete need it to build a
    // signed request and the trait does not pass a region to them.
    assert_eq!(provisioned.node_id, "eu-west-2/i-0abc123def4567890");
  }

  /// A Linode token is a single field, so the credential the adapter receives is the legacy
  /// bare-token shape as well as the JSON one.
  fn linode_credential() -> String {
    single_field_credential("linode_test_token").to_stored_value()
  }

  fn mock_linode(
    server: &mut mockito::Server,
    method: &str,
    path: &str,
    body: &str,
  ) -> mockito::Mock {
    server
      .mock(method, path)
      .match_header("authorization", mockito::Matcher::Regex("^Bearer .+".to_string()))
      .with_status(200)
      .with_body(body)
      .expect_at_least(1)
      .create()
  }

  #[test]
  fn linode_create_node_bakes_the_payload_and_reports_its_endpoint() {
    let mut server = mockito::Server::new();

    let create = mock_linode(
      &mut server,
      "POST",
      "/v4/linode/instances",
      r#"{"id":2002,"label":"primary-node","status":"provisioning","ipv4":["198.51.100.20"]}"#,
    );
    let poll = mock_linode(
      &mut server,
      "GET",
      "/v4/linode/instances/2002",
      r#"{"id":2002,"label":"primary-node","status":"running","ipv4":["198.51.100.20"]}"#,
    );

    let adapter = LinodeProviderAdapter::with_base_url(&server.url());

    let provisioned = adapter
      .create_node(
        &linode_credential(),
        "Primary Node",
        "us-ord",
        "g6-nanode-1",
        "#cloud-config\npackages: [wireguard]\n",
      )
      .expect("provisioning should succeed against the mocked Linode API");

    create.assert();
    poll.assert();

    assert_eq!(provisioned.public_ipv4, "198.51.100.20");
    assert_eq!(provisioned.endpoint_port, DEFAULT_WIREGUARD_PORT);
    // Linode's API is global, so the id needs no region prefix to be actionable later.
    assert_eq!(provisioned.node_id, "2002");
  }

  /// Once the create call returns, a billable Linode exists. The orchestrator's rollback only
  /// fires after `create_node` returns a node id, so a later failure has to clean up itself.
  #[test]
  fn linode_create_node_deletes_the_instance_when_it_never_starts() {
    let mut server = mockito::Server::new();

    let _create = mock_linode(
      &mut server,
      "POST",
      "/v4/linode/instances",
      r#"{"id":2002,"label":"n","status":"provisioning","ipv4":["198.51.100.20"]}"#,
    );
    let _poll = mock_linode(
      &mut server,
      "GET",
      "/v4/linode/instances/2002",
      r#"{"id":2002,"label":"n","status":"offline","ipv4":["198.51.100.20"]}"#,
    );
    let delete = mock_linode(&mut server, "DELETE", "/v4/linode/instances/2002", "{}");

    let adapter = LinodeProviderAdapter::with_base_url(&server.url());

    let error = adapter
      .create_node(&linode_credential(), "n", "us-ord", "g6-nanode-1", "#cloud-config\n")
      .expect_err("an instance that never runs must fail");

    delete.assert();
    assert!(
      error.to_string().contains("nothing is still billing"),
      "the user must be told the instance was cleaned up; was: {error}"
    );
  }

  /// A delete that fails must name the instance rather than reporting success — otherwise the
  /// user pays for a Linode the app has forgotten (AGENTS.md §8).
  #[test]
  fn linode_reports_an_instance_it_could_not_delete() {
    let mut server = mockito::Server::new();

    let _create = mock_linode(
      &mut server,
      "POST",
      "/v4/linode/instances",
      r#"{"id":2002,"label":"n","status":"provisioning","ipv4":["198.51.100.20"]}"#,
    );
    let _poll = mock_linode(
      &mut server,
      "GET",
      "/v4/linode/instances/2002",
      r#"{"id":2002,"label":"n","status":"offline","ipv4":["198.51.100.20"]}"#,
    );
    let _delete = server
      .mock("DELETE", "/v4/linode/instances/2002")
      .with_status(500)
      .expect_at_least(1)
      .create();

    let adapter = LinodeProviderAdapter::with_base_url(&server.url());

    let error = adapter
      .create_node(&linode_credential(), "n", "us-ord", "g6-nanode-1", "#cloud-config\n")
      .expect_err("the failure must surface");

    let rendered = error.to_string();
    assert!(rendered.contains("2002"), "the surviving instance must be named; was: {rendered}");
    assert!(
      rendered.contains("could NOT be deleted"),
      "a failed cleanup must not read as success; was: {rendered}"
    );
  }

  /// A region without the Metadata service accepts `user_data` and ignores it, producing a server
  /// that boots cleanly and hand-shakes with nobody. Such regions must never reach the picker.
  #[test]
  fn linode_lists_only_regions_that_run_the_metadata_service() {
    // Built directly rather than through `mock_linode`: the catalogue endpoints are paginated, so
    // the request carries a query string the helper cannot express.
    let mut server = mockito::Server::new();
    let _regions = server
      .mock("GET", "/v4/regions")
      .match_query(mockito::Matcher::Any)
      .with_status(200)
      .with_body(
        r#"{"data":[
          {"id":"us-ord","status":"ok","capabilities":["Linodes","Metadata"]},
          {"id":"ap-west","status":"ok","capabilities":["Linodes"]}
        ],"pages":1}"#,
      )
      .expect_at_least(1)
      .create();

    let adapter = LinodeProviderAdapter::with_base_url(&server.url());
    let regions = adapter
      .list_regions(&linode_credential())
      .expect("regions should list");

    let slugs: Vec<&str> = regions.iter().map(|region| region.slug.as_str()).collect();
    assert_eq!(slugs, vec!["us-ord"], "a non-metadata region must not be offered");
    assert_eq!(regions[0].city.as_deref(), Some("Chicago"));
  }

  #[test]
  fn linode_pricing_lists_shared_plans_cheapest_first_with_live_prices() {
    let mut server = mockito::Server::new();
    let _types = server
      .mock("GET", "/v4/linode/types")
      .match_query(mockito::Matcher::Any)
      .with_status(200)
      .with_body(
        r#"{"data":[
          {"id":"g6-standard-1","label":"Linode 2GB","class":"standard","memory":2048,"vcpus":1,
           "disk":51200,"transfer":2000,"price":{"monthly":12.0,"hourly":0.018},"region_prices":[]},
          {"id":"g6-nanode-1","label":"Nanode 1GB","class":"nanode","memory":1024,"vcpus":1,
           "disk":25600,"transfer":1000,"price":{"monthly":5.0,"hourly":0.0075},"region_prices":[]},
          {"id":"g7-gpu-x","label":"GPU","class":"gpu","memory":32768,"vcpus":8,
           "disk":819200,"transfer":10000,"price":{"monthly":1000.0,"hourly":1.5},"region_prices":[]}
        ],"pages":1}"#,
      )
      .expect_at_least(1)
      .create();

    let adapter = LinodeProviderAdapter::with_base_url(&server.url());
    let pricing = adapter
      .list_plan_pricing(&linode_credential(), "us-ord")
      .expect("pricing should list");

    let plans: Vec<&str> = pricing.iter().map(|plan| plan.size_slug.as_str()).collect();
    assert_eq!(
      plans,
      vec!["g6-nanode-1", "g6-standard-1"],
      "shared plans only, cheapest first — a GPU plan is an expensive mistake to offer"
    );
    assert_eq!(pricing[0].monthly_usd, Some(5.0));
    assert!(!pricing[0].estimated, "these are live prices, not a curated table");
    // Linode reports disk in MB; the field is GB everywhere else.
    assert_eq!(pricing[0].disk_gb, 25);
  }

  /// A node id from another provider must be refused rather than parsed into someone else's
  /// instance number.
  #[test]
  fn linode_rejects_a_node_reference_that_is_not_its_own() {
    let adapter = LinodeProviderAdapter::new();

    let error = adapter
      .delete_node(&linode_credential(), "us-east-1/i-0abc123def4567890")
      .expect_err("an EC2 node id must not be accepted");

    assert!(matches!(error, ProviderAdapterError::Unsupported { .. }));
  }

  #[test]
  fn linode_sanitizes_labels_to_what_the_api_accepts() {
    assert_eq!(sanitize_linode_label("Primary Node"), "Primary-Node");
    assert_eq!(sanitize_linode_label("  weird///name  "), "weird-name");
    // Under Linode's 3-character minimum, so it falls back rather than being rejected at create.
    assert_eq!(sanitize_linode_label("a"), "blink-node");
    assert_eq!(sanitize_linode_label("***"), "blink-node");
    assert!(sanitize_linode_label(&"x".repeat(120)).len() <= 64);
  }

  /// The payload is checked after encoding, because that is the form Linode's limit applies to.
  #[test]
  fn linode_rejects_a_payload_over_the_user_data_limit() {
    let adapter = LinodeProviderAdapter::new();
    // Base64 inflates by 4/3, so this is over the limit encoded but under it raw.
    let oversized = "#cloud-config\n".to_string() + &"x".repeat(60_000);

    let error = adapter
      .create_node(&linode_credential(), "n", "us-ord", "g6-nanode-1", &oversized)
      .expect_err("an oversized payload must be refused before a billable instance exists");

    assert!(matches!(error, ProviderAdapterError::Unsupported { .. }));
    assert!(error.to_string().contains("user-data limit"), "was: {error}");
  }

  /// The rendered cloud-init must fit Linode's user-data ceiling once encoded, the same guard the
  /// EC2 adapter keeps against an edit to the shared bootstrap script.
  #[test]
  fn the_rendered_cloud_init_fits_inside_the_linode_user_data_limit() {
    use crate::backend::bootstrap::{
      allocate_peer_addresses, Bootstrap, BootstrapParams, BootstrapPeer, DEFAULT_PEER_SLOTS,
      DEFAULT_SERVER_INTERFACE_CIDR,
    };
    use crate::backend::crypto::keypair::{
      generate_client_keypair, generate_preshared_key, generate_server_keypair,
    };

    let server = generate_server_keypair().expect("server keypair should generate");
    let peers = allocate_peer_addresses(DEFAULT_SERVER_INTERFACE_CIDR, DEFAULT_PEER_SLOTS)
      .expect("the default subnet has room for a full slot complement")
      .into_iter()
      .map(|address_cidr| BootstrapPeer {
        public_key: generate_client_keypair()
          .expect("client keypair should generate")
          .public_key,
        address_cidr,
        preshared_key: generate_preshared_key().expect("preshared key should generate"),
      })
      .collect();

    let artifact = Bootstrap
      .render_cloud_init(
        &BootstrapParams {
          peers,
          ..BootstrapParams::default()
        },
        &server.private_key,
        &server.public_key,
      )
      .expect("cloud-init should render");

    let encoded = base64_encode(artifact.init_payload.as_bytes());
    assert!(
      encoded.len() <= LINODE_USER_DATA_LIMIT_BYTES,
      "encoded payload is {} bytes, over Linode's {LINODE_USER_DATA_LIMIT_BYTES}-byte limit",
      encoded.len()
    );
  }

  /// The GCE credential is one field holding a whole service account key file.
  fn gce_credential() -> String {
    ProviderCredential::from_fields([(
      "service_account_json".to_string(),
      super::gce::tests::test_service_account_json(),
    )])
    .to_stored_value()
  }

  /// Both the token endpoint and the compute API are served by one mock, the way the EC2 tests
  /// serve STS, SSM and EC2 from a single server.
  fn mock_gce_token(server: &mut mockito::Server) -> mockito::Mock {
    server
      .mock("POST", "/token")
      .with_status(200)
      .with_body(r#"{"access_token":"ya29.test-token","expires_in":3600,"token_type":"Bearer"}"#)
      .expect_at_least(1)
      .create()
  }

  fn mock_gce(
    server: &mut mockito::Server,
    method: &str,
    path: &str,
    body: &str,
  ) -> mockito::Mock {
    server
      .mock(method, path)
      .match_header("authorization", mockito::Matcher::Regex("^Bearer .+".to_string()))
      .with_status(200)
      .with_body(body)
      .expect_at_least(1)
      .create()
  }

  const GCE_PROJECT_PATH: &str = "/compute/v1/projects/blink-test";

  #[test]
  fn gce_create_node_opens_the_port_and_reports_its_endpoint() {
    let mut server = mockito::Server::new();
    let token = mock_gce_token(&mut server);
    let network = mock_gce(
      &mut server,
      "GET",
      &format!("{GCE_PROJECT_PATH}/global/networks/default"),
      r#"{"name":"default"}"#,
    );
    // The default network passes no UDP 51820, so this rule is what makes the server reachable.
    let firewall = mock_gce(
      &mut server,
      "POST",
      &format!("{GCE_PROJECT_PATH}/global/firewalls"),
      r#"{"name":"op-1","status":"RUNNING"}"#,
    );
    let insert = server
      .mock("POST", mockito::Matcher::Regex(format!("{GCE_PROJECT_PATH}/zones/.+/instances$")))
      .with_status(200)
      .with_body(r#"{"name":"op-2","status":"RUNNING"}"#)
      .expect_at_least(1)
      .create();
    let get = server
      .mock("GET", mockito::Matcher::Regex(format!("{GCE_PROJECT_PATH}/zones/.+/instances/.+")))
      .with_status(200)
      .with_body(
        r#"{"status":"RUNNING","networkInterfaces":[{"networkIP":"10.128.0.2",
          "accessConfigs":[{"type":"ONE_TO_ONE_NAT","natIP":"203.0.113.90"}]}]}"#,
      )
      .expect_at_least(1)
      .create();

    let adapter = GceProviderAdapter::with_base_url(&server.url());
    let provisioned = adapter
      .create_node(
        &gce_credential(),
        "Primary Node",
        "europe-west2-a",
        "e2-micro",
        "#cloud-config\npackages: [wireguard]\n",
      )
      .expect("provisioning should succeed against the mocked Google API");

    token.assert();
    network.assert();
    firewall.assert();
    insert.assert();
    get.assert();

    // The external address comes from the access config; `networkIP` would be an unreachable
    // internal 10.x.
    assert_eq!(provisioned.public_ipv4, "203.0.113.90");
    assert_eq!(provisioned.endpoint_port, DEFAULT_WIREGUARD_PORT);
    // The zone travels with the node id, because status, exists and delete need it to address the
    // instance and the trait does not pass one.
    assert!(
      provisioned.node_id.starts_with("europe-west2-a/"),
      "the node id should carry its zone, got {}",
      provisioned.node_id
    );
  }

  /// A project whose default network was deleted cannot launch, and saying so beats a failed
  /// insert whose message never mentions the network.
  #[test]
  fn gce_create_node_reports_a_missing_default_network_as_a_capability_error() {
    let mut server = mockito::Server::new();
    let _token = mock_gce_token(&mut server);
    let _network = server
      .mock("GET", &*format!("{GCE_PROJECT_PATH}/global/networks/default"))
      .with_status(404)
      .with_body(r#"{"error":{"code":404,"message":"not found"}}"#)
      .expect_at_least(1)
      .create();

    let adapter = GceProviderAdapter::with_base_url(&server.url());
    let error = adapter
      .create_node(&gce_credential(), "Primary Node", "us-central1-a", "e2-micro", "#cloud-config\n")
      .expect_err("a missing default network must fail");

    assert!(matches!(error, ProviderAdapterError::Unsupported { .. }));
    assert!(error.to_string().contains("default"), "was: {error}");
  }

  /// A failed insert leaves nothing billing, but the firewall rule would linger and block a retry
  /// that reuses the name.
  #[test]
  fn gce_create_node_removes_the_firewall_rule_when_the_insert_fails() {
    let mut server = mockito::Server::new();
    let _token = mock_gce_token(&mut server);
    let _network = mock_gce(
      &mut server,
      "GET",
      &format!("{GCE_PROJECT_PATH}/global/networks/default"),
      r#"{"name":"default"}"#,
    );
    let _firewall = mock_gce(
      &mut server,
      "POST",
      &format!("{GCE_PROJECT_PATH}/global/firewalls"),
      r#"{"name":"op-1"}"#,
    );
    let _insert = server
      .mock("POST", mockito::Matcher::Regex(format!("{GCE_PROJECT_PATH}/zones/.+/instances$")))
      .with_status(400)
      .with_body(r#"{"error":{"code":400,"message":"Invalid machine type"}}"#)
      .expect_at_least(1)
      .create();
    let firewall_delete = server
      .mock("DELETE", mockito::Matcher::Regex(format!("{GCE_PROJECT_PATH}/global/firewalls/.+")))
      .with_status(200)
      .with_body("{}")
      .expect_at_least(1)
      .create();

    let adapter = GceProviderAdapter::with_base_url(&server.url());
    let error = adapter
      .create_node(&gce_credential(), "Primary Node", "us-central1-a", "e2-micro", "#cloud-config\n")
      .expect_err("a failed insert must surface");

    // The rule is gone, so a retry can reuse the name.
    firewall_delete.assert();
    assert!(error.to_string().contains("insert call failed"), "was: {error}");
    // Google's own explanation travels in the source chain, which is what diagnostics render.
    assert!(
      error_chain_text(&error).contains("Invalid machine type"),
      "the provider's reason must survive: {}",
      error_chain_text(&error)
    );
  }

  /// The full `Display` text of an error and everything it was caused by. Mirrors what the
  /// orchestrator's `describe_error_chain` puts in front of a user.
  fn error_chain_text(error: &dyn std::error::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
      parts.push(cause.to_string());
      source = cause.source();
    }
    parts.join(" | ")
  }

  /// Delete removes the instance and then its rule. The boot disk is `autoDelete`, so it goes with
  /// the instance — a disk left behind is the classic way a "deleted" GCE node keeps billing.
  #[test]
  fn gce_delete_node_removes_the_instance_and_its_firewall_rule() {
    let mut server = mockito::Server::new();
    let _token = mock_gce_token(&mut server);
    let instance_delete = server
      .mock("DELETE", mockito::Matcher::Regex(format!("{GCE_PROJECT_PATH}/zones/.+/instances/.+")))
      .with_status(200)
      .with_body("{}")
      .expect_at_least(1)
      .create();
    let firewall_delete = server
      .mock("DELETE", mockito::Matcher::Regex(format!("{GCE_PROJECT_PATH}/global/firewalls/.+")))
      .with_status(200)
      .with_body("{}")
      .expect_at_least(1)
      .create();

    let adapter = GceProviderAdapter::with_base_url(&server.url());
    adapter
      .delete_node(&gce_credential(), "us-central1-a/blink-node-abc12345")
      .expect("delete should succeed");

    instance_delete.assert();
    firewall_delete.assert();
  }

  /// A rule that cannot be removed is reported rather than swallowed — it is not billable, but
  /// leaving it silently blocks a later node from reusing the name.
  #[test]
  fn gce_reports_a_firewall_rule_it_could_not_remove() {
    let mut server = mockito::Server::new();
    let _token = mock_gce_token(&mut server);
    let _instance_delete = server
      .mock("DELETE", mockito::Matcher::Regex(format!("{GCE_PROJECT_PATH}/zones/.+/instances/.+")))
      .with_status(200)
      .with_body("{}")
      .expect_at_least(1)
      .create();
    let _firewall_delete = server
      .mock("DELETE", mockito::Matcher::Regex(format!("{GCE_PROJECT_PATH}/global/firewalls/.+")))
      .with_status(500)
      .with_body(r#"{"error":{"code":500,"message":"backend error"}}"#)
      .expect_at_least(1)
      .create();

    let adapter = GceProviderAdapter::with_base_url(&server.url());
    let error = adapter
      .delete_node(&gce_credential(), "us-central1-a/blink-node-abc12345")
      .expect_err("a failed rule delete must surface");

    let rendered = error.to_string();
    assert!(rendered.contains("could NOT be removed"), "was: {rendered}");
    assert!(rendered.contains("blink-node-abc12345-wg"), "the rule must be named: {rendered}");
  }

  /// Minting a token per API call would triple every request, so it is cached until it nears
  /// expiry.
  #[test]
  fn gce_reuses_one_access_token_across_calls() {
    let mut server = mockito::Server::new();
    // Exactly one mint, however many API calls follow.
    let token = server
      .mock("POST", "/token")
      .with_status(200)
      .with_body(r#"{"access_token":"ya29.test-token","expires_in":3600}"#)
      .expect(1)
      .create();
    let _project = mock_gce(&mut server, "GET", GCE_PROJECT_PATH, r#"{"name":"blink-test"}"#);

    let adapter = GceProviderAdapter::with_base_url(&server.url());
    let credential = gce_credential();
    adapter.validate_token(&credential).expect("first call should succeed");
    adapter.validate_token(&credential).expect("second call should succeed");

    token.assert();
  }

  #[test]
  fn gce_validate_token_reports_the_project_and_service_account() {
    let mut server = mockito::Server::new();
    let _token = mock_gce_token(&mut server);
    let _project = mock_gce(&mut server, "GET", GCE_PROJECT_PATH, r#"{"name":"blink-test"}"#);

    let adapter = GceProviderAdapter::with_base_url(&server.url());
    let identity = adapter.validate_token(&gce_credential()).expect("validation should succeed");

    // The project is what Google bills and what the instance lives in, so it is the account id.
    assert_eq!(identity.account_id, "blink-test");
    assert_eq!(
      identity.label.as_deref(),
      Some("blink@blink-test.iam.gserviceaccount.com")
    );
    // Google reports no account standing, and inventing "active" would assert something unchecked.
    assert_eq!(identity.status, None);
  }

  /// The line shown after a credential is checked, for each shape a provider can answer with.
  #[test]
  fn an_identity_describes_itself_without_inventing_the_parts_it_lacks() {
    // DigitalOcean: the only provider that reports account standing.
    assert_eq!(
      ProviderIdentity {
        account_id: "abc-123".to_string(),
        label: Some("vpn@example.com".to_string()),
        status: Some("active".to_string()),
      }
      .describe(),
      "vpn@example.com (abc-123), status active"
    );

    // Most providers: an account and something human-readable, but no standing to report.
    assert_eq!(
      ProviderIdentity {
        account_id: "blink-test".to_string(),
        label: Some("blink@blink-test.iam.gserviceaccount.com".to_string()),
        status: None,
      }
      .describe(),
      "blink@blink-test.iam.gserviceaccount.com (blink-test)"
    );

    // Docker: no account at all, so the id stands alone rather than being padded out.
    assert_eq!(
      ProviderIdentity {
        account_id: "docker-local".to_string(),
        label: None,
        status: Some("daemon 27.0.2".to_string()),
      }
      .describe(),
      "docker-local, status daemon 27.0.2"
    );

    // An empty string is treated as absent: a stray "()" or a dangling "status" reads as a bug.
    assert_eq!(
      ProviderIdentity {
        account_id: "acct".to_string(),
        label: Some("  ".to_string()),
        status: Some(String::new()),
      }
      .describe(),
      "acct"
    );
  }

  /// Every adapter must report the account that will actually be billed.
  ///
  /// This is what the old `Account` type could not express: its `uuid`/`email`/`status` were
  /// DigitalOcean's field names, so EC2 put an ARN in `email` and everyone hardcoded
  /// `status: "active"` — a claim those providers never made.
  #[test]
  fn no_adapter_claims_an_account_status_its_provider_does_not_report() {
    let mut server = mockito::Server::new();
    let _token = mock_gce_token(&mut server);
    let _project = mock_gce(&mut server, "GET", GCE_PROJECT_PATH, r#"{"name":"blink-test"}"#);

    let identity = GceProviderAdapter::with_base_url(&server.url())
      .validate_token(&gce_credential())
      .expect("validation should succeed");

    assert!(
      !identity.account_id.trim().is_empty(),
      "the account that gets billed must always be named"
    );
    assert_eq!(
      identity.status, None,
      "Google reports no account standing, so none may be asserted on its behalf"
    );
  }

  /// A pasted excerpt, or a key for a project the account cannot see, must be named as a
  /// credential problem rather than surfacing later as a puzzling 404.
  #[test]
  fn gce_rejects_a_credential_that_is_not_a_service_account_key() {
    let credential =
      ProviderCredential::from_fields([("service_account_json".to_string(), "not json".to_string())])
        .to_stored_value();

    let error = GceProviderAdapter::new()
      .validate_token(&credential)
      .expect_err("garbage must be rejected");

    assert!(matches!(error, ProviderAdapterError::Unsupported { .. }));
    assert!(error.to_string().contains("Re-save"), "was: {error}");
  }

  #[test]
  fn gce_rejects_a_node_reference_that_is_not_its_own() {
    let error = GceProviderAdapter::new()
      .delete_node(&gce_credential(), "12345")
      .expect_err("a bare Linode id must not be accepted");

    assert!(matches!(error, ProviderAdapterError::Unsupported { .. }));
    assert!(error.to_string().contains("<zone>/<instance-name>"), "was: {error}");
  }

  #[test]
  fn gce_sanitizes_instance_names_to_what_the_api_accepts() {
    let name = sanitize_gce_instance_name("Primary Node");
    assert!(name.starts_with("primary-node-"), "was: {name}");
    assert!(name.len() <= 63, "GCE caps instance names at 63 characters: {name}");

    // A name must start with a letter.
    assert!(sanitize_gce_instance_name("2nd node").starts_with("blink-"));
    assert!(sanitize_gce_instance_name("***").starts_with("blink-"));
    assert!(sanitize_gce_instance_name(&"x".repeat(200)).len() <= 63);

    // The name is the instance's identity within a project and zone, so two nodes the user calls
    // the same thing must not collide — GCE rejects the second rather than disambiguating it.
    assert_ne!(
      sanitize_gce_instance_name("Primary Node"),
      sanitize_gce_instance_name("Primary Node")
    );
  }

  /// GCE NICs run at 1460, so the default 1420 tunnel MTU is 40 bytes too large. The hint is what
  /// keeps a GCE tunnel from connecting and then stalling on large transfers.
  #[test]
  fn gce_asks_client_configs_for_a_smaller_mtu_and_other_providers_do_not() {
    assert_eq!(GceProviderAdapter::new().client_mtu_hint(), Some(1380));
    assert_eq!(LinodeProviderAdapter::new().client_mtu_hint(), None);
    assert_eq!(Ec2ProviderAdapter::new().client_mtu_hint(), None);
  }

  /// GCE takes metadata verbatim, so its limit applies to the rendered script as it stands.
  #[test]
  fn the_rendered_cloud_init_fits_inside_the_gce_metadata_limit() {
    use crate::backend::bootstrap::{
      allocate_peer_addresses, Bootstrap, BootstrapParams, BootstrapPeer, DEFAULT_PEER_SLOTS,
      DEFAULT_SERVER_INTERFACE_CIDR,
    };
    use crate::backend::crypto::keypair::{
      generate_client_keypair, generate_preshared_key, generate_server_keypair,
    };

    let server = generate_server_keypair().expect("server keypair should generate");
    let peers = allocate_peer_addresses(DEFAULT_SERVER_INTERFACE_CIDR, DEFAULT_PEER_SLOTS)
      .expect("the default subnet has room for a full slot complement")
      .into_iter()
      .map(|address_cidr| BootstrapPeer {
        public_key: generate_client_keypair()
          .expect("client keypair should generate")
          .public_key,
        address_cidr,
        preshared_key: generate_preshared_key().expect("preshared key should generate"),
      })
      .collect();

    let artifact = Bootstrap
      .render_cloud_init(
        &BootstrapParams {
          peers,
          ..BootstrapParams::default()
        },
        &server.private_key,
        &server.public_key,
      )
      .expect("cloud-init should render");

    assert!(
      artifact.init_payload.len() <= GCE_USER_DATA_LIMIT_BYTES,
      "payload is {} bytes, over Google's {GCE_USER_DATA_LIMIT_BYTES}-byte metadata limit",
      artifact.init_payload.len()
    );
  }

  /// An OpenStack credential pointed at a mock server.
  ///
  /// There is no `with_base_url` for this adapter: the auth URL *is* part of the credential, and
  /// every other address comes from the catalogue authentication returns. Pointing the adapter at
  /// a mock is therefore just a matter of what the credential says.
  fn openstack_credential(base_url: &str) -> String {
    ProviderCredential::from_fields([
      ("auth_url".to_string(), format!("{base_url}/v3")),
      ("application_credential_id".to_string(), "cred-id".to_string()),
      (
        "application_credential_secret".to_string(),
        "cred-secret-not-real".to_string(),
      ),
    ])
    .to_stored_value()
  }

  /// Keystone's answer: a token in a header, and a catalogue whose endpoints point back at this
  /// same mock so Nova, Neutron and Glance are all served from one server.
  fn mock_keystone(server: &mut mockito::Server, region: &str) -> mockito::Mock {
    let base = server.url();
    let body = serde_json::json!({
      "token": {
        "project": { "id": "proj-1" },
        "catalog": [
          { "type": "compute", "endpoints": [
            { "interface": "public", "region": region, "url": format!("{base}/compute") }]},
          { "type": "network", "endpoints": [
            { "interface": "public", "region": region, "url": format!("{base}/network") }]},
          { "type": "image", "endpoints": [
            { "interface": "public", "region": region, "url": format!("{base}/image") }]},
        ],
      },
    })
    .to_string();

    server
      .mock("POST", "/v3/auth/tokens")
      .with_status(201)
      .with_header("x-subject-token", "gAAAAAtest-token")
      .with_body(body)
      .expect_at_least(1)
      .create()
  }

  fn mock_os(
    server: &mut mockito::Server,
    method: &str,
    path: &str,
    body: &str,
  ) -> mockito::Mock {
    server
      .mock(method, path)
      .match_header("x-auth-token", mockito::Matcher::Any)
      .with_status(200)
      .with_body(body)
      .expect_at_least(1)
      .create()
  }

  const OS_UBUNTU_IMAGES: &str =
    r#"{"images":[{"id":"img-1","name":"Ubuntu 24.04 LTS (Noble)"},{"id":"img-9","name":"CentOS 9"}]}"#;

  /// The common shape: a tenant network, so the server comes up on a private address and needs a
  /// floating IP allocated and attached.
  #[test]
  fn openstack_create_node_allocates_a_floating_ip_for_a_private_server() {
    let mut server = mockito::Server::new();
    let keystone = mock_keystone(&mut server, "RegionOne");
    let _images = mock_os(&mut server, "GET", "/image/v2/images?status=active&limit=1000", OS_UBUNTU_IMAGES);
    let _networks = mock_os(
      &mut server,
      "GET",
      "/network/v2.0/networks",
      r#"{"networks":[{"id":"net-1","name":"private","status":"ACTIVE","router:external":false,"shared":false},
        {"id":"net-ext","name":"public","status":"ACTIVE","router:external":true,"shared":false}]}"#,
    );
    let group = mock_os(
      &mut server,
      "POST",
      "/network/v2.0/security-groups",
      r#"{"security_group":{"id":"sg-1"}}"#,
    );
    let rule = mock_os(
      &mut server,
      "POST",
      "/network/v2.0/security-group-rules",
      r#"{"security_group_rule":{"id":"rule-1"}}"#,
    );
    let create = mock_os(
      &mut server,
      "POST",
      "/compute/servers",
      r#"{"server":{"id":"srv-1","status":"BUILD"}}"#,
    );
    let _poll = mock_os(
      &mut server,
      "GET",
      "/compute/servers/srv-1",
      r#"{"server":{"id":"srv-1","status":"ACTIVE",
        "addresses":{"private":[{"addr":"10.0.0.5","OS-EXT-IPS:type":"fixed"}]}}}"#,
    );
    let ports = mock_os(
      &mut server,
      "GET",
      "/network/v2.0/ports?device_id=srv-1",
      r#"{"ports":[{"id":"port-1"}]}"#,
    );
    let floating = mock_os(
      &mut server,
      "POST",
      "/network/v2.0/floatingips",
      r#"{"floatingip":{"id":"fip-1","floating_ip_address":"203.0.113.55","port_id":"port-1"}}"#,
    );

    let adapter = OpenStackProviderAdapter::with_fast_polling();
    let provisioned = adapter
      .create_node(
        &openstack_credential(&server.url()),
        "Primary Node",
        "RegionOne",
        "flavor-2",
        "#cloud-config\npackages: [wireguard]\n",
      )
      .expect("provisioning should succeed against the mocked OpenStack cloud");

    keystone.assert();
    group.assert();
    rule.assert();
    create.assert();
    ports.assert();
    floating.assert();

    assert_eq!(provisioned.public_ipv4, "203.0.113.55");
    assert_eq!(provisioned.endpoint_port, DEFAULT_WIREGUARD_PORT);
    // The region travels with the node id, because status, exists and delete need it to pick a
    // catalogue endpoint and the trait does not pass one.
    assert_eq!(provisioned.node_id, "RegionOne/srv-1");
  }

  /// The DreamCompute shape: attached straight to a shared external network, so the address on the
  /// server is already routable and no floating IP is involved. Algo assumed only this shape and
  /// broke on clouds that need the other.
  #[test]
  fn openstack_create_node_skips_the_floating_ip_when_the_address_is_already_routable() {
    let mut server = mockito::Server::new();
    let _keystone = mock_keystone(&mut server, "RegionOne");
    let _images = mock_os(&mut server, "GET", "/image/v2/images?status=active&limit=1000", OS_UBUNTU_IMAGES);
    let _networks = mock_os(
      &mut server,
      "GET",
      "/network/v2.0/networks",
      r#"{"networks":[{"id":"net-ext","name":"public","status":"ACTIVE","router:external":true,"shared":true}]}"#,
    );
    let _group = mock_os(
      &mut server,
      "POST",
      "/network/v2.0/security-groups",
      r#"{"security_group":{"id":"sg-1"}}"#,
    );
    let _rule = mock_os(
      &mut server,
      "POST",
      "/network/v2.0/security-group-rules",
      r#"{"security_group_rule":{"id":"rule-1"}}"#,
    );
    let _create = mock_os(
      &mut server,
      "POST",
      "/compute/servers",
      r#"{"server":{"id":"srv-1","status":"BUILD"}}"#,
    );
    let _poll = mock_os(
      &mut server,
      "GET",
      "/compute/servers/srv-1",
      r#"{"server":{"id":"srv-1","status":"ACTIVE",
        "addresses":{"public":[{"addr":"198.51.100.8","OS-EXT-IPS:type":"fixed"}]}}}"#,
    );
    // Never called on this path — allocating one would leave a second billable resource behind.
    let floating = server
      .mock("POST", "/network/v2.0/floatingips")
      .with_status(200)
      .with_body(r#"{"floatingip":{"id":"fip-1","floating_ip_address":"203.0.113.55"}}"#)
      .expect(0)
      .create();

    let adapter = OpenStackProviderAdapter::with_fast_polling();
    let provisioned = adapter
      .create_node(
        &openstack_credential(&server.url()),
        "Primary Node",
        "RegionOne",
        "flavor-2",
        "#cloud-config\n",
      )
      .expect("provisioning should succeed");

    floating.assert();
    assert_eq!(provisioned.public_ipv4, "198.51.100.8");
  }

  /// A cloud with nothing to attach to gets a capability error naming what is missing, not a
  /// failed create.
  #[test]
  fn openstack_reports_a_cloud_with_no_usable_network_as_a_capability_error() {
    let mut server = mockito::Server::new();
    let _keystone = mock_keystone(&mut server, "RegionOne");
    let _images = mock_os(&mut server, "GET", "/image/v2/images?status=active&limit=1000", OS_UBUNTU_IMAGES);
    let _networks = mock_os(&mut server, "GET", "/network/v2.0/networks", r#"{"networks":[]}"#);

    let error = OpenStackProviderAdapter::with_fast_polling()
      .create_node(
        &openstack_credential(&server.url()),
        "Primary Node",
        "RegionOne",
        "flavor-2",
        "#cloud-config\n",
      )
      .expect_err("a cloud with no network must fail");

    assert!(matches!(error, ProviderAdapterError::Unsupported { .. }));
    assert!(error.to_string().contains("no network"), "was: {error}");
  }

  /// A cloud without a supported Ubuntu tells the user what it does have, which beats
  /// "image not found" for a name only the operator knows.
  #[test]
  fn openstack_reports_a_missing_ubuntu_image_and_lists_what_is_available() {
    let mut server = mockito::Server::new();
    let _keystone = mock_keystone(&mut server, "RegionOne");
    let _images = mock_os(
      &mut server,
      "GET",
      "/image/v2/images?status=active&limit=1000",
      r#"{"images":[{"id":"img-9","name":"CentOS Stream 9"},{"id":"img-8","name":"Debian 12"}]}"#,
    );

    let error = OpenStackProviderAdapter::with_fast_polling()
      .create_node(
        &openstack_credential(&server.url()),
        "Primary Node",
        "RegionOne",
        "flavor-2",
        "#cloud-config\n",
      )
      .expect_err("a cloud with no Ubuntu image must fail");

    assert!(matches!(error, ProviderAdapterError::Unsupported { .. }));
    let rendered = error.to_string();
    assert!(rendered.contains("CentOS Stream 9"), "the user needs to see what is on offer: {rendered}");
  }

  /// Once the server exists it is billing, so a later failure must remove it rather than return.
  #[test]
  fn openstack_create_node_tears_down_everything_when_the_server_never_becomes_active() {
    let mut server = mockito::Server::new();
    let _keystone = mock_keystone(&mut server, "RegionOne");
    let _images = mock_os(&mut server, "GET", "/image/v2/images?status=active&limit=1000", OS_UBUNTU_IMAGES);
    let _networks = mock_os(
      &mut server,
      "GET",
      "/network/v2.0/networks",
      r#"{"networks":[{"id":"net-1","name":"private","status":"ACTIVE","router:external":false,"shared":false}]}"#,
    );
    let _group = mock_os(
      &mut server,
      "POST",
      "/network/v2.0/security-groups",
      r#"{"security_group":{"id":"sg-1"}}"#,
    );
    let _rule = mock_os(
      &mut server,
      "POST",
      "/network/v2.0/security-group-rules",
      r#"{"security_group_rule":{"id":"rule-1"}}"#,
    );
    let _create = mock_os(
      &mut server,
      "POST",
      "/compute/servers",
      r#"{"server":{"id":"srv-1","status":"BUILD"}}"#,
    );
    // ERROR is terminal: waiting out the timeout would only delay the same failure.
    let _poll = mock_os(
      &mut server,
      "GET",
      "/compute/servers/srv-1",
      r#"{"server":{"id":"srv-1","status":"ERROR","fault":{"message":"No valid host was found"}}}"#,
    );
    let server_delete = server
      .mock("DELETE", "/compute/servers/srv-1")
      .with_status(204)
      .expect_at_least(1)
      .create();
    let group_delete = server
      .mock("DELETE", "/network/v2.0/security-groups/sg-1")
      .with_status(204)
      .expect_at_least(1)
      .create();

    let error = OpenStackProviderAdapter::with_fast_polling()
      .create_node(
        &openstack_credential(&server.url()),
        "Primary Node",
        "RegionOne",
        "flavor-2",
        "#cloud-config\n",
      )
      .expect_err("a server that errors must fail");

    server_delete.assert();
    group_delete.assert();
    assert!(
      error.to_string().contains("nothing is still billing"),
      "the user must be told the cleanup succeeded: {error}"
    );
  }

  /// Teardown order is forced by Neutron: the group cannot go while the server's port holds it.
  #[test]
  fn openstack_delete_node_removes_the_server_its_floating_ip_and_its_group() {
    let mut server = mockito::Server::new();
    let _keystone = mock_keystone(&mut server, "RegionOne");
    let _ports = mock_os(
      &mut server,
      "GET",
      "/network/v2.0/ports?device_id=srv-1",
      r#"{"ports":[{"id":"port-1"}]}"#,
    );
    let _floating_list = mock_os(
      &mut server,
      "GET",
      "/network/v2.0/floatingips",
      r#"{"floatingips":[{"id":"fip-1","floating_ip_address":"203.0.113.55","port_id":"port-1"}]}"#,
    );
    let _server_get = mock_os(
      &mut server,
      "GET",
      "/compute/servers/srv-1",
      r#"{"server":{"id":"srv-1","status":"ACTIVE",
        "security_groups":[{"name":"blink-primary-node-abc12345"},{"name":"default"}]}}"#,
    );
    let _groups = mock_os(
      &mut server,
      "GET",
      "/network/v2.0/security-groups",
      r#"{"security_groups":[{"id":"sg-1","name":"blink-primary-node-abc12345"},
        {"id":"sg-default","name":"default"}]}"#,
    );
    let server_delete = server
      .mock("DELETE", "/compute/servers/srv-1")
      .with_status(204)
      .expect_at_least(1)
      .create();
    let floating_delete = server
      .mock("DELETE", "/network/v2.0/floatingips/fip-1")
      .with_status(204)
      .expect_at_least(1)
      .create();
    let group_delete = server
      .mock("DELETE", "/network/v2.0/security-groups/sg-1")
      .with_status(204)
      .expect_at_least(1)
      .create();
    // The operator's own group must be left alone.
    let default_group_delete = server
      .mock("DELETE", "/network/v2.0/security-groups/sg-default")
      .with_status(204)
      .expect(0)
      .create();

    OpenStackProviderAdapter::with_fast_polling()
      .delete_node(&openstack_credential(&server.url()), "RegionOne/srv-1")
      .expect("delete should succeed");

    server_delete.assert();
    floating_delete.assert();
    group_delete.assert();
    default_group_delete.assert();
  }

  /// A floating IP that outlives its server keeps billing on most clouds, so a failure to release
  /// one must be reported rather than swallowed.
  #[test]
  fn openstack_reports_a_floating_ip_it_could_not_release() {
    let mut server = mockito::Server::new();
    let _keystone = mock_keystone(&mut server, "RegionOne");
    let _ports = mock_os(
      &mut server,
      "GET",
      "/network/v2.0/ports?device_id=srv-1",
      r#"{"ports":[{"id":"port-1"}]}"#,
    );
    let _floating_list = mock_os(
      &mut server,
      "GET",
      "/network/v2.0/floatingips",
      r#"{"floatingips":[{"id":"fip-1","floating_ip_address":"203.0.113.55","port_id":"port-1"}]}"#,
    );
    let _server_get = mock_os(
      &mut server,
      "GET",
      "/compute/servers/srv-1",
      r#"{"server":{"id":"srv-1","status":"ACTIVE","security_groups":[]}}"#,
    );
    let _groups = mock_os(&mut server, "GET", "/network/v2.0/security-groups", r#"{"security_groups":[]}"#);
    let _server_delete = server
      .mock("DELETE", "/compute/servers/srv-1")
      .with_status(204)
      .expect_at_least(1)
      .create();
    let _floating_delete = server
      .mock("DELETE", "/network/v2.0/floatingips/fip-1")
      .with_status(500)
      .with_body(r#"{"NeutronError":{"message":"Internal error"}}"#)
      .expect_at_least(1)
      .create();

    let error = OpenStackProviderAdapter::with_fast_polling()
      .delete_node(&openstack_credential(&server.url()), "RegionOne/srv-1")
      .expect_err("a stuck floating IP must surface");

    let rendered = error.to_string();
    assert!(rendered.contains("203.0.113.55"), "the address must be named: {rendered}");
    assert!(rendered.contains("may still be billing"), "was: {rendered}");
  }

  /// OpenStack publishes no prices, and a rendered $0.00 would say a paid flavor is free.
  #[test]
  fn openstack_pricing_reports_no_price_rather_than_zero() {
    let mut server = mockito::Server::new();
    let _keystone = mock_keystone(&mut server, "RegionOne");
    let _flavors = mock_os(
      &mut server,
      "GET",
      "/compute/flavors/detail",
      r#"{"flavors":[
        {"id":"f-2","name":"m1.small","ram":2048,"vcpus":1,"disk":20},
        {"id":"f-1","name":"m1.tiny","ram":512,"vcpus":1,"disk":10},
        {"id":"f-x","name":"retired","ram":256,"vcpus":1,"disk":5,"OS-FLV-DISABLED:disabled":true},
        {"id":"f-p","name":"operator-only","ram":128,"vcpus":1,"disk":5,"os-flavor-access:is_public":false}
      ]}"#,
    );

    let pricing = OpenStackProviderAdapter::with_fast_polling()
      .list_plan_pricing(&openstack_credential(&server.url()), "RegionOne")
      .expect("flavors should list");

    let slugs: Vec<&str> = pricing.iter().map(|plan| plan.size_slug.as_str()).collect();
    // Smallest first; a disabled flavor cannot be launched and a non-public one is not ours.
    assert_eq!(slugs, vec!["f-1", "f-2"], "was: {slugs:?}");
    assert_eq!(pricing[0].monthly_usd, None, "a price we do not know must not be reported as zero");
    assert_eq!(pricing[0].hourly_usd, None);
    assert!(pricing[0].transfer_note.contains("operator"), "was: {}", pricing[0].transfer_note);
    // Nova launches by flavor id; the name is a label and is not guaranteed unique.
    assert_eq!(pricing[0].plan_label, "m1.tiny");
  }

  /// No flavor id is guessable across clouds, so an empty selection must be refused rather than
  /// defaulted.
  #[test]
  fn openstack_refuses_to_guess_a_flavor() {
    let error = OpenStackProviderAdapter::with_fast_polling()
      .create_node(&openstack_credential("http://127.0.0.1:1"), "n", "RegionOne", "  ", "#cloud-config\n")
      .expect_err("an empty flavor must be refused");

    assert!(matches!(error, ProviderAdapterError::Unsupported { .. }));
    assert!(error.to_string().contains("Choose a plan"), "was: {error}");
  }

  #[test]
  fn openstack_rejects_a_node_reference_that_is_not_its_own() {
    let error = OpenStackProviderAdapter::with_fast_polling()
      .delete_node(&openstack_credential("http://127.0.0.1:1"), "12345")
      .expect_err("a bare Linode id must not be accepted");

    assert!(matches!(error, ProviderAdapterError::Unsupported { .. }));
    assert!(error.to_string().contains("<region>/<server-id>"), "was: {error}");
  }

  #[test]
  fn openstack_rejects_a_credential_that_is_missing_a_field() {
    let credential = ProviderCredential::from_fields([(
      "auth_url".to_string(),
      "https://identity.example.com/v3".to_string(),
    )])
    .to_stored_value();

    let error = OpenStackProviderAdapter::with_fast_polling()
      .validate_token(&credential)
      .expect_err("a partial credential must be rejected");

    assert!(
      error.to_string().contains("application_credential_id"),
      "the missing field must be named: {error}"
    );
  }

  /// An OpenStack region name means whatever its operator decided, so no geography is invented for
  /// it — even when it collides with a name another provider owns.
  #[test]
  fn openstack_regions_carry_no_invented_geography() {
    let region = OpenStackProviderAdapter::new().describe_region("RegionOne");
    assert_eq!(region.slug, "RegionOne");
    assert_eq!(region.city, None);
    assert_eq!(region.country, None);
  }

  /// An account with no default VPC gets a capability error naming the problem, not a failed launch.
  #[test]
  fn ec2_create_node_reports_a_missing_default_vpc_as_a_capability_error() {
    let mut server = mockito::Server::new();
    let _ssm = mock_ssm_ami(&mut server);
    let _vpcs = mock_ec2(&mut server, "DescribeVpcs", r#"<DescribeVpcsResponse><vpcSet/></DescribeVpcsResponse>"#);

    let adapter = Ec2ProviderAdapter::with_base_url(&server.url());

    let error = adapter
      .create_node(&aws_credential(), "Primary Node", "eu-west-2", "t3.micro", "#cloud-config\n")
      .expect_err("a missing default VPC must fail");

    assert!(
      matches!(error, ProviderAdapterError::Unsupported { .. }),
      "expected a capability error, got {error:?}"
    );
    assert!(error.to_string().contains("default VPC"));
  }

  /// A failure after the security group exists must not leave it behind.
  #[test]
  fn ec2_create_node_removes_the_security_group_when_the_launch_fails() {
    let mut server = mockito::Server::new();

    let _ssm = mock_ssm_ami(&mut server);
    let _vpcs = mock_ec2(
      &mut server,
      "DescribeVpcs",
      r#"<DescribeVpcsResponse><vpcSet><item><vpcId>vpc-0aaa</vpcId></item></vpcSet></DescribeVpcsResponse>"#,
    );
    let _subnets = mock_ec2(
      &mut server,
      "DescribeSubnets",
      r#"<DescribeSubnetsResponse><subnetSet><item><subnetId>subnet-0bbb</subnetId></item></subnetSet></DescribeSubnetsResponse>"#,
    );
    let _create_group = mock_ec2(
      &mut server,
      "CreateSecurityGroup",
      r#"<CreateSecurityGroupResponse><groupId>sg-0ccc</groupId></CreateSecurityGroupResponse>"#,
    );
    let _authorize = mock_ec2(
      &mut server,
      "AuthorizeSecurityGroupIngress",
      r#"<AuthorizeSecurityGroupIngressResponse><return>true</return></AuthorizeSecurityGroupIngressResponse>"#,
    );

    let run_failure = server
      .mock("POST", "/ec2")
      .match_body(ec2_action("RunInstances"))
      .with_status(400)
      .with_body(
        r#"<Response><Errors><Error><Code>InsufficientInstanceCapacity</Code>
          <Message>No capacity for t3.micro</Message></Error></Errors></Response>"#,
      )
      .expect_at_least(1)
      .create();

    // The rollback delete is the assertion: without it the user pays nothing for the group, but it
    // lingers in their account with no record of where it came from.
    let group_deleted = server
      .mock("POST", "/ec2")
      .match_body(ec2_action("DeleteSecurityGroup"))
      .with_status(200)
      .with_body(r#"<DeleteSecurityGroupResponse><return>true</return></DeleteSecurityGroupResponse>"#)
      .expect_at_least(1)
      .create();

    let adapter = Ec2ProviderAdapter::with_base_url(&server.url());

    adapter
      .create_node(&aws_credential(), "Primary Node", "eu-west-2", "t3.micro", "#cloud-config\n")
      .expect_err("a failed launch must surface as an error");

    run_failure.assert();
    group_deleted.assert();
  }

  /// Destroy removes the instance and the security group that was created with it.
  #[test]
  fn ec2_delete_node_terminates_the_instance_and_removes_its_security_group() {
    let mut server = mockito::Server::new();

    let describe = mock_ec2(
      &mut server,
      "DescribeInstances",
      r#"<DescribeInstancesResponse><reservationSet><item><instancesSet><item>
        <instanceId>i-0abc123def4567890</instanceId>
        <instanceState><code>48</code><name>terminated</name></instanceState>
        <groupSet><item><groupId>sg-0ccc</groupId></item></groupSet>
      </item></instancesSet></item></reservationSet></DescribeInstancesResponse>"#,
    );
    let terminate = mock_ec2(
      &mut server,
      "TerminateInstances",
      r#"<TerminateInstancesResponse><return>true</return></TerminateInstancesResponse>"#,
    );
    let delete_group = mock_ec2(
      &mut server,
      "DeleteSecurityGroup",
      r#"<DeleteSecurityGroupResponse><return>true</return></DeleteSecurityGroupResponse>"#,
    );

    let adapter = Ec2ProviderAdapter::with_base_url(&server.url());

    adapter
      .delete_node(&aws_credential(), "eu-west-2/i-0abc123def4567890")
      .expect("destroy should succeed");

    describe.assert();
    terminate.assert();
    delete_group.assert();
  }

  /// A security group that cannot be removed must be reported.
  ///
  /// Returning success here is the habit AGENTS.md §8 forbids: the instance is gone, so the app
  /// looks clean, while a resource the user did not ask for stays in their account.
  #[test]
  fn ec2_delete_node_reports_a_security_group_it_could_not_remove() {
    let mut server = mockito::Server::new();

    let _describe = mock_ec2(
      &mut server,
      "DescribeInstances",
      r#"<DescribeInstancesResponse><reservationSet><item><instancesSet><item>
        <instanceId>i-0abc123def4567890</instanceId>
        <instanceState><code>48</code><name>terminated</name></instanceState>
        <groupSet><item><groupId>sg-0ccc</groupId></item></groupSet>
      </item></instancesSet></item></reservationSet></DescribeInstancesResponse>"#,
    );
    let _terminate = mock_ec2(
      &mut server,
      "TerminateInstances",
      r#"<TerminateInstancesResponse><return>true</return></TerminateInstancesResponse>"#,
    );
    let _blocked = server
      .mock("POST", "/ec2")
      .match_body(ec2_action("DeleteSecurityGroup"))
      .with_status(400)
      .with_body(
        r#"<Response><Errors><Error><Code>DependencyViolation</Code>
          <Message>resource sg-0ccc has a dependent object</Message></Error></Errors></Response>"#,
      )
      .expect_at_least(1)
      .create();

    let adapter = Ec2ProviderAdapter::with_base_url(&server.url());

    let error = adapter
      .delete_node(&aws_credential(), "eu-west-2/i-0abc123def4567890")
      .expect_err("an undeletable security group must be reported, not swallowed");

    assert!(
      error.to_string().contains("sg-0ccc"),
      "the error should name the group left behind: {error}"
    );
  }

  /// A terminated instance lingers in `DescribeInstances` for about an hour. Reporting it as
  /// existing would leave a destroyed node looking alive on the fleet list.
  #[test]
  fn ec2_node_exists_is_false_once_the_instance_is_terminated() {
    let mut server = mockito::Server::new();
    let _describe = mock_ec2(
      &mut server,
      "DescribeInstances",
      r#"<DescribeInstancesResponse><reservationSet><item><instancesSet><item>
        <instanceId>i-0abc123def4567890</instanceId>
        <instanceState><code>48</code><name>terminated</name></instanceState>
      </item></instancesSet></item></reservationSet></DescribeInstancesResponse>"#,
    );

    let adapter = Ec2ProviderAdapter::with_base_url(&server.url());
    let credential = aws_credential();

    assert!(!adapter
      .node_exists(&credential, "eu-west-2/i-0abc123def4567890")
      .expect("existence check should succeed"));
    assert_eq!(
      adapter
        .node_status(&credential, "eu-west-2/i-0abc123def4567890")
        .expect("status check should succeed"),
      ProviderNodeStatus::Deleted
    );
  }

  /// A rejected key must be reported as a credential problem, so the user is told to re-enter it.
  #[test]
  fn ec2_validate_token_surfaces_an_aws_auth_failure() {
    let mut server = mockito::Server::new();
    let _sts = server
      .mock("POST", "/sts")
      .with_status(403)
      .with_body(
        r#"<ErrorResponse><Error><Code>InvalidClientTokenId</Code>
          <Message>The security token included in the request is invalid.</Message></Error></ErrorResponse>"#,
      )
      .expect_at_least(1)
      .create();

    let adapter = Ec2ProviderAdapter::with_base_url(&server.url());

    let error = adapter
      .validate_token(&aws_credential())
      .expect_err("an invalid key must fail validation");

    assert!(
      logging::describe_error_chain(&error).contains("InvalidClientTokenId"),
      "the AWS reason should reach the user: {}",
      logging::describe_error_chain(&error)
    );
  }

  /// A credential missing a field names it, rather than failing later as an opaque 403.
  #[test]
  fn ec2_rejects_a_credential_that_is_missing_its_secret() {
    let adapter = Ec2ProviderAdapter::new();
    let partial = ProviderCredential::from_fields([(
      "access_key_id".to_string(),
      "AKIAEXAMPLENOTREAL".to_string(),
    )])
    .to_stored_value();

    let error = adapter
      .validate_token(&partial)
      .expect_err("an incomplete credential must be rejected");

    assert!(error.to_string().contains("secret_access_key"), "got {error}");
  }

  /// A DigitalOcean node id reaching the EC2 adapter is a caller error, not a panic.
  #[test]
  fn ec2_rejects_a_node_reference_that_is_not_its_own() {
    let adapter = Ec2ProviderAdapter::new();

    let error = adapter
      .node_status(&aws_credential(), "12345")
      .expect_err("a numeric droplet id is not an EC2 node reference");

    assert!(matches!(error, ProviderAdapterError::Unsupported { .. }), "got {error:?}");
  }

  /// EC2 caps user-data at 16 KiB before base64. The rendered cloud-init is several KB, so this
  /// guards against an edit to the shared bootstrap script silently breaking provisioning.
  #[test]
  fn the_rendered_cloud_init_fits_inside_the_ec2_user_data_limit() {
    use crate::backend::bootstrap::{
      allocate_peer_addresses, Bootstrap, BootstrapParams, BootstrapPeer, DEFAULT_PEER_SLOTS,
      DEFAULT_SERVER_INTERFACE_CIDR,
    };
    use crate::backend::crypto::keypair::{
      generate_client_keypair, generate_preshared_key, generate_server_keypair,
    };

    let server = generate_server_keypair().expect("server keypair should generate");

    // A full slot complement, because that is what production renders and the whole point of
    // this test is the size of a real payload.
    let peers = allocate_peer_addresses(DEFAULT_SERVER_INTERFACE_CIDR, DEFAULT_PEER_SLOTS)
      .expect("the default subnet has room for a full slot complement")
      .into_iter()
      .map(|address_cidr| BootstrapPeer {
        // A distinct key per slot, as provisioning generates. Reusing one trips the duplicate
        // check, and would understate the payload size this test exists to bound.
        public_key: generate_client_keypair()
          .expect("client keypair should generate")
          .public_key,
        address_cidr,
        preshared_key: generate_preshared_key().expect("preshared key should generate"),
      })
      .collect();

    let artifact = Bootstrap
      .render_cloud_init(
        &BootstrapParams {
          peers,
          ..BootstrapParams::default()
        },
        &server.private_key,
        &server.public_key,
      )
      .expect("cloud-init should render");

    assert!(
      artifact.init_payload.as_bytes().len() <= EC2_USER_DATA_LIMIT_BYTES,
      "cloud-init is {} bytes, over EC2's {EC2_USER_DATA_LIMIT_BYTES}-byte user-data limit",
      artifact.init_payload.as_bytes().len()
    );
  }

  #[test]
  fn base64_encoding_matches_known_vectors() {
    // Padding is where a hand-written encoder usually goes wrong, so all three lengths are covered.
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    assert_eq!(base64_encode(b"#cloud-config\n"), "I2Nsb3VkLWNvbmZpZwo=");
  }

  /// AWS tag values allow far more than a hostname does, so the user's label is preserved rather
  /// than mangled the way a DigitalOcean droplet name has to be.
  #[test]
  fn ec2_tag_values_keep_spaces_and_reject_only_what_aws_does() {
    assert_eq!(sanitize_ec2_tag_value("  Primary Node  "), "Primary Node");
    assert_eq!(sanitize_ec2_tag_value("tab\there"), "tabhere");
    assert_eq!(sanitize_ec2_tag_value("   "), "blink-node");
    assert_eq!(sanitize_ec2_tag_value(""), "blink-node");
  }

  /// Two nodes the user called the same thing must not collide on the security group name.
  #[test]
  fn ec2_security_group_names_are_unique_per_call() {
    let first = ec2_security_group_name("Primary Node");
    let second = ec2_security_group_name("Primary Node");

    assert_ne!(first, second);
    assert!(first.starts_with("blink-Primary-Node-"), "got {first}");
    assert!(
      first.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
      "got {first}"
    );
  }

  fn single_field_credential(token: &str) -> ProviderCredential {
    ProviderCredential::from_fields([(
      LEGACY_SINGLE_CREDENTIAL_KEY.to_string(),
      token.to_string(),
    )])
  }

  /// A credential saved before credentials had named fields is a bare token with no JSON around it.
  ///
  /// Every existing install has one. If this stopped parsing, the stored value would be
  /// unreadable — the provider would look configured while every API call failed.
  #[test]
  fn a_legacy_bare_token_still_parses_as_the_single_credential_field() {
    let parsed = ProviderCredential::parse("dop_v1_legacy_bare_token");

    assert_eq!(
      parsed.single().expect("a bare token must resolve"),
      "dop_v1_legacy_bare_token"
    );
  }

  /// A multi-field credential round-trips through storage without losing a field.
  #[test]
  fn a_multi_field_credential_round_trips_through_its_stored_form() {
    let credential = ProviderCredential::from_fields([
      ("access_key_id".to_string(), "AKIAEXAMPLE".to_string()),
      ("secret_access_key".to_string(), "wJalrEXAMPLEKEY".to_string()),
    ]);

    let parsed = ProviderCredential::parse(&credential.to_stored_value());

    assert_eq!(parsed.field("access_key_id"), Some("AKIAEXAMPLE"));
    assert_eq!(parsed.field("secret_access_key"), Some("wJalrEXAMPLEKEY"));
    assert_eq!(parsed, credential);
  }

  /// A token that merely happens to start with `{` must not be mistaken for a field set.
  #[test]
  fn a_bare_token_that_is_not_valid_json_is_treated_as_a_single_field() {
    let parsed = ProviderCredential::parse("{not-json-just-an-odd-token");

    assert_eq!(
      parsed.single().expect("an unparseable value falls back to the single field"),
      "{not-json-just-an-odd-token"
    );
  }

  /// A missing field names itself, rather than failing later as an opaque authentication error.
  #[test]
  fn a_missing_credential_field_is_reported_by_name() {
    let credential = ProviderCredential::from_fields([(
      "access_key_id".to_string(),
      "AKIAEXAMPLE".to_string(),
    )]);

    let error = credential
      .require("secret_access_key")
      .expect_err("a missing field must be an error");

    assert!(error.to_string().contains("secret_access_key"));
  }

  #[derive(Clone)]
  struct TestProviderAdapter;

  impl ProviderAdapter for TestProviderAdapter {
    fn provider_name(&self) -> &'static str {
      "test-provider"
    }

    fn credential_fields(&self) -> Vec<CredentialField> {
      vec![CredentialField {
        key: LEGACY_SINGLE_CREDENTIAL_KEY.to_string(),
        label: "API token".to_string(),
        placeholder: "test_...".to_string(),
        secret: true,
        help: None,
        token_page_url: None,
        scope_hint: None,
      }]
    }

    fn validate_token(&self, _token: &str) -> Result<ProviderIdentity, ProviderAdapterError> {
      Ok(ProviderIdentity {
        account_id: "test-account".to_string(),
        label: Some("test@example.com".to_string()),
        status: None,
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

    fn init_delivery(&self) -> InitDelivery {
      InitDelivery::CloudInit
    }

    fn create_node(
      &self,
      _token: &str,
      _node_name: &str,
      _region: &str,
      _size_slug: &str,
      _init_payload: &str,
    ) -> Result<ProvisionedNode, ProviderAdapterError> {
      Ok(ProvisionedNode {
        node_id: "42".to_string(),
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

    fn node_status(
      &self,
      _token: &str,
      _node_id: &str,
    ) -> Result<ProviderNodeStatus, ProviderAdapterError> {
      Ok(ProviderNodeStatus::Active)
    }

    fn node_exists(&self, _token: &str, _node_id: &str) -> Result<bool, ProviderAdapterError> {
      Ok(true)
    }

    fn delete_node(&self, _token: &str, _node_id: &str) -> Result<(), ProviderAdapterError> {
      Ok(())
    }
  }

  /// Both builtin slugs resolve to a node provider, and an unregistered one resolves to none —
  /// which is what turns an unknown slug into an explicit capability error instead of a panic.
  #[test]
  fn node_provider_resolves_for_registered_slugs_only() {
    let registry = ProviderRegistry::new();

    assert!(registry.node_provider_for_slug("docker").is_some());
    assert!(registry.node_provider_for_slug("digitalocean").is_some());
    assert!(registry.node_provider_for_slug("aws-ec2").is_some());
    assert!(registry.node_provider_for_slug("linode").is_some());
    assert!(registry.node_provider_for_slug("gce").is_some());
    assert!(registry.node_provider_for_slug("openstack").is_some());
    assert!(registry.node_provider_for_slug("nimbus-cloud").is_none());
  }

  #[test]
  fn provider_flow_persists_token_reference_not_plaintext() {
    let db = Database::new(test_db_path("token_ref_only"));
    db.run_migrations().expect("migrations should succeed");

    let registry = ProviderRegistry::new();
    let secure_storage = SecureStorage::dev_mock();
    let plaintext_token = "dop_v1_sensitive_plaintext_token";

    let provider = registry
      .upsert_provider_credential_for_slug(
        &db,
        &secure_storage,
        "provider-1",
        "DigitalOcean",
        "digitalocean",
        &single_field_credential(plaintext_token),
      )
      .expect("provider flow should succeed");

    let token_ref = provider.token_ref.expect("provider should store token_ref");
    assert_ne!(token_ref, plaintext_token);

    let recovered = secure_storage
      .get_provider_credential(&token_ref)
      .expect("token retrieval should succeed")
      .expect("token should exist in mock secret storage");
    assert_eq!(
      ProviderCredential::parse(&recovered)
        .single()
        .expect("the stored credential must carry the token"),
      plaintext_token
    );

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
      .upsert_provider_credential_for_slug(
        &db,
        &secure_storage,
        "provider-1",
        "DigitalOcean",
        "digitalocean",
        &single_field_credential("dop_v1_stale_test_token"),
      )
      .expect("provider create should succeed");

    let token_ref = provider
      .token_ref
      .clone()
      .expect("token ref should be present");

    secure_storage
      .delete_provider_credential(&token_ref)
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
      .upsert_provider_credential_for_slug(
        &db,
        &secure_storage,
        provider_id,
        "Test Provider",
        "test-provider",
        &single_field_credential("test-token"),
      )
      .expect("provider create should succeed");

    let request = NodeInitRequest {
      node_name: "test-node".to_string(),
      region: "test-region".to_string(),
      size_slug: "test-size".to_string(),
      init_payload: "#!/bin/sh\necho hi".to_string(),
    };

    let result = registry
      .init_provider_node(&db, &secure_storage, provider_id, &request)
      .expect("node init should succeed through NodeProvider interface");

    assert_eq!(result.provisioned.node_id, "42");
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
      .upsert_provider_credential_for_slug(
        &db,
        &secure_storage,
        "provider-1",
        "DigitalOcean",
        "digitalocean",
        &single_field_credential(token),
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
      .get_provider_credential(&token_ref)
      .expect("token lookup should succeed")
      .expect("the credential should be readable from Credential Manager");
    assert_eq!(
      ProviderCredential::parse(&recovered)
        .single()
        .expect("the stored credential must carry the token"),
      token
    );

    secure_storage
      .delete_provider_credential(&token_ref)
      .expect("cleanup delete should succeed");
  }
}
