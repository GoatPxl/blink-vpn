//! Google Compute Engine API client.
//!
//! Two things make this adapter differ from the DigitalOcean/Linode shape:
//!
//! 1. **Authentication is a token exchange, not a header.** A service-account key signs a JWT that
//!    is traded for a one-hour access token. That token is cached on the adapter, because minting
//!    one per API call would triple every request.
//! 2. **Create is asynchronous.** `instances.insert` returns an `Operation` rather than the
//!    instance; the resource is polled separately for `RUNNING` and its external IP.

use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::retry::RetryPolicy;

pub const DEFAULT_GCE_ZONE: &str = "us-central1-a";
pub const DEFAULT_GCE_MACHINE_TYPE: &str = "e2-micro";

/// Boot image, referenced by **family** rather than by a pinned image name.
///
/// A family URL always resolves to the newest image in it, so this cannot rot the way a specific
/// image name does. Ubuntu images ship cloud-init and read the `user-data` metadata key.
pub const DEFAULT_GCE_IMAGE_FAMILY: &str =
  "projects/ubuntu-os-cloud/global/images/family/ubuntu-2404-lts-amd64";

/// Scope requested for the access token. `compute` is the narrowest scope that can create and
/// delete instances and firewall rules; `cloud-platform` would grant far more than this app needs.
pub const GCE_SCOPE: &str = "https://www.googleapis.com/auth/compute";

pub const GCE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// GCE caps a single metadata value at 256 KiB, and takes the payload **verbatim** — unlike EC2
/// and Linode, there is no base64 step, so the limit applies to the rendered script as-is.
pub const GCE_USER_DATA_LIMIT_BYTES: usize = 256 * 1024;

/// Tunnel MTU for client configs on this provider.
///
/// GCE instance NICs run at 1460 rather than 1500, so WireGuard's default tunnel MTU of 1420 is
/// 40 bytes too large: packets at the top of that range fragment or vanish, which presents as a
/// tunnel that connects and then stalls on large transfers. 1420 - 40 = 1380.
pub const GCE_CLIENT_MTU: u16 = 1380;

/// Refresh a cached token this long before it actually expires, so a call that starts near the
/// boundary does not race it.
const TOKEN_EXPIRY_SKEW: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct GceConfig {
  pub zone: &'static str,
  pub machine_type: &'static str,
  pub image_family: &'static str,
  pub boot_disk_gb: u64,
  pub poll_interval: Duration,
  pub poll_max_attempts: usize,
  pub request_timeout: Duration,
  pub instance_ready_timeout: Duration,
}

impl GceConfig {
  pub fn defaults() -> Self {
    Self {
      zone: DEFAULT_GCE_ZONE,
      machine_type: DEFAULT_GCE_MACHINE_TYPE,
      image_family: DEFAULT_GCE_IMAGE_FAMILY,
      boot_disk_gb: 10,
      poll_interval: Duration::from_secs(4),
      poll_max_attempts: 60,
      request_timeout: Duration::from_secs(20),
      instance_ready_timeout: Duration::from_secs(240),
    }
  }
}

/// A service-account key file, as downloaded from the Google Cloud console.
///
/// The user pastes one artifact and everything else is read out of it — asking separately for a
/// project id that is already in the file invites a mismatch that surfaces as a 404 on a project
/// the key cannot see.
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceAccountKey {
  pub project_id: String,
  pub client_email: String,
  pub private_key: String,
  #[serde(default)]
  pub private_key_id: String,
}

impl ServiceAccountKey {
  pub fn parse(raw: &str) -> Result<Self, GceError> {
    let key: Self = serde_json::from_str(raw.trim()).map_err(|source| GceError::Credential {
      detail: format!("The service account key is not valid JSON: {source}"),
    })?;

    for (value, field) in [
      (&key.project_id, "project_id"),
      (&key.client_email, "client_email"),
      (&key.private_key, "private_key"),
    ] {
      if value.trim().is_empty() {
        return Err(GceError::Credential {
          detail: format!(
            "The service account key is missing '{field}'. Paste the whole JSON key file, not an excerpt."
          ),
        });
      }
    }

    Ok(key)
  }

  /// Identity of this exact credential, so a re-saved key invalidates a cached token immediately
  /// rather than after the old one expires.
  fn fingerprint(&self) -> String {
    format!("{}:{}", self.client_email, self.private_key_id)
  }
}

