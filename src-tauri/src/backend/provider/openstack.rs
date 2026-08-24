//! Generic OpenStack API client — Keystone, Nova, Neutron and Glance.
//!
//! This adapter targets OpenStack the software rather than any one operator's cloud, which makes
//! it different in kind from the others:
//!
//! - **There is no fixed endpoint.** The user supplies a Keystone URL; every other service address
//!   comes from the catalogue that authentication returns.
//! - **Clouds differ in what they offer.** Networking in particular varies: some hand out a routable
//!   address on server create, others require a floating IP allocated and associated afterwards.
//!   Both shapes are handled; a cloud that supports neither gets a capability error naming what is
//!   missing rather than a failed create.
//! - **There are no published prices.** Nothing in the API knows what an operator charges, so
//!   flavors are reported with no price at all rather than a zero that would read as free.
//!
//! Rackspace's OpenStack Flex is the reference target, but nothing here is specific to it.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::Deserialize;
use thiserror::Error;

use super::retry::RetryPolicy;

/// Nova's user-data ceiling, measured on the **base64-encoded** form the request carries.
pub const OPENSTACK_USER_DATA_LIMIT_BYTES: usize = 65_535;

/// Refresh a cached token this long before it expires, so a call starting near the boundary does
/// not race it.
const TOKEN_EXPIRY_SKEW: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct OpenStackConfig {
  pub poll_interval: Duration,
  pub poll_max_attempts: usize,
  pub request_timeout: Duration,
  pub server_ready_timeout: Duration,
  pub dependency_release_timeout: Duration,
}

impl OpenStackConfig {
  pub fn defaults() -> Self {
    Self {
      poll_interval: Duration::from_secs(4),
      poll_max_attempts: 60,
      request_timeout: Duration::from_secs(20),
      server_ready_timeout: Duration::from_secs(300),
      dependency_release_timeout: Duration::from_secs(180),
    }
  }
}

/// What the user supplies. Application credentials are preferred over a username and password:
/// they are scoped, independently revocable, and carry no domain or project name to get wrong.
#[derive(Debug, Clone)]
pub struct OpenStackCredentials {
  pub auth_url: String,
  pub application_credential_id: String,
  pub application_credential_secret: String,
}

impl OpenStackCredentials {
  /// Identity of this exact credential, so a re-saved one invalidates a cached session at once
  /// rather than after the old token expires.
  fn fingerprint(&self) -> String {
    format!("{}|{}", self.auth_url, self.application_credential_id)
  }
}

/// A Keystone session: the token, where each service lives, and when it all stops being valid.
#[derive(Debug, Clone)]
struct CachedSession {
  credential_fingerprint: String,
  token: String,
  project_id: String,
  /// `(service type, region) -> public endpoint`. Region is empty for a catalogue entry that
  /// declares none.
  endpoints: BTreeMap<(String, String), String>,
  expires_at: Instant,
}

impl CachedSession {
  /// The public endpoint for a service in a region.
  ///
  /// An empty region matches a single-region catalogue, which is the common case for a small
  /// cloud. Where a cloud has several and the caller named none, the ambiguity is reported rather
  /// than resolved by picking one — silently provisioning in the wrong city is worse than asking.
  fn endpoint(&self, service: &str, region: &str) -> Result<String, OpenStackError> {
    let matches: Vec<(&(String, String), &String)> = self
      .endpoints
      .iter()
      .filter(|((candidate, _), _)| candidate == service)
      .collect();

    if matches.is_empty() {
      return Err(OpenStackError::Capability {
        detail: format!(
          "This cloud's service catalogue has no public '{service}' endpoint, so this app cannot use it."
        ),
      });
    }

    if !region.trim().is_empty() {
      return matches
        .iter()
        .find(|((_, candidate), _)| candidate == region.trim())
        .map(|(_, url)| (*url).clone())
        .ok_or_else(|| OpenStackError::Capability {
          detail: format!("This cloud offers no '{service}' endpoint in region '{region}'."),
        });
    }

    let regions: Vec<&str> = matches
      .iter()
      .map(|((_, region), _)| region.as_str())
      .filter(|region| !region.is_empty())
      .collect();

    if regions.len() > 1 {
      return Err(OpenStackError::Capability {
        detail: format!(
          "This cloud has more than one region ({}). Choose one before provisioning.",
          regions.join(", ")
        ),
      });
    }

    Ok(matches[0].1.clone())
  }

  fn regions(&self) -> Vec<String> {
    let mut regions: Vec<String> = self
      .endpoints
      .keys()
      .filter(|(service, _)| service == "compute")
      .map(|(_, region)| region.clone())
      .filter(|region| !region.trim().is_empty())
      .collect();
    regions.sort();
    regions.dedup();
    regions
  }
}

pub struct OpenStackApiClient {
  http: Client,
  retry_policy: RetryPolicy,
  session_cache: Mutex<Option<CachedSession>>,
}

impl OpenStackApiClient {
  pub fn new_with_timeout(
    retry_policy: RetryPolicy,
    request_timeout: Duration,
  ) -> Result<Self, OpenStackError> {
    let http = Client::builder()
      .timeout(request_timeout)
      .build()
      .map_err(|source| OpenStackError::Transport {
        detail: "Unable to initialize HTTP client for OpenStack requests.".to_string(),
        source,
      })?;

    Ok(Self {
      http,
      retry_policy,
      session_cache: Mutex::new(None),
    })
  }

