use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::retry::RetryPolicy;

pub const DEFAULT_REGION: &str = "nyc1";
pub const DEFAULT_SIZE: &str = "s-1vcpu-1gb";
pub const DEFAULT_IMAGE: &str = "ubuntu-24-04-x64";

#[derive(Debug, Clone)]
pub struct DigitalOceanConfig {
  pub region: &'static str,
  pub size: &'static str,
  pub image: &'static str,
  pub poll_interval: Duration,
  pub poll_max_attempts: usize,
  pub request_timeout: Duration,
  pub droplet_ready_timeout: Duration,
}

impl DigitalOceanConfig {
  pub fn phase1_defaults() -> Self {
    Self {
      region: DEFAULT_REGION,
      size: DEFAULT_SIZE,
      image: DEFAULT_IMAGE,
      poll_interval: Duration::from_secs(4),
      poll_max_attempts: 60,
      request_timeout: Duration::from_secs(20),
      droplet_ready_timeout: Duration::from_secs(240),
    }
  }
}

#[derive(Debug, Clone)]
pub struct DigitalOceanApiClient {
  base_url: String,
  http: Client,
  retry_policy: RetryPolicy,
}

impl DigitalOceanApiClient {
  pub fn new_with_timeout(
    base_url: &str,
    retry_policy: RetryPolicy,
    request_timeout: Duration,
  ) -> Result<Self, DigitalOceanError> {
    let http = Client::builder()
      .timeout(request_timeout)
      .build()
      .map_err(|source| DigitalOceanError::Transport {
        detail: "Unable to initialize HTTP client for DigitalOcean requests.".to_string(),
        source,
      })?;

    Ok(Self {
      base_url: base_url.trim_end_matches('/').to_string(),
      http,
      retry_policy,
    })
  }

  pub fn validate_token(&self, token: &str) -> Result<Account, DigitalOceanError> {
    let response = self.send_with_retry(|| {
      self
        .http
        .get(format!("{}/v2/account", self.base_url))
        .bearer_auth(token)
        .send()
    })?;

    self.expect_status("validate_token", response, &[StatusCode::OK])?
      .json::<AccountEnvelope>()
      .map(|envelope| envelope.account)
      .map_err(|source| DigitalOceanError::Decode {
        action: "validate_token",
        detail: "DigitalOcean account response could not be parsed.".to_string(),
        source,
      })
  }

  pub fn create_droplet(
    &self,
    token: &str,
    request: &CreateDropletRequest,
  ) -> Result<Droplet, DigitalOceanError> {
    let response = self.send_with_retry(|| {
      self
        .http
        .post(format!("{}/v2/droplets", self.base_url))
        .bearer_auth(token)
        .json(request)
        .send()
    })?;

    self.expect_status("create_droplet", response, &[StatusCode::CREATED, StatusCode::ACCEPTED])?
      .json::<DropletEnvelope>()
      .map(|envelope| envelope.droplet)
      .map_err(|source| DigitalOceanError::Decode {
        action: "create_droplet",
        detail: "DigitalOcean droplet create response could not be parsed.".to_string(),
        source,
      })
  }

  pub fn list_sizes(&self, token: &str) -> Result<Vec<Size>, DigitalOceanError> {
    let response = self.send_with_retry(|| {
      self
        .http
        .get(format!("{}/v2/sizes", self.base_url))
        .bearer_auth(token)
        .send()
    })?;

    self.expect_status("list_sizes", response, &[StatusCode::OK])?
      .json::<SizesEnvelope>()
      .map(|envelope| envelope.sizes)
      .map_err(|source| DigitalOceanError::Decode {
        action: "list_sizes",
        detail: "DigitalOcean sizes response could not be parsed.".to_string(),
        source,
      })
  }

  pub fn get_droplet(&self, token: &str, droplet_id: u64) -> Result<Droplet, DigitalOceanError> {
    let response = self.send_with_retry(|| {
      self
        .http
        .get(format!("{}/v2/droplets/{droplet_id}", self.base_url))
        .bearer_auth(token)
        .send()
    })?;

    self.expect_status("get_droplet", response, &[StatusCode::OK])?
      .json::<DropletEnvelope>()
      .map(|envelope| envelope.droplet)
      .map_err(|source| DigitalOceanError::Decode {
        action: "get_droplet",
        detail: "DigitalOcean droplet detail response could not be parsed.".to_string(),
        source,
      })
  }