#[derive(Debug, Clone)]
struct CachedAccessToken {
  credential_fingerprint: String,
  access_token: String,
  expires_at: Instant,
}

#[derive(Serialize)]
struct JwtClaims<'a> {
  iss: &'a str,
  scope: &'a str,
  aud: &'a str,
  iat: u64,
  exp: u64,
}

pub struct GceApiClient {
  compute_base_url: String,
  token_url: String,
  http: Client,
  retry_policy: RetryPolicy,
  token_cache: Mutex<Option<CachedAccessToken>>,
}

impl GceApiClient {
  pub fn new_with_timeout(
    compute_base_url: &str,
    token_url: &str,
    retry_policy: RetryPolicy,
    request_timeout: Duration,
  ) -> Result<Self, GceError> {
    let http = Client::builder()
      .timeout(request_timeout)
      .build()
      .map_err(|source| GceError::Transport {
        detail: "Unable to initialize HTTP client for Google Compute Engine requests.".to_string(),
        source,
      })?;

    Ok(Self {
      compute_base_url: compute_base_url.trim_end_matches('/').to_string(),
      token_url: token_url.to_string(),
      http,
      retry_policy,
      token_cache: Mutex::new(None),
    })
  }

  /// A valid access token, minted if the cache has none for this credential.
  ///
  /// The lock is held across the mint. Commands run on the blocking pool, so at worst a status
  /// poll waits on a user action's token — cheaper than the alternative, which is two threads
  /// minting separate tokens and each overwriting the other's cache entry.
  fn access_token(&self, key: &ServiceAccountKey) -> Result<String, GceError> {
    let fingerprint = key.fingerprint();
    let mut cache = self.token_cache.lock().map_err(|_| GceError::Credential {
      detail: "The Google access-token cache was poisoned by an earlier failure.".to_string(),
    })?;

    if let Some(cached) = cache.as_ref() {
      if cached.credential_fingerprint == fingerprint
        && cached.expires_at > Instant::now() + TOKEN_EXPIRY_SKEW
      {
        return Ok(cached.access_token.clone());
      }
    }

    let minted = self.mint_access_token(key)?;
    *cache = Some(CachedAccessToken {
      credential_fingerprint: fingerprint,
      access_token: minted.access_token.clone(),
      expires_at: Instant::now() + Duration::from_secs(minted.expires_in),
    });

    Ok(minted.access_token)
  }

  fn mint_access_token(&self, key: &ServiceAccountKey) -> Result<MintedToken, GceError> {
    let issued_at = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map_err(|_| GceError::Credential {
        detail: "The system clock is before the Unix epoch, so a token cannot be signed."
          .to_string(),
      })?
      .as_secs();

    let mut header = Header::new(Algorithm::RS256);
    if !key.private_key_id.trim().is_empty() {
      header.kid = Some(key.private_key_id.clone());
    }

    let claims = JwtClaims {
      iss: &key.client_email,
      scope: GCE_SCOPE,
      aud: &self.token_url,
      iat: issued_at,
      exp: issued_at + 3600,
    };

    let encoding_key =
      EncodingKey::from_rsa_pem(key.private_key.as_bytes()).map_err(|source| GceError::Credential {
        detail: format!(
          "The service account key's private_key could not be read as a PEM RSA key: {source}"
        ),
      })?;

    let assertion = jsonwebtoken::encode(&header, &claims, &encoding_key).map_err(|source| {
      GceError::Credential {
        detail: format!("Could not sign the Google authentication assertion: {source}"),
      }
    })?;

    let response = self.send_with_retry(|| {
      self
        .http
        .post(&self.token_url)
        .form(&[
          ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
          ("assertion", assertion.as_str()),
        ])
        .send()
    })?;

    let body = self.expect_status("mint_access_token", response, &[StatusCode::OK])?;
    parse_json("mint_access_token", "Google token response could not be parsed.", &body)
  }

  /// Confirm the service account can read the project it names.
  ///
  /// Nothing in the response body is needed — reaching a 200 is the whole result, and it proves
  /// three things at once: the key signs, the Compute Engine API is enabled, and the account can
  /// see this project.
  pub fn get_project(&self, key: &ServiceAccountKey) -> Result<(), GceError> {
    let token = self.access_token(key)?;
    let response = self.send_with_retry(|| {
      self
        .http
        .get(format!(
          "{}/compute/v1/projects/{}",
          self.compute_base_url, key.project_id
        ))
        .bearer_auth(&token)
        .send()
    })?;

    self.expect_status("get_project", response, &[StatusCode::OK])?;
    Ok(())
  }