  /// A valid session, authenticating if the cache has none for this credential.
  fn session(&self, credentials: &OpenStackCredentials) -> Result<CachedSession, OpenStackError> {
    let fingerprint = credentials.fingerprint();
    let mut cache = self.session_cache.lock().map_err(|_| OpenStackError::Capability {
      detail: "The OpenStack session cache was poisoned by an earlier failure.".to_string(),
    })?;

    if let Some(cached) = cache.as_ref() {
      if cached.credential_fingerprint == fingerprint
        && cached.expires_at > Instant::now() + TOKEN_EXPIRY_SKEW
      {
        return Ok(cached.clone());
      }
    }

    let session = self.authenticate(credentials)?;
    *cache = Some(session.clone());
    Ok(session)
  }

  /// Drop any cached session, so the next call authenticates again.
  ///
  /// Used when a request comes back 401 mid-flight: a token can be revoked, or expire against a
  /// clock this machine disagrees with, and re-authenticating once is cheaper than surfacing an
  /// auth error the user cannot act on.
  fn invalidate_session(&self) {
    if let Ok(mut cache) = self.session_cache.lock() {
      *cache = None;
    }
  }

  pub fn authenticate(
    &self,
    credentials: &OpenStackCredentials,
  ) -> Result<CachedSession, OpenStackError> {
    let auth_url = credentials.auth_url.trim().trim_end_matches('/');
    let body = serde_json::json!({
      "auth": {
        "identity": {
          "methods": ["application_credential"],
          "application_credential": {
            "id": credentials.application_credential_id.trim(),
            "secret": credentials.application_credential_secret,
          },
        },
      },
    });

    let response = self.send_with_retry(|| {
      self
        .http
        .post(format!("{auth_url}/auth/tokens"))
        .json(&body)
        .send()
    })?;

    if response.status == StatusCode::UNAUTHORIZED || response.status == StatusCode::FORBIDDEN {
      return Err(OpenStackError::Unauthorized {
        action: "authenticate",
        detail: "This cloud rejected the application credential. Check the id and secret, and that the credential has not expired or been revoked."
          .to_string(),
      });
    }

    if response.status != StatusCode::CREATED && response.status != StatusCode::OK {
      return Err(OpenStackError::UnexpectedStatus {
        action: "authenticate",
        status: response.status.as_u16(),
        detail: format!(
          "Keystone did not issue a token. {}",
          describe_error_body(&response.body)
        ),
      });
    }

    // Keystone returns the token in a header, not the body — the body carries the catalogue.
    let token = response
      .token_header
      .ok_or_else(|| OpenStackError::UnexpectedStatus {
        action: "authenticate",
        status: response.status.as_u16(),
        detail: "Keystone accepted the credential but returned no X-Subject-Token header."
          .to_string(),
      })?;

    let parsed: TokenEnvelope = parse_json(
      "authenticate",
      "Keystone token response could not be parsed.",
      &response.body,
    )?;

    let mut endpoints = BTreeMap::new();
    for entry in &parsed.token.catalog {
      for endpoint in &entry.endpoints {
        // Only public interfaces are reachable from a desktop; `internal` and `admin` addresses
        // are routable inside the cloud and would time out here.
        if endpoint.interface == "public" {
          endpoints.insert(
            (entry.service_type.clone(), endpoint.region.clone().unwrap_or_default()),
            endpoint.url.trim_end_matches('/').to_string(),
          );
        }
      }
    }

    if endpoints.is_empty() {
      return Err(OpenStackError::Capability {
        detail: "This cloud's service catalogue lists no public endpoints, so this app cannot reach any of its services."
          .to_string(),
      });
    }

    Ok(CachedSession {
      credential_fingerprint: credentials.fingerprint(),
      token,
      project_id: parsed.token.project.map(|project| project.id).unwrap_or_default(),
      endpoints,
      // Keystone states an absolute expiry; this client only needs a relative one, and taking the
      // stated lifetime rather than parsing an RFC3339 timestamp avoids a date dependency for a
      // value that is always about an hour.
      expires_at: Instant::now() + Duration::from_secs(3_000),
    })
  }

  pub fn project_id(&self, credentials: &OpenStackCredentials) -> Result<String, OpenStackError> {
    Ok(self.session(credentials)?.project_id)
  }

  pub fn regions(&self, credentials: &OpenStackCredentials) -> Result<Vec<String>, OpenStackError> {
    Ok(self.session(credentials)?.regions())
  }

  pub fn list_flavors(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
  ) -> Result<Vec<NovaFlavor>, OpenStackError> {
    let body = self.get(credentials, "compute", region, "/flavors/detail", "list_flavors")?;
    let envelope: FlavorsEnvelope =
      parse_json("list_flavors", "Nova flavor list could not be parsed.", &body)?;
    Ok(envelope.flavors)
  }

