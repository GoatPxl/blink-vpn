//! Linode (Akamai) API client.
//!
//! Shaped after `digitalocean.rs`, which is its closest cousin: a bearer token, JSON in and out,
//! and a single create call that takes the whole bootstrap payload. The differences that matter
//! are noted where they occur — the Metadata service requirement, the mandatory `root_pass`, and
//! Linode's `{"errors":[{...}]}` error envelope.

use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::retry::RetryPolicy;

pub const DEFAULT_LINODE_REGION: &str = "us-ord";
pub const DEFAULT_LINODE_TYPE: &str = "g6-nanode-1";

/// Deploy image. A cloud-init-capable image is required: without one the Metadata service is
/// never read and `user_data` is silently ignored, producing a server that boots fine and
/// hand-shakes with nobody.
pub const DEFAULT_LINODE_IMAGE: &str = "linode/ubuntu24.04";

/// The region capability that means "this region runs the Metadata service".
///
/// `user_data` is accepted by the create call everywhere, but only honoured where this is
/// present — the failure mode is a healthy-looking instance that never bootstrapped, so regions
/// without it are filtered out before the user can pick one.
pub const METADATA_CAPABILITY: &str = "Metadata";

/// Ceiling for `metadata.user_data`, measured on the **base64-encoded** form, which is what the
/// API documents and what the request actually carries.
pub const LINODE_USER_DATA_LIMIT_BYTES: usize = 65_535;

#[derive(Debug, Clone)]
pub struct LinodeConfig {
  pub region: &'static str,
  pub instance_type: &'static str,
  pub image: &'static str,
  pub poll_interval: Duration,
  pub poll_max_attempts: usize,
  pub request_timeout: Duration,
  pub instance_ready_timeout: Duration,
}

impl LinodeConfig {
  pub fn defaults() -> Self {
    Self {
      region: DEFAULT_LINODE_REGION,
      instance_type: DEFAULT_LINODE_TYPE,
      image: DEFAULT_LINODE_IMAGE,
      poll_interval: Duration::from_secs(4),
      poll_max_attempts: 60,
      request_timeout: Duration::from_secs(20),
      instance_ready_timeout: Duration::from_secs(240),
    }
  }
}

#[derive(Debug, Clone)]
pub struct LinodeApiClient {
  base_url: String,
  http: Client,
  retry_policy: RetryPolicy,
}

impl LinodeApiClient {
  pub fn new_with_timeout(
    base_url: &str,
    retry_policy: RetryPolicy,
    request_timeout: Duration,
  ) -> Result<Self, LinodeError> {
    let http = Client::builder()
      .timeout(request_timeout)
      .build()
      .map_err(|source| LinodeError::Transport {
        detail: "Unable to initialize HTTP client for Linode requests.".to_string(),
        source,
      })?;

    Ok(Self {
      base_url: base_url.trim_end_matches('/').to_string(),
      http,
      retry_policy,
    })
  }

  pub fn get_account(&self, token: &str) -> Result<LinodeAccount, LinodeError> {
    let response = self.send_with_retry(|| {
      self
        .http
        .get(format!("{}/v4/account", self.base_url))
        .bearer_auth(token)
        .send()
    })?;

    self.expect_status("get_account", response, &[StatusCode::OK])?
      .json::<LinodeAccount>()
      .map_err(|source| LinodeError::Decode {
        action: "get_account",
        detail: "Linode account response could not be parsed.".to_string(),
        source,
      })
  }

  pub fn list_regions(&self, token: &str) -> Result<Vec<LinodeRegion>, LinodeError> {
    self.list_paginated("list_regions", token, "/v4/regions")
  }

  pub fn list_types(&self, token: &str) -> Result<Vec<LinodeType>, LinodeError> {
    self.list_paginated("list_types", token, "/v4/linode/types")
  }