  pub fn list_zones(&self, key: &ServiceAccountKey) -> Result<Vec<GceZone>, GceError> {
    let token = self.access_token(key)?;
    let response = self.send_with_retry(|| {
      self
        .http
        .get(format!(
          "{}/compute/v1/projects/{}/zones",
          self.compute_base_url, key.project_id
        ))
        .bearer_auth(&token)
        .send()
    })?;

    let body = self.expect_status("list_zones", response, &[StatusCode::OK])?;
    let envelope: ItemsEnvelope<GceZone> =
      parse_json("list_zones", "Google zone list could not be parsed.", &body)?;
    Ok(envelope.items)
  }

  pub fn list_machine_types(
    &self,
    key: &ServiceAccountKey,
    zone: &str,
  ) -> Result<Vec<GceMachineType>, GceError> {
    let token = self.access_token(key)?;
    let response = self.send_with_retry(|| {
      self
        .http
        .get(format!(
          "{}/compute/v1/projects/{}/zones/{zone}/machineTypes",
          self.compute_base_url, key.project_id
        ))
        .bearer_auth(&token)
        .send()
    })?;

    let body = self.expect_status("list_machine_types", response, &[StatusCode::OK])?;
    let envelope: ItemsEnvelope<GceMachineType> = parse_json(
      "list_machine_types",
      "Google machine type list could not be parsed.",
      &body,
    )?;
    Ok(envelope.items)
  }

  /// Whether the project's `default` VPC network exists.
  ///
  /// A project whose default network was deleted cannot launch into one, and finding that out from
  /// a failed insert produces a far worse message than checking first.
  pub fn default_network_exists(&self, key: &ServiceAccountKey) -> Result<bool, GceError> {
    let token = self.access_token(key)?;
    let response = self.send_with_retry(|| {
      self
        .http
        .get(format!(
          "{}/compute/v1/projects/{}/global/networks/default",
          self.compute_base_url, key.project_id
        ))
        .bearer_auth(&token)
        .send()
    })?;

    match self.expect_status("default_network_exists", response, &[StatusCode::OK]) {
      Ok(_) => Ok(true),
      Err(GceError::NotFound { .. }) => Ok(false),
      Err(error) => Err(error),
    }
  }

  /// Open the WireGuard port to instances carrying `network_tag`.
  ///
  /// The default network's own rules do not pass UDP 51820, so without this the instance boots,
  /// bootstraps, and refuses every handshake. Scoped by tag rather than to the whole network so
  /// teardown can remove exactly this node's rule.
  pub fn create_firewall_rule(
    &self,
    key: &ServiceAccountKey,
    name: &str,
    network_tag: &str,
    port: u16,
  ) -> Result<(), GceError> {
    let token = self.access_token(key)?;
    let body = serde_json::json!({
      "name": name,
      "network": format!("projects/{}/global/networks/default", key.project_id),
      "direction": "INGRESS",
      "priority": 1000,
      "sourceRanges": ["0.0.0.0/0"],
      "targetTags": [network_tag],
      "allowed": [{ "IPProtocol": "udp", "ports": [port.to_string()] }],
      "description": "GoatStare WireGuard ingress",
    });

    let response = self.send_with_retry(|| {
      self
        .http
        .post(format!(
          "{}/compute/v1/projects/{}/global/firewalls",
          self.compute_base_url, key.project_id
        ))
        .bearer_auth(&token)
        .json(&body)
        .send()
    })?;

    // The returned Operation is not polled: the rule is usable as soon as the call is accepted,
    // and the instance it protects takes far longer to boot than the rule takes to propagate.
    self.expect_status("create_firewall_rule", response, &[StatusCode::OK])?;
    Ok(())
  }