  /// The newest active image whose name looks like a supported Ubuntu.
  ///
  /// Image names are operator-chosen, so this matches loosely and in preference order rather than
  /// demanding an exact string. A cloud with no match gets a capability error listing what it does
  /// offer, which is far more useful than "image not found".
  pub fn resolve_ubuntu_image(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
  ) -> Result<String, OpenStackError> {
    let body = self.get(
      credentials,
      "image",
      region,
      "/v2/images?status=active&limit=1000",
      "resolve_ubuntu_image",
    )?;
    let envelope: ImagesEnvelope = parse_json(
      "resolve_ubuntu_image",
      "Glance image list could not be parsed.",
      &body,
    )?;

    // Newest supported release first: a cloud offering both should provision the one this app is
    // actually tested against.
    for wanted in [["ubuntu 24.04", "noble"], ["ubuntu 22.04", "jammy"]] {
      if let Some(image) = envelope.images.iter().find(|image| {
        let name = image.name.to_ascii_lowercase();
        wanted.iter().any(|needle| name.contains(needle))
      }) {
        return Ok(image.id.clone());
      }
    }

    let available: Vec<&str> = envelope
      .images
      .iter()
      .map(|image| image.name.as_str())
      .filter(|name| !name.trim().is_empty())
      .take(8)
      .collect();

    Err(OpenStackError::Capability {
      detail: if available.is_empty() {
        "This cloud offers no active images, so there is nothing to provision from.".to_string()
      } else {
        format!(
          "This cloud offers no Ubuntu 24.04 or 22.04 image. It does offer: {}. GoatStare needs a cloud-init Ubuntu image.",
          available.join(", ")
        )
      },
    })
  }

  /// A network the project can attach a server to.
  ///
  /// Returns the network id and whether it is an external network being attached directly — the
  /// two topologies differ in whether a floating IP is needed afterwards.
  pub fn resolve_network(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
  ) -> Result<(String, bool), OpenStackError> {
    let body = self.get(credentials, "network", region, "/v2.0/networks", "resolve_network")?;
    let envelope: NetworksEnvelope =
      parse_json("resolve_network", "Neutron network list could not be parsed.", &body)?;

    let tenant: Vec<&NeutronNetwork> = envelope
      .networks
      .iter()
      .filter(|network| !network.external && network.status.eq_ignore_ascii_case("ACTIVE"))
      .collect();

    if tenant.len() == 1 {
      return Ok((tenant[0].id.clone(), false));
    }

    if tenant.len() > 1 {
      // Several to choose from: prefer a conventionally-named one rather than guessing.
      if let Some(preferred) = tenant.iter().find(|network| {
        let name = network.name.to_ascii_lowercase();
        name == "private" || name == "default"
      }) {
        return Ok((preferred.id.clone(), false));
      }

      let names: Vec<&str> = tenant.iter().map(|network| network.name.as_str()).collect();
      return Err(OpenStackError::Capability {
        detail: format!(
          "This project has several networks ({}) and none is named 'private' or 'default', so this app cannot tell which to use.",
          names.join(", ")
        ),
      });
    }

    // No tenant network. Some clouds hand out a routable address by attaching directly to a shared
    // external network instead, which is the DreamCompute shape.
    if let Some(shared_external) = envelope
      .networks
      .iter()
      .find(|network| network.external && network.shared && network.status.eq_ignore_ascii_case("ACTIVE"))
    {
      return Ok((shared_external.id.clone(), true));
    }

    Err(OpenStackError::Capability {
      detail: "This project has no network this app can attach a server to. Create a private network, or ask your operator for a shared external one."
        .to_string(),
    })
  }

  pub fn create_security_group(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    name: &str,
  ) -> Result<String, OpenStackError> {
    let body = serde_json::json!({
      "security_group": { "name": name, "description": "GoatStare WireGuard ingress" },
    });
    let response = self.post(
      credentials,
      "network",
      region,
      "/v2.0/security-groups",
      &body,
      "create_security_group",
    )?;
    let envelope: SecurityGroupEnvelope = parse_json(
      "create_security_group",
      "Neutron security group response could not be parsed.",
      &response,
    )?;
    Ok(envelope.security_group.id)
  }

  pub fn authorize_udp_ingress(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    security_group_id: &str,
    port: u16,
  ) -> Result<(), OpenStackError> {
    let body = serde_json::json!({
      "security_group_rule": {
        "security_group_id": security_group_id,
        "direction": "ingress",
        "ethertype": "IPv4",
        "protocol": "udp",
        "port_range_min": port,
        "port_range_max": port,
        "remote_ip_prefix": "0.0.0.0/0",
      },
    });
    self.post(
      credentials,
      "network",
      region,
      "/v2.0/security-group-rules",
      &body,
      "authorize_udp_ingress",
    )?;
    Ok(())
  }

  /// Resolve security group names to ids.
  ///
  /// Nova reports a server's groups by name while Neutron deletes by id, so teardown has to make
  /// this hop. Only the names asked for are returned, so a group belonging to something else is
  /// never a candidate for deletion.
  pub fn security_group_ids_by_name(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    names: &[String],
  ) -> Result<Vec<(String, String)>, OpenStackError> {
    if names.is_empty() {
      return Ok(Vec::new());
    }

    let body = self.get(
      credentials,
      "network",
      region,
      "/v2.0/security-groups",
      "security_group_ids_by_name",
    )?;
    let envelope: SecurityGroupsEnvelope = parse_json(
      "security_group_ids_by_name",
      "Neutron security group list could not be parsed.",
      &body,
    )?;

    Ok(
      envelope
        .security_groups
        .into_iter()
        .filter(|group| names.iter().any(|name| name == &group.name))
        .map(|group| (group.id, group.name))
        .collect(),
    )
  }