  /// Walk a paginated collection endpoint to completion.
  ///
  /// Linode paginates at 100 entries; the region and type catalogues are both past that, and a
  /// silently truncated list would hide regions and plans from the picker rather than fail.
  fn list_paginated<T: serde::de::DeserializeOwned>(
    &self,
    action: &'static str,
    token: &str,
    path: &str,
  ) -> Result<Vec<T>, LinodeError> {
    let mut collected = Vec::new();
    let mut page = 1_u32;

    loop {
      let response = self.send_with_retry(|| {
        self
          .http
          .get(format!("{}{path}", self.base_url))
          .query(&[("page", page.to_string()), ("page_size", "100".to_string())])
          .bearer_auth(token)
          .send()
      })?;

      let envelope = self
        .expect_status(action, response, &[StatusCode::OK])?
        .json::<PaginatedEnvelope<T>>()
        .map_err(|source| LinodeError::Decode {
          action,
          detail: "Linode paginated response could not be parsed.".to_string(),
          source,
        })?;

      collected.extend(envelope.data);

      if page >= envelope.pages {
        return Ok(collected);
      }
      page += 1;
    }
  }

  pub fn create_instance(
    &self,
    token: &str,
    request: &CreateLinodeRequest,
  ) -> Result<LinodeInstance, LinodeError> {
    let response = self.send_with_retry(|| {
      self
        .http
        .post(format!("{}/v4/linode/instances", self.base_url))
        .bearer_auth(token)
        .json(request)
        .send()
    })?;

    self
      .expect_status("create_instance", response, &[StatusCode::OK, StatusCode::ACCEPTED])?
      .json::<LinodeInstance>()
      .map_err(|source| LinodeError::Decode {
        action: "create_instance",
        detail: "Linode instance create response could not be parsed.".to_string(),
        source,
      })
  }

  pub fn get_instance(&self, token: &str, instance_id: u64) -> Result<LinodeInstance, LinodeError> {
    let response = self.send_with_retry(|| {
      self
        .http
        .get(format!("{}/v4/linode/instances/{instance_id}", self.base_url))
        .bearer_auth(token)
        .send()
    })?;

    self.expect_status("get_instance", response, &[StatusCode::OK])?
      .json::<LinodeInstance>()
      .map_err(|source| LinodeError::Decode {
        action: "get_instance",
        detail: "Linode instance detail response could not be parsed.".to_string(),
        source,
      })
  }

  pub fn wait_for_instance_running(
    &self,
    token: &str,
    instance_id: u64,
    poll_interval: Duration,
    max_attempts: usize,
    timeout: Duration,
  ) -> Result<LinodeInstance, LinodeError> {
    let started = Instant::now();

    for attempt in 0..max_attempts {
      if started.elapsed() > timeout {
        return Err(LinodeError::Timeout {
          action: "wait_for_instance_running",
          detail: "Linode instance did not start before the configured timeout. Reconcile or retry the provisioning flow.".to_string(),
        });
      }

      let instance = self.get_instance(token, instance_id)?;
      if instance.status == "running" {
        return Ok(instance);
      }

      // `provisioning`/`booting` are expected; a stopped instance never becomes running on its
      // own, so waiting out the full timeout would only delay the same failure.
      if instance.status == "offline" || instance.status == "stopped" {
        return Err(LinodeError::UnexpectedStatus {
          action: "wait_for_instance_running",
          status: 0,
          detail: format!("Linode instance settled in '{}' rather than running.", instance.status),
        });
      }

      if attempt + 1 < max_attempts {
        thread::sleep(poll_interval);
      }
    }

    Err(LinodeError::Timeout {
      action: "wait_for_instance_running",
      detail: "Linode instance did not start before the configured poll limit. Reconcile or retry the provisioning flow.".to_string(),
    })
  }