  pub fn delete_firewall_rule(&self, key: &ServiceAccountKey, name: &str) -> Result<(), GceError> {
    let token = self.access_token(key)?;
    let response = self.send_with_retry(|| {
      self
        .http
        .delete(format!(
          "{}/compute/v1/projects/{}/global/firewalls/{name}",
          self.compute_base_url, key.project_id
        ))
        .bearer_auth(&token)
        .send()
    })?;

    match self.expect_status("delete_firewall_rule", response, &[StatusCode::OK]) {
      Ok(_) => Ok(()),
      // Idempotent: a rule already gone is the state we wanted.
      Err(GceError::NotFound { .. }) => Ok(()),
      Err(error) => Err(error),
    }
  }

  #[allow(clippy::too_many_arguments)]
  pub fn insert_instance(
    &self,
    key: &ServiceAccountKey,
    zone: &str,
    name: &str,
    machine_type: &str,
    image_family: &str,
    boot_disk_gb: u64,
    network_tag: &str,
    user_data: &str,
  ) -> Result<(), GceError> {
    let token = self.access_token(key)?;
    let body = serde_json::json!({
      "name": name,
      "machineType": format!("zones/{zone}/machineTypes/{machine_type}"),
      "disks": [{
        "boot": true,
        // The disk is billed separately from the instance and does not disappear with it unless
        // this is set — the classic way a "deleted" GCE node keeps costing money.
        "autoDelete": true,
        "initializeParams": {
          "sourceImage": image_family,
          "diskSizeGb": boot_disk_gb.to_string(),
        },
      }],
      "networkInterfaces": [{
        "network": format!("projects/{}/global/networks/default", key.project_id),
        // Without an access config the instance has no external address at all and nothing can
        // reach it. ONE_TO_ONE_NAT with no reserved address means an ephemeral public IPv4.
        "accessConfigs": [{ "type": "ONE_TO_ONE_NAT", "name": "External NAT" }],
      }],
      "tags": { "items": [network_tag] },
      "labels": { "goatstare": "managed" },
      "metadata": {
        // Ubuntu images run cloud-init, which reads this exact key. `startup-script` would also
        // execute but is not cloud-init, and the payload is a #cloud-config document.
        "items": [{ "key": "user-data", "value": user_data }],
      },
    });

    let response = self.send_with_retry(|| {
      self
        .http
        .post(format!(
          "{}/compute/v1/projects/{}/zones/{zone}/instances",
          self.compute_base_url, key.project_id
        ))
        .bearer_auth(&token)
        .json(&body)
        .send()
    })?;

    // The returned Operation is not polled. `wait_for_instance_running` polls the *instance*
    // instead, which is the state that actually matters — an Operation reaching DONE only means
    // the instance was created, not that it is running with an address.
    self.expect_status("insert_instance", response, &[StatusCode::OK])?;
    Ok(())
  }

  pub fn get_instance(
    &self,
    key: &ServiceAccountKey,
    zone: &str,
    name: &str,
  ) -> Result<GceInstance, GceError> {
    let token = self.access_token(key)?;
    let response = self.send_with_retry(|| {
      self
        .http
        .get(format!(
          "{}/compute/v1/projects/{}/zones/{zone}/instances/{name}",
          self.compute_base_url, key.project_id
        ))
        .bearer_auth(&token)
        .send()
    })?;

    let body = self.expect_status("get_instance", response, &[StatusCode::OK])?;
    parse_json("get_instance", "Google instance response could not be parsed.", &body)
  }

  pub fn delete_instance(
    &self,
    key: &ServiceAccountKey,
    zone: &str,
    name: &str,
  ) -> Result<(), GceError> {
    let token = self.access_token(key)?;
    let response = self.send_with_retry(|| {
      self
        .http
        .delete(format!(
          "{}/compute/v1/projects/{}/zones/{zone}/instances/{name}",
          self.compute_base_url, key.project_id
        ))
        .bearer_auth(&token)
        .send()
    })?;

    match self.expect_status("delete_instance", response, &[StatusCode::OK]) {
      Ok(_) => Ok(()),
      Err(GceError::NotFound { .. }) => Ok(()),
      Err(error) => Err(error),
    }
  }