  pub fn delete_security_group(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    security_group_id: &str,
  ) -> Result<(), OpenStackError> {
    self.delete(
      credentials,
      "network",
      region,
      &format!("/v2.0/security-groups/{security_group_id}"),
      "delete_security_group",
    )
  }

  #[allow(clippy::too_many_arguments)]
  pub fn create_server(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    name: &str,
    image_id: &str,
    flavor_id: &str,
    network_id: &str,
    security_group: &str,
    user_data_base64: &str,
  ) -> Result<String, OpenStackError> {
    let body = serde_json::json!({
      "server": {
        "name": name,
        "imageRef": image_id,
        "flavorRef": flavor_id,
        "networks": [{ "uuid": network_id }],
        "security_groups": [{ "name": security_group }],
        "user_data": user_data_base64,
        "metadata": { "goatstare": "managed" },
      },
    });

    let response = self.post(credentials, "compute", region, "/servers", &body, "create_server")?;
    let envelope: ServerEnvelope =
      parse_json("create_server", "Nova server create response could not be parsed.", &response)?;
    Ok(envelope.server.id)
  }

  pub fn get_server(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    server_id: &str,
  ) -> Result<NovaServer, OpenStackError> {
    let body = self.get(
      credentials,
      "compute",
      region,
      &format!("/servers/{server_id}"),
      "get_server",
    )?;
    let envelope: ServerEnvelope =
      parse_json("get_server", "Nova server response could not be parsed.", &body)?;
    Ok(envelope.server)
  }

  pub fn delete_server(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    server_id: &str,
  ) -> Result<(), OpenStackError> {
    self.delete(
      credentials,
      "compute",
      region,
      &format!("/servers/{server_id}"),
      "delete_server",
    )
  }

  pub fn wait_for_server_active(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    server_id: &str,
    poll_interval: Duration,
    max_attempts: usize,
    timeout: Duration,
  ) -> Result<NovaServer, OpenStackError> {
    let started = Instant::now();

    for attempt in 0..max_attempts {
      if started.elapsed() > timeout {
        return Err(OpenStackError::Timeout {
          action: "wait_for_server_active",
          detail: "The server did not become ACTIVE before the configured timeout. Reconcile or retry the provisioning flow."
            .to_string(),
        });
      }

      match self.get_server(credentials, region, server_id) {
        Ok(server) => {
          if server.status.eq_ignore_ascii_case("ACTIVE") {
            return Ok(server);
          }
          // ERROR is terminal — waiting out the timeout only delays the same failure.
          if server.status.eq_ignore_ascii_case("ERROR") {
            return Err(OpenStackError::UnexpectedStatus {
              action: "wait_for_server_active",
              status: 0,
              detail: server
                .fault
                .as_ref()
                .map(|fault| format!("The server entered ERROR: {}", fault.message))
                .unwrap_or_else(|| "The server entered ERROR.".to_string()),
            });
          }
        }
        Err(OpenStackError::NotFound { .. }) => {}
        Err(error) => return Err(error),
      }

      if attempt + 1 < max_attempts {
        thread::sleep(poll_interval);
      }
    }

    Err(OpenStackError::Timeout {
      action: "wait_for_server_active",
      detail: "The server did not become ACTIVE before the configured poll limit. Reconcile or retry the provisioning flow."
        .to_string(),
    })
  }

  pub fn wait_for_server_gone(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    server_id: &str,
    poll_interval: Duration,
    timeout: Duration,
  ) -> Result<(), OpenStackError> {
    let started = Instant::now();

    while started.elapsed() < timeout {
      match self.get_server(credentials, region, server_id) {
        Err(OpenStackError::NotFound { .. }) => return Ok(()),
        Ok(server) if server.status.eq_ignore_ascii_case("DELETED") => return Ok(()),
        Ok(_) => {}
        Err(error) => return Err(error),
      }
      thread::sleep(poll_interval);
    }

    Err(OpenStackError::Timeout {
      action: "wait_for_server_gone",
      detail: "The server was still present after the delete timeout.".to_string(),
    })
  }

  /// Ports belonging to a server, which is how its floating IPs are found.
  pub fn ports_for_server(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    server_id: &str,
  ) -> Result<Vec<String>, OpenStackError> {
    let body = self.get(
      credentials,
      "network",
      region,
      &format!("/v2.0/ports?device_id={server_id}"),
      "ports_for_server",
    )?;
    let envelope: PortsEnvelope =
      parse_json("ports_for_server", "Neutron port list could not be parsed.", &body)?;
    Ok(envelope.ports.into_iter().map(|port| port.id).collect())
  }