  /// First public IPv4 on the instance.
  ///
  /// Linode returns the address in the create response, but `ipv4` also carries private addresses
  /// on accounts using them, so the RFC1918 ranges are skipped rather than taking index 0.
  pub fn extract_public_ipv4(&self, instance: &LinodeInstance) -> Result<String, LinodeError> {
    instance
      .ipv4
      .iter()
      .find(|address| !is_private_ipv4(address))
      .cloned()
      .ok_or_else(|| LinodeError::NotFound {
        action: "extract_public_ipv4",
        detail: "Linode instance has no public IPv4. Wait for networking completion and retry."
          .to_string(),
      })
  }

  pub fn delete_instance(&self, token: &str, instance_id: u64) -> Result<(), LinodeError> {
    let response = self.send_with_retry(|| {
      self
        .http
        .delete(format!("{}/v4/linode/instances/{instance_id}", self.base_url))
        .bearer_auth(token)
        .send()
    })?;

    match self.expect_status("delete_instance", response, &[StatusCode::OK]) {
      Ok(_) => Ok(()),
      // Destroy is idempotent: an instance that is already gone is the state we wanted.
      Err(LinodeError::NotFound { .. }) => Ok(()),
      Err(error) => Err(error),
    }
  }

  fn send_with_retry<F>(&self, mut operation: F) -> Result<reqwest::blocking::Response, LinodeError>
  where
    F: FnMut() -> Result<reqwest::blocking::Response, reqwest::Error>,
  {
    let mut last_transport_error: Option<reqwest::Error> = None;

    for attempt in 0..=self.retry_policy.max_retries {
      match operation() {
        Ok(response) => {
          let status = response.status().as_u16();
          if self.retry_policy.should_retry_status(status)
            && attempt < self.retry_policy.max_retries
          {
            // Linode states how long to wait on a 429. Honouring it beats guessing with backoff,
            // and ignoring it earns another 429.
            let delay = retry_after_delay(&response)
              .unwrap_or_else(|| self.retry_policy.backoff_for_attempt(attempt));
            thread::sleep(delay);
            continue;
          }
          return Ok(response);
        }
        Err(error) => {
          if attempt < self.retry_policy.max_retries && is_retryable_transport_error(&error) {
            last_transport_error = Some(error);
            thread::sleep(self.retry_policy.backoff_for_attempt(attempt));
            continue;
          }

          return Err(LinodeError::Transport {
            detail: "Linode request failed at transport layer. Check connectivity and retry."
              .to_string(),
            source: error,
          });
        }
      }
    }

    if let Some(source) = last_transport_error {
      return Err(LinodeError::Transport {
        detail: "Linode request retries were exhausted by transient transport failures.".to_string(),
        source,
      });
    }

    Err(LinodeError::Timeout {
      action: "send_with_retry",
      detail: "Retry policy exhausted before obtaining a stable response.".to_string(),
    })
  }

  fn expect_status(
    &self,
    action: &'static str,
    response: reqwest::blocking::Response,
    allowed: &[StatusCode],
  ) -> Result<reqwest::blocking::Response, LinodeError> {
    let status = response.status();
    if allowed.contains(&status) {
      return Ok(response);
    }

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
      return Err(LinodeError::Unauthorized {
        action,
        detail: "Linode token was rejected. Verify the token has read/write access to Linodes."
          .to_string(),
      });
    }

    if status == StatusCode::NOT_FOUND {
      return Err(LinodeError::NotFound {
        action,
        detail: "Linode resource was not found.".to_string(),
      });
    }

    let body = response.text().unwrap_or_else(|_| "<unavailable>".to_string());

    Err(LinodeError::UnexpectedStatus {
      action,
      status: status.as_u16(),
      detail: format!("Linode API returned unexpected status. {}", describe_error_body(&body)),
    })
  }
}