  pub fn wait_for_instance_running(
    &self,
    key: &ServiceAccountKey,
    zone: &str,
    name: &str,
    poll_interval: Duration,
    max_attempts: usize,
    timeout: Duration,
  ) -> Result<GceInstance, GceError> {
    let started = Instant::now();

    for attempt in 0..max_attempts {
      if started.elapsed() > timeout {
        return Err(GceError::Timeout {
          action: "wait_for_instance_running",
          detail: "Google instance did not reach RUNNING before the configured timeout. Reconcile or retry the provisioning flow.".to_string(),
        });
      }

      match self.get_instance(key, zone, name) {
        Ok(instance) => {
          // RUNNING alone is not enough: the external address is assigned as networking settles,
          // and reporting an endpoint before it exists hands out an unusable config.
          if instance.status == "RUNNING" && instance.external_ipv4().is_some() {
            return Ok(instance);
          }
          if instance.status == "TERMINATED" || instance.status == "SUSPENDED" {
            return Err(GceError::UnexpectedStatus {
              action: "wait_for_instance_running",
              status: 0,
              detail: format!("Google instance settled in '{}' rather than RUNNING.", instance.status),
            });
          }
        }
        // The instance is not visible for a moment after insert returns.
        Err(GceError::NotFound { .. }) => {}
        Err(error) => return Err(error),
      }

      if attempt + 1 < max_attempts {
        thread::sleep(poll_interval);
      }
    }

    Err(GceError::Timeout {
      action: "wait_for_instance_running",
      detail: "Google instance did not reach RUNNING before the configured poll limit. Reconcile or retry the provisioning flow.".to_string(),
    })
  }

  /// Send with retries, reading each response body as it arrives.
  ///
  /// The body is buffered rather than streamed because Google signals rate limiting *inside* a 403
  /// body — deciding whether to retry means reading it, and a `Response` cannot be read twice.
  /// Every payload this client handles is a small JSON document, so buffering costs nothing.
  fn send_with_retry<F>(&self, mut operation: F) -> Result<RawResponse, GceError>
  where
    F: FnMut() -> Result<reqwest::blocking::Response, reqwest::Error>,
  {
    let mut last_transport_error: Option<reqwest::Error> = None;

    for attempt in 0..=self.retry_policy.max_retries {
      match operation() {
        Ok(response) => {
          let status = response.status();
          let body = response.text().unwrap_or_else(|_| "<unavailable>".to_string());
          let retries_left = attempt < self.retry_policy.max_retries;

          if retries_left && self.retry_policy.should_retry_status(status.as_u16()) {
            thread::sleep(self.retry_policy.backoff_for_attempt(attempt));
            continue;
          }

          // Google reports rate limiting as 403 with a reason in the body — the same shape trap
          // AWS sets with throttling-in-a-400. Treating it as an auth failure would tell the user
          // their key is bad when the project is merely busy.
          if retries_left && status == StatusCode::FORBIDDEN && is_rate_limit_body(&body) {
            thread::sleep(self.retry_policy.backoff_for_attempt(attempt));
            continue;
          }

          return Ok(RawResponse { status, body });
        }
        Err(error) => {
          if attempt < self.retry_policy.max_retries && is_retryable_transport_error(&error) {
            last_transport_error = Some(error);
            thread::sleep(self.retry_policy.backoff_for_attempt(attempt));
            continue;
          }

          return Err(GceError::Transport {
            detail: "Google Compute Engine request failed at transport layer. Check connectivity and retry."
              .to_string(),
            source: error,
          });
        }
      }
    }

    if let Some(source) = last_transport_error {
      return Err(GceError::Transport {
        detail: "Google Compute Engine request retries were exhausted by transient transport failures."
          .to_string(),
        source,
      });
    }

    Err(GceError::Timeout {
      action: "send_with_retry",
      detail: "Retry policy exhausted before obtaining a stable response.".to_string(),
    })
  }

  /// The response body, if the status is one the caller expected.
  fn expect_status(
    &self,
    action: &'static str,
    response: RawResponse,
    allowed: &[StatusCode],
  ) -> Result<String, GceError> {
    if allowed.contains(&response.status) {
      return Ok(response.body);
    }

    if response.status == StatusCode::NOT_FOUND {
      return Err(GceError::NotFound {
        action,
        detail: "Google Compute Engine resource was not found.".to_string(),
      });
    }

    if response.status == StatusCode::UNAUTHORIZED || response.status == StatusCode::FORBIDDEN {
      return Err(GceError::Unauthorized {
        action,
        detail: format!(
          "Google rejected the service account. Confirm the Compute Engine API is enabled and the account has the Compute Admin role. {}",
          describe_error_body(&response.body)
        ),
      });
    }

    Err(GceError::UnexpectedStatus {
      action,
      status: response.status.as_u16(),
      detail: format!(
        "Google Compute Engine API returned unexpected status. {}",
        describe_error_body(&response.body)
      ),
    })
  }
}