  pub fn wait_for_droplet_active(
    &self,
    token: &str,
    droplet_id: u64,
    poll_interval: Duration,
    max_attempts: usize,
    timeout: Duration,
  ) -> Result<Droplet, DigitalOceanError> {
    let started = Instant::now();

    for attempt in 0..max_attempts {
      if started.elapsed() > timeout {
        return Err(DigitalOceanError::Timeout {
          action: "wait_for_droplet_active",
          detail: "Droplet did not become active before the configured timeout. Reconcile or retry the provisioning flow.".to_string(),
        });
      }

      let droplet = self.get_droplet(token, droplet_id)?;
      if droplet.status == "active" {
        return Ok(droplet);
      }

      if attempt + 1 < max_attempts {
        thread::sleep(poll_interval);
      }
    }

    Err(DigitalOceanError::Timeout {
      action: "wait_for_droplet_active",
      detail: "Droplet did not become active before the configured poll limit. Reconcile or retry the provisioning flow.".to_string(),
    })
  }

  pub fn extract_public_ipv4(&self, droplet: &Droplet) -> Result<String, DigitalOceanError> {
    droplet
      .networks
      .v4
      .iter()
      .find(|network| network.kind == "public")
      .map(|network| network.ip_address.clone())
      .ok_or_else(|| DigitalOceanError::NotFound {
        action: "extract_public_ipv4",
        detail: "Droplet has no public IPv4 yet. Wait for networking completion and retry."
          .to_string(),
      })
  }

  pub fn delete_droplet(&self, token: &str, droplet_id: u64) -> Result<(), DigitalOceanError> {
    let response = self.send_with_retry(|| {
      self
        .http
        .delete(format!("{}/v2/droplets/{droplet_id}", self.base_url))
        .bearer_auth(token)
        .send()
    })?;

    self.expect_status("delete_droplet", response, &[StatusCode::NO_CONTENT])?;
    Ok(())
  }

  fn send_with_retry<F>(&self, mut operation: F) -> Result<reqwest::blocking::Response, DigitalOceanError>
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
            thread::sleep(self.retry_policy.backoff_for_attempt(attempt));
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

          return Err(DigitalOceanError::Transport {
            detail: "DigitalOcean request failed at transport layer. Check connectivity and retry."
              .to_string(),
            source: error,
          });
        }
      }
    }

    if let Some(source) = last_transport_error {
      return Err(DigitalOceanError::Transport {
        detail:
          "DigitalOcean request retries were exhausted by transient transport failures."
            .to_string(),
        source,
      });
    }

    Err(DigitalOceanError::Timeout {
      action: "send_with_retry",
      detail: "Retry policy exhausted before obtaining a stable response."
        .to_string(),
    })
  }

  fn expect_status(
    &self,
    action: &'static str,
    response: reqwest::blocking::Response,
    allowed: &[StatusCode],
  ) -> Result<reqwest::blocking::Response, DigitalOceanError> {
    let status = response.status();
    if allowed.contains(&status) {
      return Ok(response);
    }

    if status == StatusCode::UNAUTHORIZED {
      return Err(DigitalOceanError::Unauthorized {
        action,
        detail: "DigitalOcean token was rejected. Verify token scope and token_ref selection."
          .to_string(),
      });
    }

    if status == StatusCode::NOT_FOUND {
      return Err(DigitalOceanError::NotFound {
        action,
        detail: "DigitalOcean resource was not found.".to_string(),
      });
    }

    let body_snippet = response
      .text()
      .unwrap_or_else(|_| "<unavailable>".to_string())
      .chars()
      .take(256)
      .collect::<String>();

    Err(DigitalOceanError::UnexpectedStatus {
      action,
      status: status.as_u16(),
      detail: format!(
        "DigitalOcean API returned unexpected status. Body snippet: {body_snippet}"
      ),
    })
  }
}

fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
  error.is_timeout() || error.is_connect() || error.is_request()
}

#[derive(Debug, Error)]
pub enum DigitalOceanError {
  #[error("DigitalOcean transport failure: {detail}")]
  Transport {
    detail: String,
    #[source]
    source: reqwest::Error,
  },
  #[error("DigitalOcean token was rejected during {action}: {detail}")]
  Unauthorized { action: &'static str, detail: String },
  #[error("DigitalOcean response decode failure in {action}: {detail}")]
  Decode {
    action: &'static str,
    detail: String,
    #[source]
    source: reqwest::Error,
  },
  #[error("DigitalOcean request {action} received HTTP {status}: {detail}")]
  UnexpectedStatus {
    action: &'static str,
    status: u16,
    detail: String,
  },
  #[error("DigitalOcean resource not found during {action}: {detail}")]
  NotFound { action: &'static str, detail: String },
  #[error("DigitalOcean operation timeout during {action}: {detail}")]
  Timeout { action: &'static str, detail: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDropletRequest {
  pub name: String,
  pub region: String,
  pub size: String,
  pub image: String,
  pub tags: Vec<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub user_data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountEnvelope {
  pub account: Account,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
  pub uuid: String,
  pub email: String,
  pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropletEnvelope {
  pub droplet: Droplet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Droplet {
  pub id: u64,
  pub name: String,
  pub status: String,
  pub networks: DropletNetworks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropletNetworks {
  pub v4: Vec<DropletNetworkV4>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropletNetworkV4 {
  pub ip_address: String,
  #[serde(rename = "type")]
  pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizesEnvelope {
  pub sizes: Vec<Size>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Size {
  pub slug: String,
  pub available: bool,
  pub memory: u64,
  pub vcpus: u64,
  pub disk: u64,
  pub transfer: f64,
  pub price_monthly: f64,
  pub price_hourly: f64,
  pub regions: Vec<String>,
  pub description: String,
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use mockito::{Matcher, Server};

  use super::{
    CreateDropletRequest, DEFAULT_IMAGE, DEFAULT_REGION, DEFAULT_SIZE, DigitalOceanApiClient,
    RetryPolicy,
  };

  fn client(base_url: &str) -> DigitalOceanApiClient {
    DigitalOceanApiClient::new_with_timeout(
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
  fn validates_token_via_account_endpoint() {
    let mut server = Server::new();
    let _m = server
      .mock("GET", "/v2/account")
      .match_header("authorization", Matcher::Regex("^Bearer .+".to_string()))
      .with_status(200)
      .with_body(r#"{"account":{"uuid":"abc","email":"vpn@example.com","status":"active"}}"#)
      .create();

    let api = client(&server.url());
    let account = api.validate_token("test-token").expect("token should validate");
    assert_eq!(account.uuid, "abc");
  }

  #[test]
  fn retries_on_429_then_succeeds() {
    let mut server = Server::new();
    let _rate_limited = server
      .mock("GET", "/v2/account")
      .match_header("authorization", Matcher::Regex("^Bearer .+".to_string()))
      .with_status(429)
      .expect(1)
      .create();
    let _success = server
      .mock("GET", "/v2/account")
      .match_header("authorization", Matcher::Regex("^Bearer .+".to_string()))
      .with_status(200)
      .with_body(r#"{"account":{"uuid":"u1","email":"vpn@example.com","status":"active"}}"#)
      .expect(1)
      .create();

    let api = client(&server.url());
    let account = api.validate_token("test-token").expect("token should validate after retry");
    assert_eq!(account.uuid, "u1");
  }

  #[test]
  fn creates_polls_gets_ip_and_deletes_droplet() {
    let mut server = Server::new();

    let _create = server
      .mock("POST", "/v2/droplets")
      .match_header("authorization", Matcher::Regex("^Bearer .+".to_string()))
      .with_status(202)
      .with_body(
        r#"{
          "droplet": {
            "id": 1001,
            "name": "phase1-node",
            "status": "new",
            "networks": {"v4": []}
          }
        }"#,
      )
      .create();

    let _poll_new = server
      .mock("GET", "/v2/droplets/1001")
      .match_header("authorization", Matcher::Regex("^Bearer .+".to_string()))
      .with_status(200)
      .with_body(
        r#"{
          "droplet": {
            "id": 1001,
            "name": "phase1-node",
            "status": "new",
            "networks": {"v4": []}
          }
        }"#,
      )
      .expect(1)
      .create();

    let _poll_active = server
      .mock("GET", "/v2/droplets/1001")
      .match_header("authorization", Matcher::Regex("^Bearer .+".to_string()))
      .with_status(200)
      .with_body(
        r#"{
          "droplet": {
            "id": 1001,
            "name": "phase1-node",
            "status": "active",
            "networks": {
              "v4": [{"ip_address":"203.0.113.5","type":"public"}]
            }
          }
        }"#,
      )
      .expect(1)
      .create();

    let _delete = server
      .mock("DELETE", "/v2/droplets/1001")
      .match_header("authorization", Matcher::Regex("^Bearer .+".to_string()))
      .with_status(204)
      .create();

    let api = client(&server.url());
    let request = CreateDropletRequest {
      name: "phase1-node".to_string(),
      region: DEFAULT_REGION.to_string(),
      size: DEFAULT_SIZE.to_string(),
      image: DEFAULT_IMAGE.to_string(),
      tags: vec!["phase1".to_string()],
      user_data: Some("#cloud-config\n".to_string()),
    };

    let created = api
      .create_droplet("test-token", &request)
      .expect("droplet create should succeed");
    let active = api
      .wait_for_droplet_active(
        "test-token",
        created.id,
        Duration::from_millis(1),
        4,
        Duration::from_secs(1),
      )
      .expect("droplet should become active");
    let ip = api
      .extract_public_ipv4(&active)
      .expect("public ip should be present");
    assert_eq!(ip, "203.0.113.5");

    api
      .delete_droplet("test-token", created.id)
      .expect("droplet delete should succeed");
  }

  #[test]
  fn retries_transient_connect_failure_then_succeeds() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
      atomic::{AtomicUsize, Ordering},
      Arc,
    };
    use std::thread;

    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let address = listener.local_addr().expect("local addr should exist");
    let request_count = Arc::new(AtomicUsize::new(0));
    let request_count_thread = request_count.clone();

    let server = thread::spawn(move || {
      for stream in listener.incoming().take(2) {
        let mut stream = stream.expect("incoming connection should exist");
        let current = request_count_thread.fetch_add(1, Ordering::SeqCst);
        if current == 0 {
          continue;
        }

        let mut buffer = [0_u8; 512];
        let _ = stream.read(&mut buffer);
        let body = r#"{"account":{"uuid":"u2","email":"vpn@example.com","status":"active"}}"#;
        let response = format!(
          "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
          body.len(),
          body
        );
        stream.write_all(response.as_bytes()).expect("response should write");
        break;
      }
    });

    let api = client(&format!("http://{}", address));
    let account = api.validate_token("test-token").expect("token should validate after retry");
    assert_eq!(account.uuid, "u2");
    assert!(request_count.load(Ordering::SeqCst) >= 2);

    let _ = server.join();
  }

  #[test]
  fn lists_sizes_from_api() {
    let mut server = Server::new();
    let _m = server
      .mock("GET", "/v2/sizes")
      .match_header("authorization", Matcher::Regex("^Bearer .+".to_string()))
      .with_status(200)
      .with_body(
        r#"{
          "sizes": [
            {
              "slug": "s-1vcpu-1gb",
              "available": true,
              "memory": 1024,
              "vcpus": 1,
              "disk": 25,
              "transfer": 1,
              "price_monthly": 6,
              "price_hourly": 0.00893,
              "regions": ["nyc1","sfo3"],
              "description": "Basic"
            }
          ]
        }"#,
      )
      .create();

    let api = client(&server.url());
    let sizes = api.list_sizes("test-token").expect("sizes should list");
    assert_eq!(sizes.len(), 1);
    assert_eq!(sizes[0].slug, "s-1vcpu-1gb");
    assert_eq!(sizes[0].price_monthly, 6.0);
  }
}