/// Turn Linode's `{"errors":[{"reason","field"}]}` envelope into something a user can act on,
/// falling back to a bounded body snippet when the shape is not what was expected.
fn describe_error_body(body: &str) -> String {
  match serde_json::from_str::<LinodeErrorEnvelope>(body) {
    Ok(envelope) if !envelope.errors.is_empty() => envelope
      .errors
      .iter()
      .map(|error| match &error.field {
        Some(field) if !field.trim().is_empty() => format!("{field}: {}", error.reason),
        _ => error.reason.clone(),
      })
      .collect::<Vec<_>>()
      .join("; "),
    _ => format!(
      "Body snippet: {}",
      body.chars().take(256).collect::<String>()
    ),
  }
}

fn retry_after_delay(response: &reqwest::blocking::Response) -> Option<Duration> {
  response
    .headers()
    .get(reqwest::header::RETRY_AFTER)?
    .to_str()
    .ok()?
    .trim()
    .parse::<u64>()
    .ok()
    // A server-supplied wait is trusted, but not unboundedly: a bad value should not park
    // provisioning for an hour.
    .map(|seconds| Duration::from_secs(seconds.min(60)))
}

fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
  error.is_timeout() || error.is_connect() || error.is_request()
}

fn is_private_ipv4(address: &str) -> bool {
  address
    .parse::<std::net::Ipv4Addr>()
    .map(|parsed| parsed.is_private() || parsed.is_loopback() || parsed.is_link_local())
    .unwrap_or(false)
}