  pub fn floating_ips_for_ports(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    port_ids: &[String],
  ) -> Result<Vec<FloatingIp>, OpenStackError> {
    if port_ids.is_empty() {
      return Ok(Vec::new());
    }

    let body = self.get(
      credentials,
      "network",
      region,
      "/v2.0/floatingips",
      "floating_ips_for_ports",
    )?;
    let envelope: FloatingIpsEnvelope = parse_json(
      "floating_ips_for_ports",
      "Neutron floating IP list could not be parsed.",
      &body,
    )?;

    Ok(
      envelope
        .floatingips
        .into_iter()
        .filter(|floating| {
          floating
            .port_id
            .as_ref()
            .is_some_and(|port| port_ids.iter().any(|candidate| candidate == port))
        })
        .collect(),
    )
  }

  /// Allocate a floating IP from an external network and attach it to a port.
  pub fn associate_floating_ip(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    port_id: &str,
  ) -> Result<FloatingIp, OpenStackError> {
    let networks = self.get(credentials, "network", region, "/v2.0/networks", "associate_floating_ip")?;
    let envelope: NetworksEnvelope = parse_json(
      "associate_floating_ip",
      "Neutron network list could not be parsed.",
      &networks,
    )?;

    let pool = envelope
      .networks
      .iter()
      .find(|network| network.external && network.status.eq_ignore_ascii_case("ACTIVE"))
      .ok_or_else(|| OpenStackError::Capability {
        detail: "This server has only private addresses and the cloud offers no external network to allocate a floating IP from, so nothing outside the cloud can reach it."
          .to_string(),
      })?;

    let body = serde_json::json!({
      "floatingip": { "floating_network_id": pool.id, "port_id": port_id },
    });
    let response = self.post(
      credentials,
      "network",
      region,
      "/v2.0/floatingips",
      &body,
      "associate_floating_ip",
    )?;
    let envelope: FloatingIpEnvelope = parse_json(
      "associate_floating_ip",
      "Neutron floating IP response could not be parsed.",
      &response,
    )?;
    Ok(envelope.floatingip)
  }

  pub fn release_floating_ip(
    &self,
    credentials: &OpenStackCredentials,
    region: &str,
    floating_ip_id: &str,
  ) -> Result<(), OpenStackError> {
    self.delete(
      credentials,
      "network",
      region,
      &format!("/v2.0/floatingips/{floating_ip_id}"),
      "release_floating_ip",
    )
  }

  pub fn retry_policy(&self) -> RetryPolicy {
    self.retry_policy
  }

  fn get(
    &self,
    credentials: &OpenStackCredentials,
    service: &str,
    region: &str,
    path: &str,
    action: &'static str,
  ) -> Result<String, OpenStackError> {
    self.authenticated(credentials, service, region, action, |base, token| {
      self.http.get(format!("{base}{path}")).header("X-Auth-Token", token).send()
    })
  }

  fn post(
    &self,
    credentials: &OpenStackCredentials,
    service: &str,
    region: &str,
    path: &str,
    body: &serde_json::Value,
    action: &'static str,
  ) -> Result<String, OpenStackError> {
    self.authenticated(credentials, service, region, action, |base, token| {
      self
        .http
        .post(format!("{base}{path}"))
        .header("X-Auth-Token", token)
        .json(body)
        .send()
    })
  }

  fn delete(
    &self,
    credentials: &OpenStackCredentials,
    service: &str,
    region: &str,
    path: &str,
    action: &'static str,
  ) -> Result<(), OpenStackError> {
    match self.authenticated(credentials, service, region, action, |base, token| {
      self.http.delete(format!("{base}{path}")).header("X-Auth-Token", token).send()
    }) {
      Ok(_) => Ok(()),
      // Destroy is idempotent: a resource already gone is the state we wanted.
      Err(OpenStackError::NotFound { .. }) => Ok(()),
      Err(error) => Err(error),
    }
  }

  /// Run a request against a catalogue endpoint, re-authenticating once on a 401.
  ///
  /// A token can be revoked, or expire against a clock this machine disagrees with. Retrying once
  /// with a fresh session is cheaper than surfacing an auth error the user cannot act on.
  fn authenticated<F>(
    &self,
    credentials: &OpenStackCredentials,
    service: &str,
    region: &str,
    action: &'static str,
    mut request: F,
  ) -> Result<String, OpenStackError>
  where
    F: FnMut(&str, &str) -> Result<reqwest::blocking::Response, reqwest::Error>,
  {
    for attempt in 0..2 {
      let session = self.session(credentials)?;
      let base = session.endpoint(service, region)?;
      let response = self.send_with_retry(|| request(&base, &session.token))?;

      if response.status == StatusCode::UNAUTHORIZED && attempt == 0 {
        self.invalidate_session();
        continue;
      }

      return self.expect_ok(action, response);
    }

    Err(OpenStackError::Unauthorized {
      action,
      detail: "This cloud rejected the token twice, including one freshly issued.".to_string(),
    })
  }