/// A response whose body has already been read. See `send_with_retry` for why.
struct RawResponse {
  status: StatusCode,
  body: String,
}

fn parse_json<T: serde::de::DeserializeOwned>(
  action: &'static str,
  detail: &str,
  body: &str,
) -> Result<T, GceError> {
  serde_json::from_str(body).map_err(|source| GceError::Decode {
    action,
    detail: detail.to_string(),
    source,
  })
}

/// Google's error envelope is `{"error":{"message":..., "errors":[{"reason":...}]}}`.
fn describe_error_body(body: &str) -> String {
  match serde_json::from_str::<GoogleErrorEnvelope>(body) {
    Ok(envelope) if !envelope.error.message.trim().is_empty() => envelope.error.message,
    _ => format!("Body snippet: {}", body.chars().take(256).collect::<String>()),
  }
}

fn is_rate_limit_body(body: &str) -> bool {
  serde_json::from_str::<GoogleErrorEnvelope>(body)
    .map(|envelope| {
      envelope.error.errors.iter().any(|error| {
        matches!(
          error.reason.as_str(),
          "rateLimitExceeded" | "userRateLimitExceeded" | "quotaExceeded" | "backendError"
        )
      })
    })
    .unwrap_or(false)
}

fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
  error.is_timeout() || error.is_connect() || error.is_request()
}