#[derive(Debug, Error)]
pub enum LinodeError {
  #[error("Linode transport failure: {detail}")]
  Transport {
    detail: String,
    #[source]
    source: reqwest::Error,
  },
  #[error("Linode token was rejected during {action}: {detail}")]
  Unauthorized { action: &'static str, detail: String },
  #[error("Linode response decode failure in {action}: {detail}")]
  Decode {
    action: &'static str,
    detail: String,
    #[source]
    source: reqwest::Error,
  },
  #[error("Linode request {action} received HTTP {status}: {detail}")]
  UnexpectedStatus {
    action: &'static str,
    status: u16,
    detail: String,
  },
  #[error("Linode resource not found during {action}: {detail}")]
  NotFound { action: &'static str, detail: String },
  #[error("Linode operation timeout during {action}: {detail}")]
  Timeout { action: &'static str, detail: String },
}

#[derive(Debug, Clone, Deserialize)]
struct PaginatedEnvelope<T> {
  data: Vec<T>,
  pages: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct LinodeErrorEnvelope {
  #[serde(default)]
  errors: Vec<LinodeApiError>,
}

#[derive(Debug, Clone, Deserialize)]
struct LinodeApiError {
  reason: String,
  #[serde(default)]
  field: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateLinodeRequest {
  pub label: String,
  pub region: String,
  #[serde(rename = "type")]
  pub instance_type: String,
  pub image: String,
  pub root_pass: String,
  pub booted: bool,
  pub tags: Vec<String>,
  pub metadata: LinodeMetadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinodeMetadata {
  /// The bootstrap payload, base64-encoded — the API takes it that way.
  pub user_data: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinodeAccount {
  #[serde(default)]
  pub euuid: String,
  #[serde(default)]
  pub email: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinodeInstance {
  pub id: u64,
  pub status: String,
  #[serde(default)]
  pub ipv4: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinodeRegion {
  pub id: String,
  #[serde(default)]
  pub capabilities: Vec<String>,
}

impl LinodeRegion {
  /// Whether this region runs the Metadata service, without which `user_data` is ignored.
  pub fn supports_metadata(&self) -> bool {
    self
      .capabilities
      .iter()
      .any(|capability| capability.eq_ignore_ascii_case(METADATA_CAPABILITY))
  }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinodeType {
  pub id: String,
  #[serde(default)]
  pub label: String,
  #[serde(default)]
  pub class: String,
  #[serde(default)]
  pub memory: u64,
  #[serde(default)]
  pub vcpus: u64,
  #[serde(default)]
  pub disk: u64,
  #[serde(default)]
  pub transfer: u64,
  #[serde(default)]
  pub price: LinodePrice,
  /// Per-region price overrides, where the flat price does not apply.
  #[serde(default)]
  pub region_prices: Vec<LinodeRegionPrice>,
}

impl LinodeType {
  /// Price for a region, preferring an explicit regional override over the flat rate.
  pub fn price_for_region(&self, region: &str) -> (f64, f64) {
    self
      .region_prices
      .iter()
      .find(|entry| entry.id == region)
      .map(|entry| (entry.monthly, entry.hourly))
      .unwrap_or((self.price.monthly, self.price.hourly))
  }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LinodePrice {
  #[serde(default)]
  pub monthly: f64,
  #[serde(default)]
  pub hourly: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinodeRegionPrice {
  pub id: String,
  #[serde(default)]
  pub monthly: f64,
  #[serde(default)]
  pub hourly: f64,
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use mockito::{Matcher, Server};

  use super::*;

  fn client(base_url: &str) -> LinodeApiClient {
    LinodeApiClient::new_with_timeout(
      base_url,
      RetryPolicy {
        max_retries: 2,
        base_delay_ms: 1,
        max_delay_ms: 1,
        jitter_max_ms: 0,
      },
      Duration::from_secs(20),
    )
    .expect("client should build")
  }

  #[test]
  fn validates_a_token_against_the_account_endpoint() {
    let mut server = Server::new();
    let _m = server
      .mock("GET", "/v4/account")
      .match_header("authorization", Matcher::Regex("^Bearer .+".to_string()))
      .with_status(200)
      .with_body(r#"{"euuid":"ABC-123","email":"vpn@example.com"}"#)
      .create();

    let account = client(&server.url())
      .get_account("test-token")
      .expect("token should validate");
    assert_eq!(account.euuid, "ABC-123");
    assert_eq!(account.email, "vpn@example.com");
  }

  #[test]
  fn creates_polls_and_reports_the_public_address() {
    let mut server = Server::new();

    let _create = server
      .mock("POST", "/v4/linode/instances")
      .match_header("authorization", Matcher::Regex("^Bearer .+".to_string()))
      .with_status(200)
      .with_body(r#"{"id":1001,"label":"n","status":"provisioning","ipv4":["192.0.2.7"]}"#)
      .create();

    let _poll_booting = server
      .mock("GET", "/v4/linode/instances/1001")
      .with_status(200)
      .with_body(r#"{"id":1001,"label":"n","status":"booting","ipv4":["192.0.2.7"]}"#)
      .expect(1)
      .create();

    let _poll_running = server
      .mock("GET", "/v4/linode/instances/1001")
      .with_status(200)
      .with_body(r#"{"id":1001,"label":"n","status":"running","ipv4":["192.168.1.5","192.0.2.7"]}"#)
      .expect(1)
      .create();

    let api = client(&server.url());
    let created = api
      .create_instance(
        "test-token",
        &CreateLinodeRequest {
          label: "n".to_string(),
          region: DEFAULT_LINODE_REGION.to_string(),
          instance_type: DEFAULT_LINODE_TYPE.to_string(),
          image: DEFAULT_LINODE_IMAGE.to_string(),
          root_pass: "throwaway".to_string(),
          booted: true,
          tags: vec!["goatstare".to_string()],
          metadata: LinodeMetadata {
            user_data: "I2Nsb3VkLWNvbmZpZwo=".to_string(),
          },
        },
      )
      .expect("create should succeed");

    let running = api
      .wait_for_instance_running(
        "test-token",
        created.id,
        Duration::from_millis(1),
        4,
        Duration::from_secs(1),
      )
      .expect("instance should reach running");

    // The private address is listed first; taking index 0 would hand out an unreachable endpoint.
    assert_eq!(
      api.extract_public_ipv4(&running).expect("public ip should resolve"),
      "192.0.2.7"
    );
  }

  #[test]
  fn a_deleted_instance_is_treated_as_already_gone() {
    let mut server = Server::new();
    let _m = server
      .mock("DELETE", "/v4/linode/instances/404")
      .with_status(404)
      .with_body(r#"{"errors":[{"reason":"Not found"}]}"#)
      .create();

    client(&server.url())
      .delete_instance("test-token", 404)
      .expect("a missing instance means destroy already happened");
  }

  /// Linode reports what went wrong in a structured envelope; surfacing the raw JSON instead
  /// makes the user read our transport layer rather than their problem.
  #[test]
  fn api_errors_are_reported_by_reason_and_field() {
    let mut server = Server::new();
    let _m = server
      .mock("POST", "/v4/linode/instances")
      .with_status(400)
      .with_body(r#"{"errors":[{"reason":"Not enough capacity","field":"region"}]}"#)
      .create();

    let error = client(&server.url())
      .create_instance(
        "test-token",
        &CreateLinodeRequest {
          label: "n".to_string(),
          region: "us-ord".to_string(),
          instance_type: DEFAULT_LINODE_TYPE.to_string(),
          image: DEFAULT_LINODE_IMAGE.to_string(),
          root_pass: "throwaway".to_string(),
          booted: true,
          tags: vec![],
          metadata: LinodeMetadata {
            user_data: String::new(),
          },
        },
      )
      .expect_err("a 400 must surface");

    let rendered = error.to_string();
    assert!(rendered.contains("region: Not enough capacity"), "was: {rendered}");
  }

  #[test]
  fn paginated_catalogues_are_walked_to_the_end() {
    let mut server = Server::new();
    let _page_one = server
      .mock("GET", "/v4/regions")
      .match_query(Matcher::UrlEncoded("page".into(), "1".into()))
      .with_status(200)
      .with_body(r#"{"data":[{"id":"us-ord","status":"ok","capabilities":["Metadata"]}],"pages":2}"#)
      .create();
    let _page_two = server
      .mock("GET", "/v4/regions")
      .match_query(Matcher::UrlEncoded("page".into(), "2".into()))
      .with_status(200)
      .with_body(r#"{"data":[{"id":"eu-west","status":"ok","capabilities":[]}],"pages":2}"#)
      .create();

    let regions = client(&server.url())
      .list_regions("test-token")
      .expect("regions should list");

    assert_eq!(regions.len(), 2, "a truncated list silently hides regions");
    assert!(regions[0].supports_metadata());
    assert!(!regions[1].supports_metadata());
  }

  #[test]
  fn region_prices_override_the_flat_rate() {
    let linode_type = LinodeType {
      id: "g6-nanode-1".to_string(),
      label: "Nanode 1GB".to_string(),
      class: "nanode".to_string(),
      memory: 1024,
      vcpus: 1,
      disk: 25_600,
      transfer: 1000,
      price: LinodePrice { monthly: 5.0, hourly: 0.0075 },
      region_prices: vec![LinodeRegionPrice {
        id: "br-gru".to_string(),
        monthly: 7.0,
        hourly: 0.0105,
      }],
    };

    assert_eq!(linode_type.price_for_region("us-ord"), (5.0, 0.0075));
    assert_eq!(linode_type.price_for_region("br-gru"), (7.0, 0.0105));
  }

  #[test]
  fn retries_a_rate_limited_request_then_succeeds() {
    let mut server = Server::new();
    let _limited = server
      .mock("GET", "/v4/account")
      .with_status(429)
      .with_header("retry-after", "0")
      .expect(1)
      .create();
    let _ok = server
      .mock("GET", "/v4/account")
      .with_status(200)
      .with_body(r#"{"euuid":"U2","email":"vpn@example.com"}"#)
      .expect(1)
      .create();

    let account = client(&server.url())
      .get_account("test-token")
      .expect("the retry should succeed");
    assert_eq!(account.euuid, "U2");
  }
}