  fn expect_ok(
    &self,
    action: &'static str,
    response: RawResponse,
  ) -> Result<String, OpenStackError> {
    if response.status.is_success() {
      return Ok(response.body);
    }

    if response.status == StatusCode::NOT_FOUND {
      return Err(OpenStackError::NotFound {
        action,
        detail: "The OpenStack resource was not found.".to_string(),
      });
    }

    if response.status == StatusCode::UNAUTHORIZED || response.status == StatusCode::FORBIDDEN {
      return Err(OpenStackError::Unauthorized {
        action,
        detail: format!(
          "This cloud rejected the request. The application credential may lack the required role. {}",
          describe_error_body(&response.body)
        ),
      });
    }

    if response.status == StatusCode::CONFLICT {
      return Err(OpenStackError::Conflict {
        action,
        detail: describe_error_body(&response.body),
      });
    }

    Err(OpenStackError::UnexpectedStatus {
      action,
      status: response.status.as_u16(),
      detail: describe_error_body(&response.body),
    })
  }

  fn send_with_retry<F>(&self, mut operation: F) -> Result<RawResponse, OpenStackError>
  where
    F: FnMut() -> Result<reqwest::blocking::Response, reqwest::Error>,
  {
    let mut last_transport_error: Option<reqwest::Error> = None;

    for attempt in 0..=self.retry_policy.max_retries {
      match operation() {
        Ok(response) => {
          let status = response.status();
          let token_header = response
            .headers()
            .get("x-subject-token")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());
          let body = response.text().unwrap_or_else(|_| "<unavailable>".to_string());

          if self.retry_policy.should_retry_status(status.as_u16())
            && attempt < self.retry_policy.max_retries
          {
            thread::sleep(self.retry_policy.backoff_for_attempt(attempt));
            continue;
          }

          return Ok(RawResponse { status, body, token_header });
        }
        Err(error) => {
          if attempt < self.retry_policy.max_retries && is_retryable_transport_error(&error) {
            last_transport_error = Some(error);
            thread::sleep(self.retry_policy.backoff_for_attempt(attempt));
            continue;
          }

          return Err(OpenStackError::Transport {
            detail: "OpenStack request failed at transport layer. Check the auth URL and connectivity, then retry."
              .to_string(),
            source: error,
          });
        }
      }
    }

    if let Some(source) = last_transport_error {
      return Err(OpenStackError::Transport {
        detail: "OpenStack request retries were exhausted by transient transport failures."
          .to_string(),
        source,
      });
    }

    Err(OpenStackError::Timeout {
      action: "send_with_retry",
      detail: "Retry policy exhausted before obtaining a stable response.".to_string(),
    })
  }
}

struct RawResponse {
  status: StatusCode,
  body: String,
  /// Keystone returns an issued token in this header rather than in the body.
  token_header: Option<String>,
}

fn parse_json<T: serde::de::DeserializeOwned>(
  action: &'static str,
  detail: &str,
  body: &str,
) -> Result<T, OpenStackError> {
  serde_json::from_str(body).map_err(|source| OpenStackError::Decode {
    action,
    detail: detail.to_string(),
    source,
  })
}

/// OpenStack services answer with `{"<type>": {"message": ...}}` — the key varies by service, so
/// the message is pulled out generically rather than by name.
fn describe_error_body(body: &str) -> String {
  if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
    if let Some(object) = value.as_object() {
      for entry in object.values() {
        if let Some(message) = entry.get("message").and_then(|message| message.as_str()) {
          return message.to_string();
        }
      }
    }
  }

  format!("Body snippet: {}", body.chars().take(256).collect::<String>())
}

fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
  error.is_timeout() || error.is_connect() || error.is_request()
}

/// Whether an address is reachable from outside the cloud.
///
/// Some clouds put a routable address straight on the server; others hand out RFC1918 and expect a
/// floating IP. Telling them apart is what decides whether one has to be allocated.
///
/// RFC 5737 documentation ranges (192.0.2/24, 198.51.100/24, 203.0.113/24) are deliberately *not*
/// excluded. No cloud assigns one, so excluding them protects against nothing — while every
/// example and test in this codebase uses them, and treating them as unreachable would make the
/// tests exercise the opposite path from the one they name.
pub fn is_publicly_routable(address: &str) -> bool {
  address
    .parse::<std::net::Ipv4Addr>()
    .map(|parsed| {
      !(parsed.is_private()
        || parsed.is_loopback()
        || parsed.is_link_local()
        || parsed.is_unspecified()
        || parsed.is_broadcast()
        // 100.64.0.0/10, carrier-grade NAT: routable-looking, and not reachable from outside.
        || (parsed.octets()[0] == 100 && (64..128).contains(&parsed.octets()[1])))
    })
    .unwrap_or(false)
}