#[derive(Debug, Error)]
pub enum GceError {
  #[error("Google Compute Engine transport failure: {detail}")]
  Transport {
    detail: String,
    #[source]
    source: reqwest::Error,
  },
  #[error("Google service account credential problem: {detail}")]
  Credential { detail: String },
  #[error("Google rejected the service account during {action}: {detail}")]
  Unauthorized { action: &'static str, detail: String },
  #[error("Google Compute Engine response decode failure in {action}: {detail}")]
  Decode {
    action: &'static str,
    detail: String,
    #[source]
    source: serde_json::Error,
  },
  #[error("Google Compute Engine request {action} received HTTP {status}: {detail}")]
  UnexpectedStatus {
    action: &'static str,
    status: u16,
    detail: String,
  },
  #[error("Google Compute Engine resource not found during {action}: {detail}")]
  NotFound { action: &'static str, detail: String },
  #[error("Google Compute Engine operation timeout during {action}: {detail}")]
  Timeout { action: &'static str, detail: String },
}

#[derive(Debug, Clone, Deserialize)]
struct MintedToken {
  access_token: String,
  #[serde(default = "default_expires_in")]
  expires_in: u64,
}

fn default_expires_in() -> u64 {
  3600
}

#[derive(Debug, Clone, Deserialize)]
struct GoogleErrorEnvelope {
  #[serde(default)]
  error: GoogleError,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct GoogleError {
  #[serde(default)]
  message: String,
  #[serde(default)]
  errors: Vec<GoogleErrorDetail>,
}

#[derive(Debug, Clone, Deserialize)]
struct GoogleErrorDetail {
  #[serde(default)]
  reason: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ItemsEnvelope<T> {
  #[serde(default = "Vec::new")]
  items: Vec<T>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GceZone {
  pub name: String,
  #[serde(default)]
  pub status: String,
}

/// Only the name is read: the catalogue answers "can this zone launch this type", and the machine
/// specs shown to the user come from `GCE_PLAN_SPECS` rather than from here.
#[derive(Debug, Clone, Deserialize)]
pub struct GceMachineType {
  pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GceInstance {
  #[serde(default)]
  pub status: String,
  #[serde(default, rename = "networkInterfaces")]
  pub network_interfaces: Vec<GceNetworkInterface>,
}

impl GceInstance {
  /// The ephemeral public IPv4, which lives on the access config rather than on the NIC itself —
  /// reading `networkIP` instead yields the internal 10.x address and an unreachable endpoint.
  pub fn external_ipv4(&self) -> Option<String> {
    self
      .network_interfaces
      .iter()
      .flat_map(|interface| interface.access_configs.iter())
      .find_map(|config| {
        config
          .nat_ip
          .as_ref()
          .filter(|address| !address.trim().is_empty())
          .cloned()
      })
  }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GceNetworkInterface {
  #[serde(default, rename = "accessConfigs")]
  pub access_configs: Vec<GceAccessConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GceAccessConfig {
  #[serde(default, rename = "natIP")]
  pub nat_ip: Option<String>,
}

#[cfg(test)]
pub(crate) mod tests {
  use super::*;

  /// A syntactically valid RSA key, generated for tests only. It signs nothing real: the mock
  /// token endpoint accepts whatever assertion it is given, and no Google project has ever seen
  /// the matching public key. See `tests/fixtures/README.md`.
  pub(crate) const TEST_RSA_PEM: &str = include_str!("../../../tests/fixtures/test_rsa_key.pem");

  /// A service account key file in the shape Google issues, for the adapter tests next door.
  pub(crate) fn test_service_account_json() -> String {
    serde_json::json!({
      "type": "service_account",
      "project_id": "goatstare-test",
      "private_key_id": "test-key-id",
      "private_key": TEST_RSA_PEM,
      "client_email": "goatstare@goatstare-test.iam.gserviceaccount.com",
    })
    .to_string()
  }

  #[test]
  fn a_service_account_key_yields_its_project_and_identity() {
    let key = ServiceAccountKey::parse(&test_service_account_json()).expect("key should parse");

    assert_eq!(key.project_id, "goatstare-test");
    assert_eq!(key.client_email, "goatstare@goatstare-test.iam.gserviceaccount.com");
    assert!(key.private_key.contains("PRIVATE KEY"));
  }

  /// Asking the user for one artifact only works if a partial one is rejected by name — a key
  /// missing its project id would otherwise fail later as a 404 on a project that does exist.
  #[test]
  fn an_incomplete_service_account_key_is_reported_by_field() {
    let partial = serde_json::json!({
      "project_id": "goatstare-test",
      "client_email": "",
      "private_key": TEST_RSA_PEM,
    })
    .to_string();

    let error = ServiceAccountKey::parse(&partial).expect_err("an empty field must be rejected");
    assert!(error.to_string().contains("client_email"), "was: {error}");
  }

  #[test]
  fn a_key_that_is_not_json_is_rejected_as_a_credential_problem() {
    let error = ServiceAccountKey::parse("not json at all").expect_err("garbage must be rejected");
    assert!(matches!(error, GceError::Credential { .. }));
    assert!(error.to_string().contains("not valid JSON"), "was: {error}");
  }

  #[test]
  fn the_external_address_comes_from_the_access_config_not_the_interface() {
    let instance: GceInstance = serde_json::from_value(serde_json::json!({
      "name": "n",
      "status": "RUNNING",
      "networkInterfaces": [{
        "networkIP": "10.128.0.2",
        "accessConfigs": [{ "type": "ONE_TO_ONE_NAT", "natIP": "203.0.113.90" }],
      }],
    }))
    .expect("instance should parse");

    assert_eq!(instance.external_ipv4().as_deref(), Some("203.0.113.90"));
  }

  #[test]
  fn an_instance_without_an_access_config_has_no_external_address() {
    let instance: GceInstance = serde_json::from_value(serde_json::json!({
      "name": "n",
      "status": "RUNNING",
      "networkInterfaces": [{ "networkIP": "10.128.0.2", "accessConfigs": [] }],
    }))
    .expect("instance should parse");

    assert_eq!(instance.external_ipv4(), None);
  }

  /// Google answers rate limiting with 403, the same status it uses for a genuinely rejected
  /// credential. Telling those apart is what keeps a busy project from reading as a bad key.
  #[test]
  fn rate_limiting_is_recognised_inside_a_403_body() {
    let throttled = r#"{"error":{"code":403,"message":"Rate Limit Exceeded",
      "errors":[{"reason":"rateLimitExceeded"}]}}"#;
    let denied = r#"{"error":{"code":403,"message":"Permission denied",
      "errors":[{"reason":"forbidden"}]}}"#;

    assert!(is_rate_limit_body(throttled));
    assert!(!is_rate_limit_body(denied));
  }

  #[test]
  fn google_error_messages_are_surfaced_rather_than_raw_json() {
    let body = r#"{"error":{"code":404,"message":"The resource 'projects/x' was not found"}}"#;
    assert_eq!(
      describe_error_body(body),
      "The resource 'projects/x' was not found"
    );
  }
}