#[derive(Debug, Error)]
pub enum OpenStackError {
  #[error("OpenStack transport failure: {detail}")]
  Transport {
    detail: String,
    #[source]
    source: reqwest::Error,
  },
  #[error("This cloud rejected the credential during {action}: {detail}")]
  Unauthorized { action: &'static str, detail: String },
  /// Something this cloud does not offer, as opposed to something that went wrong. Surfaced to the
  /// caller as `ProviderAdapterError::Unsupported`.
  #[error("This OpenStack cloud cannot support this operation: {detail}")]
  Capability { detail: String },
  #[error("OpenStack response decode failure in {action}: {detail}")]
  Decode {
    action: &'static str,
    detail: String,
    #[source]
    source: serde_json::Error,
  },
  #[error("OpenStack request {action} received HTTP {status}: {detail}")]
  UnexpectedStatus {
    action: &'static str,
    status: u16,
    detail: String,
  },
  /// A resource still in use — typically a security group whose port has not been released yet.
  #[error("OpenStack resource is still in use during {action}: {detail}")]
  Conflict { action: &'static str, detail: String },
  #[error("OpenStack resource not found during {action}: {detail}")]
  NotFound { action: &'static str, detail: String },
  #[error("OpenStack operation timeout during {action}: {detail}")]
  Timeout { action: &'static str, detail: String },
}

#[derive(Debug, Deserialize)]
struct TokenEnvelope {
  token: TokenBody,
}

#[derive(Debug, Deserialize)]
struct TokenBody {
  #[serde(default)]
  catalog: Vec<CatalogEntry>,
  #[serde(default)]
  project: Option<TokenProject>,
}

#[derive(Debug, Deserialize)]
struct TokenProject {
  #[serde(default)]
  id: String,
}

#[derive(Debug, Deserialize)]
struct CatalogEntry {
  #[serde(rename = "type")]
  service_type: String,
  #[serde(default)]
  endpoints: Vec<CatalogEndpoint>,
}

#[derive(Debug, Deserialize)]
struct CatalogEndpoint {
  #[serde(default)]
  interface: String,
  #[serde(default)]
  region: Option<String>,
  #[serde(default)]
  url: String,
}

#[derive(Debug, Deserialize)]
struct FlavorsEnvelope {
  #[serde(default)]
  flavors: Vec<NovaFlavor>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NovaFlavor {
  pub id: String,
  #[serde(default)]
  pub name: String,
  #[serde(default)]
  pub ram: u64,
  #[serde(default)]
  pub vcpus: u64,
  #[serde(default)]
  pub disk: u64,
  /// Nova reports a disabled flavor so tooling can see it; it cannot be launched.
  #[serde(default, rename = "OS-FLV-DISABLED:disabled")]
  pub disabled: bool,
  #[serde(default = "default_true", rename = "os-flavor-access:is_public")]
  pub is_public: bool,
}

fn default_true() -> bool {
  true
}

#[derive(Debug, Deserialize)]
struct ImagesEnvelope {
  #[serde(default)]
  images: Vec<GlanceImage>,
}

#[derive(Debug, Deserialize)]
struct GlanceImage {
  id: String,
  #[serde(default)]
  name: String,
}

#[derive(Debug, Deserialize)]
struct NetworksEnvelope {
  #[serde(default)]
  networks: Vec<NeutronNetwork>,
}

#[derive(Debug, Deserialize)]
struct NeutronNetwork {
  id: String,
  #[serde(default)]
  name: String,
  #[serde(default)]
  status: String,
  #[serde(default, rename = "router:external")]
  external: bool,
  #[serde(default)]
  shared: bool,
}

#[derive(Debug, Deserialize)]
struct SecurityGroupEnvelope {
  security_group: SecurityGroupBody,
}

#[derive(Debug, Deserialize)]
struct SecurityGroupBody {
  id: String,
}

#[derive(Debug, Deserialize)]
struct SecurityGroupsEnvelope {
  #[serde(default)]
  security_groups: Vec<NamedSecurityGroup>,
}

#[derive(Debug, Deserialize)]
struct NamedSecurityGroup {
  id: String,
  #[serde(default)]
  name: String,
}

#[derive(Debug, Deserialize)]
struct ServerEnvelope {
  server: NovaServer,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NovaServer {
  pub id: String,
  #[serde(default)]
  pub status: String,
  /// `network name -> [addresses]`. The shape a cloud uses here is what distinguishes a directly
  /// routable server from one needing a floating IP.
  #[serde(default)]
  pub addresses: BTreeMap<String, Vec<ServerAddress>>,
  #[serde(default)]
  pub fault: Option<ServerFault>,
  /// Nova reports the groups by **name**; Neutron deletes by id, so the names are resolved
  /// separately. This is what lets teardown find the group without storing a second identifier.
  #[serde(default)]
  pub security_groups: Vec<ServerSecurityGroup>,
}

impl NovaServer {
  /// An address anything outside the cloud can reach.
  ///
  /// A floating address wins outright. Otherwise a fixed address is only usable if it is actually
  /// routable — clouds that hand out RFC1918 need a floating IP allocated instead.
  pub fn reachable_address(&self) -> Option<String> {
    let all = || self.addresses.values().flatten();

    if let Some(floating) = all()
      .find(|address| address.kind.as_deref() == Some("floating") && !address.addr.trim().is_empty())
    {
      return Some(floating.addr.clone());
    }

    all()
      .find(|address| is_publicly_routable(&address.addr))
      .map(|address| address.addr.clone())
  }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerAddress {
  #[serde(default)]
  pub addr: String,
  #[serde(default, rename = "OS-EXT-IPS:type")]
  pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerFault {
  #[serde(default)]
  pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerSecurityGroup {
  #[serde(default)]
  pub name: String,
}

#[derive(Debug, Deserialize)]
struct PortsEnvelope {
  #[serde(default)]
  ports: Vec<NeutronPort>,
}

#[derive(Debug, Deserialize)]
struct NeutronPort {
  id: String,
}

#[derive(Debug, Deserialize)]
struct FloatingIpsEnvelope {
  #[serde(default)]
  floatingips: Vec<FloatingIp>,
}

#[derive(Debug, Deserialize)]
struct FloatingIpEnvelope {
  floatingip: FloatingIp,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FloatingIp {
  pub id: String,
  #[serde(default)]
  pub floating_ip_address: String,
  #[serde(default)]
  pub port_id: Option<String>,
}

#[cfg(test)]
mod tests {
  use super::*;

  fn session_with(endpoints: &[(&str, &str, &str)]) -> CachedSession {
    CachedSession {
      credential_fingerprint: "test".to_string(),
      token: "gAAAAAtest".to_string(),
      project_id: "proj-1".to_string(),
      endpoints: endpoints
        .iter()
        .map(|(service, region, url)| {
          ((service.to_string(), region.to_string()), url.to_string())
        })
        .collect(),
      expires_at: Instant::now() + Duration::from_secs(3600),
    }
  }

  #[test]
  fn a_single_region_catalogue_resolves_without_naming_a_region() {
    let session = session_with(&[("compute", "RegionOne", "https://nova.example/v2.1")]);
    assert_eq!(
      session.endpoint("compute", "").expect("a lone region needs no choosing"),
      "https://nova.example/v2.1"
    );
  }

  /// Picking one silently could provision in a different country from the one the user believes
  /// they chose, which is the whole point of choosing.
  #[test]
  fn an_ambiguous_multi_region_catalogue_asks_rather_than_guessing() {
    let session = session_with(&[
      ("compute", "RegionOne", "https://one.example/v2.1"),
      ("compute", "RegionTwo", "https://two.example/v2.1"),
    ]);

    let error = session.endpoint("compute", "").expect_err("ambiguity must be reported");
    assert!(matches!(error, OpenStackError::Capability { .. }));
    assert!(error.to_string().contains("more than one region"), "was: {error}");

    assert_eq!(
      session.endpoint("compute", "RegionTwo").expect("a named region resolves"),
      "https://two.example/v2.1"
    );
  }

  #[test]
  fn a_missing_service_is_named_as_a_capability_gap() {
    let session = session_with(&[("compute", "RegionOne", "https://nova.example/v2.1")]);
    let error = session.endpoint("network", "").expect_err("no Neutron must be reported");

    assert!(matches!(error, OpenStackError::Capability { .. }));
    assert!(error.to_string().contains("network"), "was: {error}");
  }

  /// A floating address beats a fixed one; a private fixed address is not reachable at all.
  #[test]
  fn a_floating_address_is_preferred_over_a_private_fixed_one() {
    let server: NovaServer = serde_json::from_value(serde_json::json!({
      "id": "srv-1",
      "status": "ACTIVE",
      "addresses": { "private": [
        { "addr": "10.0.0.5", "OS-EXT-IPS:type": "fixed" },
        { "addr": "203.0.113.7", "OS-EXT-IPS:type": "floating" },
      ]},
    }))
    .expect("server should parse");

    assert_eq!(server.reachable_address().as_deref(), Some("203.0.113.7"));
  }

  /// The DreamCompute shape: a routable address straight on the server, no floating IP involved.
  #[test]
  fn a_routable_fixed_address_is_usable_without_a_floating_ip() {
    let server: NovaServer = serde_json::from_value(serde_json::json!({
      "id": "srv-1",
      "status": "ACTIVE",
      "addresses": { "public": [{ "addr": "198.51.100.8", "OS-EXT-IPS:type": "fixed" }]},
    }))
    .expect("server should parse");

    assert_eq!(server.reachable_address().as_deref(), Some("198.51.100.8"));
  }

  #[test]
  fn a_server_with_only_private_addresses_has_nothing_reachable() {
    let server: NovaServer = serde_json::from_value(serde_json::json!({
      "id": "srv-1",
      "status": "ACTIVE",
      "addresses": { "private": [{ "addr": "192.168.1.20", "OS-EXT-IPS:type": "fixed" }]},
    }))
    .expect("server should parse");

    assert_eq!(server.reachable_address(), None);
  }

  /// Carrier-grade NAT looks routable and is not, so a server behind it needs a floating IP like
  /// any other private address.
  #[test]
  fn carrier_grade_nat_is_not_treated_as_reachable() {
    assert!(!is_publicly_routable("100.64.0.1"));
    assert!(!is_publicly_routable("10.0.0.1"));
    assert!(!is_publicly_routable("192.168.0.1"));
    assert!(!is_publicly_routable("172.16.0.1"));
    assert!(is_publicly_routable("203.0.113.7"));
    assert!(is_publicly_routable("198.51.100.8"));
    assert!(is_publicly_routable("100.128.0.1"));
  }

  #[test]
  fn service_error_messages_are_surfaced_rather_than_raw_json() {
    assert_eq!(
      describe_error_body(r#"{"badRequest":{"code":400,"message":"Invalid flavorRef"}}"#),
      "Invalid flavorRef"
    );
    assert_eq!(
      describe_error_body(r#"{"NeutronError":{"message":"Quota exceeded","type":"OverQuota"}}"#),
      "Quota exceeded"
    );
  }
}
