//! AWS EC2 API client.
//!
//! Structured like `digitalocean.rs`: a blocking `reqwest` client, a retry policy, typed errors, and
//! wire types. Three differences follow from AWS rather than from taste.
//!
//! - **Signing.** Every request carries a SigV4 signature over its canonical form. `aws-sigv4` does
//!   the computation; this module only assembles the request and hands it over.
//! - **XML.** The EC2 Query API has no JSON protocol, so responses are XML. SSM and STS are JSON.
//! - **Region in the host.** `ec2.<region>.amazonaws.com`, and the region is also part of the
//!   signing scope, so a region argument reaches further here than a slug does for DigitalOcean.
//!
//! Nothing here reads state back off a provisioned instance. Readiness is decided from EC2's own
//! metadata (AGENTS.md §2).

use std::time::{Duration, SystemTime};

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
  SignableBody, SignableRequest, SigningSettings, sign as sigv4_sign,
};
use aws_sigv4::sign::v4;
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::Deserialize;
use thiserror::Error;

use super::retry::RetryPolicy;

/// The region used for calls that are not about a particular region, and the fallback when none is
/// supplied. `DescribeRegions` answers for the whole account from any region.
pub const DEFAULT_EC2_REGION: &str = "us-east-1";

/// EC2 Query API version. Pinned, because the response shapes parsed below belong to it.
const EC2_API_VERSION: &str = "2016-11-15";

/// Canonical's published parameter naming the current Ubuntu 22.04 LTS AMI in each region.
///
/// Resolved at provision time rather than pinned per region: a hardcoded table goes stale, which
/// means silently launching an unpatched image, and breaks outright in any region not listed.
pub const UBUNTU_AMI_SSM_PARAMETER: &str =
  "/aws/service/canonical/ubuntu/server/22.04/stable/current/amd64/hvm/ebs-gp2/ami-id";

#[derive(Debug, Clone)]
pub struct Ec2Config {
  pub poll_interval: Duration,
  pub poll_max_attempts: usize,
  pub request_timeout: Duration,
  pub instance_ready_timeout: Duration,
  /// How long to keep retrying a security-group delete that is still blocked by its instance.
  pub dependency_release_timeout: Duration,
}

impl Ec2Config {
  pub fn defaults() -> Self {
    Self {
      poll_interval: Duration::from_secs(4),
      poll_max_attempts: 60,
      request_timeout: Duration::from_secs(20),
      instance_ready_timeout: Duration::from_secs(240),
      // Terminating an instance detaches its network interface asynchronously; the security group
      // cannot go until that finishes. Measured at tens of seconds, so this is generous.
      dependency_release_timeout: Duration::from_secs(180),
    }
  }
}

/// An EC2 instance, as much of one as this app cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ec2Instance {
  pub instance_id: String,
  /// `pending` · `running` · `shutting-down` · `terminated` · `stopping` · `stopped`
  pub state: String,
  pub public_ip: Option<String>,
}

/// The AWS credential pair, resolved from the stored provider credential.
#[derive(Debug, Clone)]
pub struct AwsCredentials {
  pub access_key_id: String,
  pub secret_access_key: String,
  /// Present only for temporary credentials. Long-lived IAM user keys have none.
  pub session_token: Option<String>,
}

pub struct Ec2ApiClient {
  http: Client,
  retry_policy: RetryPolicy,
  /// Overridden in tests to point every service at one mock server.
  base_url_override: Option<String>,
}

impl Ec2ApiClient {
  pub fn new_with_timeout(
    retry_policy: RetryPolicy,
    request_timeout: Duration,
  ) -> Result<Self, Ec2Error> {
    Ok(Self {
      http: Self::build_http(request_timeout)?,
      retry_policy,
      base_url_override: None,
    })
  }

  /// Point every AWS service at one base URL so tests can use a single mock server.
  #[cfg(test)]
  pub fn with_base_url(
    base_url: &str,
    retry_policy: RetryPolicy,
    request_timeout: Duration,
  ) -> Result<Self, Ec2Error> {
    Ok(Self {
      http: Self::build_http(request_timeout)?,
      retry_policy,
      base_url_override: Some(base_url.trim_end_matches('/').to_string()),
    })
  }

  fn build_http(request_timeout: Duration) -> Result<Client, Ec2Error> {
    Client::builder()
      .timeout(request_timeout)
      .build()
      .map_err(|source| Ec2Error::Transport {
        detail: "Could not build the AWS HTTP client.".to_string(),
        source,
      })
  }

  fn endpoint(&self, service: &str, region: &str) -> String {
    match &self.base_url_override {
      Some(base) => format!("{base}/{service}"),
      None => format!("https://{service}.{region}.amazonaws.com"),
    }
  }

  // --- signing -------------------------------------------------------------------------------

  /// Sign a POST form request and send it, with retries on throttling and transport failures.
  ///
  /// Everything AWS needs is in the signature: the canonical URI, the query string, the signed
  /// headers, and a SHA-256 of the body. Getting any of it wrong yields a 403 that says only
  /// `SignatureDoesNotMatch`, which is why the computation is delegated rather than written here.
  fn send_signed(
    &self,
    service: &str,
    region: &str,
    credentials: &AwsCredentials,
    body: &str,
    content_type: &str,
    extra_headers: &[(&str, &str)],
    action: &str,
  ) -> Result<String, Ec2Error> {
    let url = self.endpoint(service, region);

    for attempt in 0..=self.retry_policy.max_retries {
      let signed = self.sign_request(service, region, credentials, &url, body, content_type, extra_headers, action)?;

      let mut request = self
        .http
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .body(body.to_string());

      for (name, value) in signed {
        request = request.header(name, value);
      }

      match request.send() {
        Ok(response) => {
          let status = response.status();
          let text = response.text().unwrap_or_default();

          if status.is_success() {
            return Ok(text);
          }

          // AWS reports throttling as a 400 with a code in the body, not only as a 429, so the
          // shared status rule is not enough on its own.
          let retryable = self.retry_policy.should_retry_status(status.as_u16())
            || is_throttling_body(&text);

          if retryable && attempt < self.retry_policy.max_retries {
            std::thread::sleep(self.retry_policy.backoff_for_attempt(attempt));
            continue;
          }

          return Err(Self::classify_failure(action, status, &text));
        }
        Err(source) => {
          let retryable = source.is_timeout() || source.is_connect() || source.is_request();
          if retryable && attempt < self.retry_policy.max_retries {
            std::thread::sleep(self.retry_policy.backoff_for_attempt(attempt));
            continue;
          }

          return Err(Ec2Error::Transport {
            detail: format!("AWS request failed while trying to {action}."),
            source,
          });
        }
      }
    }

    Err(Ec2Error::Timeout {
      action: action.to_string(),
      detail: "Retries were exhausted without a usable AWS response.".to_string(),
    })
  }

  #[allow(clippy::too_many_arguments)]
  fn sign_request(
    &self,
    service: &str,
    region: &str,
    credentials: &AwsCredentials,
    url: &str,
    body: &str,
    content_type: &str,
    extra_headers: &[(&str, &str)],
    action: &str,
  ) -> Result<Vec<(String, String)>, Ec2Error> {
    let identity = Credentials::new(
      credentials.access_key_id.clone(),
      credentials.secret_access_key.clone(),
      credentials.session_token.clone(),
      None,
      "goatstare-provider-credential",
    )
    .into();

    let signing_params = v4::SigningParams::builder()
      .identity(&identity)
      .region(region)
      .name(service)
      .time(SystemTime::now())
      .settings(SigningSettings::default())
      .build()
      .map_err(|error| Ec2Error::Signing {
        action: action.to_string(),
        detail: error.to_string(),
      })?;

    let mut headers: Vec<(&str, &str)> = vec![("content-type", content_type)];
    headers.extend_from_slice(extra_headers);

    let signable = SignableRequest::new(
      "POST",
      url,
      headers.into_iter(),
      SignableBody::Bytes(body.as_bytes()),
    )
    .map_err(|error| Ec2Error::Signing {
      action: action.to_string(),
      detail: error.to_string(),
    })?;

    let (instructions, _signature) = sigv4_sign(signable, &signing_params.into())
      .map_err(|error| Ec2Error::Signing {
        action: action.to_string(),
        detail: error.to_string(),
      })?
      .into_parts();

    let (signed_headers, _params) = instructions.into_parts();

    Ok(
      signed_headers
        .into_iter()
        .map(|header| (header.name().to_string(), header.value().to_string()))
        .collect(),
    )
  }

  /// Turn a failed AWS response into the most specific error it supports.
  fn classify_failure(action: &str, status: StatusCode, body: &str) -> Ec2Error {
    let code = extract_error_code(body).unwrap_or_default();
    let message = extract_error_message(body).unwrap_or_else(|| truncate(body, 256));

    // AWS answers a bad key with 401 or 403 depending on the service; the code is the reliable
    // signal. Distinguishing this matters because the recovery is "re-enter your key", not "retry".
    if status == StatusCode::UNAUTHORIZED
      || status == StatusCode::FORBIDDEN
      || matches!(
        code.as_str(),
        "AuthFailure"
          | "UnauthorizedOperation"
          | "InvalidClientTokenId"
          | "SignatureDoesNotMatch"
          | "MissingAuthenticationToken"
          | "AccessDenied"
          | "AccessDeniedException"
      )
    {
      return Ec2Error::Unauthorized {
        action: action.to_string(),
        detail: format!("AWS rejected the credential ({code}): {message}"),
      };
    }

    if code.ends_with(".NotFound") || code == "ParameterNotFound" || code == "InvalidGroup.NotFound" {
      return Ec2Error::NotFound {
        action: action.to_string(),
        detail: format!("{code}: {message}"),
      };
    }

    if code == "DependencyViolation" {
      return Ec2Error::DependencyViolation {
        action: action.to_string(),
        detail: message,
      };
    }

    Ec2Error::UnexpectedStatus {
      action: action.to_string(),
      status: status.as_u16(),
      detail: if code.is_empty() {
        message
      } else {
        format!("{code}: {message}")
      },
    }
  }

  // --- EC2 Query API -------------------------------------------------------------------------

  fn ec2_query(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    params: &[(String, String)],
    action: &str,
  ) -> Result<String, Ec2Error> {
    let mut form = vec![("Version".to_string(), EC2_API_VERSION.to_string())];
    form.extend_from_slice(params);
    let body = encode_form(&form);

    self.send_signed(
      "ec2",
      region,
      credentials,
      &body,
      "application/x-www-form-urlencoded",
      &[],
      action,
    )
  }

  /// Confirm the credential and report who it belongs to.
  pub fn get_caller_identity(
    &self,
    region: &str,
    credentials: &AwsCredentials,
  ) -> Result<CallerIdentity, Ec2Error> {
    let body = encode_form(&[
      ("Action".to_string(), "GetCallerIdentity".to_string()),
      ("Version".to_string(), "2011-06-15".to_string()),
    ]);

    let xml = self.send_signed(
      "sts",
      region,
      credentials,
      &body,
      "application/x-www-form-urlencoded",
      &[],
      "validate the AWS credential",
    )?;

    Ok(CallerIdentity {
      account: first_tag(&xml, "Account").unwrap_or_else(|| "unknown".to_string()),
      arn: first_tag(&xml, "Arn").unwrap_or_else(|| "unknown".to_string()),
      user_id: first_tag(&xml, "UserId").unwrap_or_else(|| "unknown".to_string()),
    })
  }

  pub fn describe_regions(
    &self,
    credentials: &AwsCredentials,
  ) -> Result<Vec<String>, Ec2Error> {
    let xml = self.ec2_query(
      DEFAULT_EC2_REGION,
      credentials,
      &[("Action".to_string(), "DescribeRegions".to_string())],
      "list AWS regions",
    )?;

    Ok(all_tags(&xml, "regionName"))
  }

  /// Which of the given instance types the region can actually launch.
  pub fn filter_offered_instance_types(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    candidates: &[&str],
  ) -> Result<Vec<String>, Ec2Error> {
    let mut params = vec![
      ("Action".to_string(), "DescribeInstanceTypeOfferings".to_string()),
      ("LocationType".to_string(), "region".to_string()),
      ("Filter.1.Name".to_string(), "instance-type".to_string()),
    ];
    for (index, candidate) in candidates.iter().enumerate() {
      params.push((format!("Filter.1.Value.{}", index + 1), (*candidate).to_string()));
    }

    let xml = self.ec2_query(region, credentials, &params, "list available instance types")?;
    Ok(all_tags(&xml, "instanceType"))
  }

  /// The current Ubuntu AMI for a region, from Canonical's public SSM parameter.
  pub fn resolve_ubuntu_ami(
    &self,
    region: &str,
    credentials: &AwsCredentials,
  ) -> Result<String, Ec2Error> {
    let body = serde_json::json!({ "Name": UBUNTU_AMI_SSM_PARAMETER }).to_string();

    let response = self.send_signed(
      "ssm",
      region,
      credentials,
      &body,
      "application/x-amz-json-1.1",
      &[("x-amz-target", "AmazonSSM.GetParameter")],
      "resolve the Ubuntu AMI for this region",
    )?;

    let parsed: SsmGetParameterResponse =
      serde_json::from_str(&response).map_err(|source| Ec2Error::Decode {
        action: "resolve the Ubuntu AMI for this region".to_string(),
        detail: "SSM did not return a parameter in the expected shape.".to_string(),
        source,
      })?;

    Ok(parsed.parameter.value)
  }

  /// The account's default VPC in a region, if it has one.
  pub fn default_vpc_id(
    &self,
    region: &str,
    credentials: &AwsCredentials,
  ) -> Result<Option<String>, Ec2Error> {
    let xml = self.ec2_query(
      region,
      credentials,
      &[
        ("Action".to_string(), "DescribeVpcs".to_string()),
        ("Filter.1.Name".to_string(), "is-default".to_string()),
        ("Filter.1.Value.1".to_string(), "true".to_string()),
      ],
      "look up the default VPC",
    )?;

    Ok(first_tag(&xml, "vpcId"))
  }

  pub fn first_subnet_id(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    vpc_id: &str,
  ) -> Result<Option<String>, Ec2Error> {
    let xml = self.ec2_query(
      region,
      credentials,
      &[
        ("Action".to_string(), "DescribeSubnets".to_string()),
        ("Filter.1.Name".to_string(), "vpc-id".to_string()),
        ("Filter.1.Value.1".to_string(), vpc_id.to_string()),
      ],
      "look up a subnet in the default VPC",
    )?;

    Ok(first_tag(&xml, "subnetId"))
  }

  /// Create a security group that admits only the WireGuard port, and tag it at creation.
  ///
  /// Tagging happens in the create call rather than afterwards so there is no window in which an
  /// untagged group exists — one that a later cleanup would not recognise as ours.
  pub fn create_security_group(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    vpc_id: &str,
    group_name: &str,
    description: &str,
    tags: &[(String, String)],
  ) -> Result<String, Ec2Error> {
    let mut params = vec![
      ("Action".to_string(), "CreateSecurityGroup".to_string()),
      ("GroupName".to_string(), group_name.to_string()),
      ("GroupDescription".to_string(), description.to_string()),
      ("VpcId".to_string(), vpc_id.to_string()),
      ("TagSpecification.1.ResourceType".to_string(), "security-group".to_string()),
    ];
    push_tag_params(&mut params, "TagSpecification.1", tags);

    let xml = self.ec2_query(region, credentials, &params, "create the security group")?;

    first_tag(&xml, "groupId").ok_or_else(|| Ec2Error::Decode {
      action: "create the security group".to_string(),
      detail: "CreateSecurityGroup did not return a groupId.".to_string(),
      source: serde_json::from_str::<serde_json::Value>("").unwrap_err(),
    })
  }

  pub fn authorize_udp_ingress(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    group_id: &str,
    port: u16,
  ) -> Result<(), Ec2Error> {
    self.ec2_query(
      region,
      credentials,
      &[
        ("Action".to_string(), "AuthorizeSecurityGroupIngress".to_string()),
        ("GroupId".to_string(), group_id.to_string()),
        ("IpPermissions.1.IpProtocol".to_string(), "udp".to_string()),
        ("IpPermissions.1.FromPort".to_string(), port.to_string()),
        ("IpPermissions.1.ToPort".to_string(), port.to_string()),
        ("IpPermissions.1.IpRanges.1.CidrIp".to_string(), "0.0.0.0/0".to_string()),
        (
          "IpPermissions.1.IpRanges.1.Description".to_string(),
          "GoatStare WireGuard".to_string(),
        ),
      ],
      "open the WireGuard port",
    )?;

    Ok(())
  }

  pub fn delete_security_group(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    group_id: &str,
  ) -> Result<(), Ec2Error> {
    match self.ec2_query(
      region,
      credentials,
      &[
        ("Action".to_string(), "DeleteSecurityGroup".to_string()),
        ("GroupId".to_string(), group_id.to_string()),
      ],
      "delete the security group",
    ) {
      Ok(_) => Ok(()),
      // Already gone is the desired end state, so destroy stays idempotent.
      Err(Ec2Error::NotFound { .. }) => Ok(()),
      Err(other) => Err(other),
    }
  }

  #[allow(clippy::too_many_arguments)]
  pub fn run_instance(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    image_id: &str,
    instance_type: &str,
    subnet_id: &str,
    group_id: &str,
    user_data_base64: &str,
    tags: &[(String, String)],
  ) -> Result<Ec2Instance, Ec2Error> {
    let mut params = vec![
      ("Action".to_string(), "RunInstances".to_string()),
      ("ImageId".to_string(), image_id.to_string()),
      ("InstanceType".to_string(), instance_type.to_string()),
      ("MinCount".to_string(), "1".to_string()),
      ("MaxCount".to_string(), "1".to_string()),
      ("UserData".to_string(), user_data_base64.to_string()),
      // The public address is what the client dials, so it has to be requested explicitly — a
      // subnet whose default is off would otherwise yield an unreachable node.
      ("NetworkInterface.1.DeviceIndex".to_string(), "0".to_string()),
      ("NetworkInterface.1.SubnetId".to_string(), subnet_id.to_string()),
      ("NetworkInterface.1.AssociatePublicIpAddress".to_string(), "true".to_string()),
      ("NetworkInterface.1.SecurityGroupId.1".to_string(), group_id.to_string()),
      ("TagSpecification.1.ResourceType".to_string(), "instance".to_string()),
    ];
    push_tag_params(&mut params, "TagSpecification.1", tags);

    let xml = self.ec2_query(region, credentials, &params, "launch the EC2 instance")?;

    parse_first_instance(&xml).ok_or_else(|| Ec2Error::Decode {
      action: "launch the EC2 instance".to_string(),
      detail: "RunInstances did not return an instance.".to_string(),
      source: serde_json::from_str::<serde_json::Value>("").unwrap_err(),
    })
  }

  /// One instance's current state, or `None` when EC2 no longer reports it.
  pub fn describe_instance(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    instance_id: &str,
  ) -> Result<Option<Ec2Instance>, Ec2Error> {
    match self.ec2_query(
      region,
      credentials,
      &[
        ("Action".to_string(), "DescribeInstances".to_string()),
        ("InstanceId.1".to_string(), instance_id.to_string()),
      ],
      "read the EC2 instance state",
    ) {
      Ok(xml) => Ok(parse_first_instance(&xml)),
      Err(Ec2Error::NotFound { .. }) => Ok(None),
      Err(other) => Err(other),
    }
  }

  /// The security group attached to an instance.
  ///
  /// Read from the instance rather than tracked separately, so destroy cleans up the group that is
  /// actually attached even if the local record is incomplete.
  pub fn security_group_for_instance(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    instance_id: &str,
  ) -> Result<Option<String>, Ec2Error> {
    match self.ec2_query(
      region,
      credentials,
      &[
        ("Action".to_string(), "DescribeInstances".to_string()),
        ("InstanceId.1".to_string(), instance_id.to_string()),
      ],
      "read the instance security group",
    ) {
      Ok(xml) => Ok(first_tag(&xml, "groupId")),
      Err(Ec2Error::NotFound { .. }) => Ok(None),
      Err(other) => Err(other),
    }
  }

  pub fn terminate_instance(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    instance_id: &str,
  ) -> Result<(), Ec2Error> {
    match self.ec2_query(
      region,
      credentials,
      &[
        ("Action".to_string(), "TerminateInstances".to_string()),
        ("InstanceId.1".to_string(), instance_id.to_string()),
      ],
      "terminate the EC2 instance",
    ) {
      Ok(_) => Ok(()),
      // Already gone is the desired end state.
      Err(Ec2Error::NotFound { .. }) => Ok(()),
      Err(other) => Err(other),
    }
  }

  /// Poll until the instance is running and has a public address.
  ///
  /// Dual-bounded by elapsed time and attempts, like the DigitalOcean equivalent, so neither a slow
  /// region nor a stuck poll can hang the flow forever.
  pub fn wait_for_instance_running(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    instance_id: &str,
    poll_interval: Duration,
    max_attempts: usize,
    timeout: Duration,
  ) -> Result<Ec2Instance, Ec2Error> {
    let started = SystemTime::now();

    for attempt in 0..max_attempts {
      if let Some(instance) = self.describe_instance(region, credentials, instance_id)? {
        match instance.state.as_str() {
          "running" if instance.public_ip.is_some() => return Ok(instance),
          "shutting-down" | "terminated" | "stopping" | "stopped" => {
            return Err(Ec2Error::UnexpectedStatus {
              action: "wait for the EC2 instance to start".to_string(),
              status: 0,
              detail: format!(
                "Instance {instance_id} reached '{}' instead of running.",
                instance.state
              ),
            });
          }
          _ => {}
        }
      }

      if started.elapsed().map(|value| value >= timeout).unwrap_or(false) {
        break;
      }

      if attempt + 1 < max_attempts {
        std::thread::sleep(poll_interval);
      }
    }

    Err(Ec2Error::Timeout {
      action: "wait for the EC2 instance to start".to_string(),
      detail: format!("Instance {instance_id} did not reach running with a public address in time."),
    })
  }

  /// Poll until EC2 reports the instance terminated.
  ///
  /// A security group cannot be deleted while its network interface is still attached, so destroy
  /// has to wait for this before it can finish cleaning up.
  pub fn wait_for_instance_terminated(
    &self,
    region: &str,
    credentials: &AwsCredentials,
    instance_id: &str,
    poll_interval: Duration,
    timeout: Duration,
  ) -> Result<(), Ec2Error> {
    let started = SystemTime::now();

    loop {
      match self.describe_instance(region, credentials, instance_id)? {
        None => return Ok(()),
        Some(instance) if instance.state == "terminated" => return Ok(()),
        Some(_) => {}
      }

      if started.elapsed().map(|value| value >= timeout).unwrap_or(false) {
        return Err(Ec2Error::Timeout {
          action: "wait for the EC2 instance to terminate".to_string(),
          detail: format!("Instance {instance_id} was still terminating after the timeout."),
        });
      }

      std::thread::sleep(poll_interval);
    }
  }

  pub fn retry_policy(&self) -> &RetryPolicy {
    &self.retry_policy
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerIdentity {
  pub account: String,
  pub arn: String,
  pub user_id: String,
}

#[derive(Debug, Deserialize)]
struct SsmGetParameterResponse {
  #[serde(rename = "Parameter")]
  parameter: SsmParameter,
}

#[derive(Debug, Deserialize)]
struct SsmParameter {
  #[serde(rename = "Value")]
  value: String,
}

#[derive(Debug, Error)]
pub enum Ec2Error {
  #[error("AWS transport failure: {detail}")]
  Transport {
    detail: String,
    #[source]
    source: reqwest::Error,
  },
  #[error("AWS rejected the credential while trying to {action}: {detail}")]
  Unauthorized { action: String, detail: String },
  #[error("Could not read the AWS response while trying to {action}: {detail}")]
  Decode {
    action: String,
    detail: String,
    #[source]
    source: serde_json::Error,
  },
  #[error("AWS returned status {status} while trying to {action}: {detail}")]
  UnexpectedStatus {
    action: String,
    status: u16,
    detail: String,
  },
  #[error("AWS reported the resource does not exist while trying to {action}: {detail}")]
  NotFound { action: String, detail: String },
  /// The resource is still in use by another one. For a security group, that means its instance
  /// has not finished releasing its network interface yet.
  #[error("AWS resource is still in use while trying to {action}: {detail}")]
  DependencyViolation { action: String, detail: String },
  #[error("Timed out while trying to {action}: {detail}")]
  Timeout { action: String, detail: String },
  #[error("Could not sign the AWS request to {action}: {detail}")]
  Signing { action: String, detail: String },
}

// --- encoding and parsing helpers ------------------------------------------------------------

/// Percent-encode per RFC 3986, which is what SigV4 canonicalization expects.
///
/// `application/x-www-form-urlencoded` would encode a space as `+`; AWS wants `%20`, and a
/// mismatch here produces a signature failure rather than a decoding error.
fn percent_encode(value: &str) -> String {
  let mut encoded = String::with_capacity(value.len());
  for byte in value.as_bytes() {
    match byte {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
        encoded.push(*byte as char)
      }
      _ => encoded.push_str(&format!("%{byte:02X}")),
    }
  }
  encoded
}

fn encode_form(params: &[(String, String)]) -> String {
  params
    .iter()
    .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
    .collect::<Vec<_>>()
    .join("&")
}

fn push_tag_params(params: &mut Vec<(String, String)>, prefix: &str, tags: &[(String, String)]) {
  for (index, (key, value)) in tags.iter().enumerate() {
    params.push((format!("{prefix}.Tag.{}.Key", index + 1), key.clone()));
    params.push((format!("{prefix}.Tag.{}.Value", index + 1), value.clone()));
  }
}

/// Text of the first `<tag>` element, or `None`.
///
/// EC2 response documents are shallow and consistently named, so a targeted scan is enough and
/// avoids modelling a full schema per action. Values are unescaped, since they are untrusted input
/// that may legitimately contain `&` or `<` (AGENTS.md §6).
pub(crate) fn first_tag(xml: &str, tag: &str) -> Option<String> {
  all_tags(xml, tag).into_iter().next()
}

pub(crate) fn all_tags(xml: &str, tag: &str) -> Vec<String> {
  let open = format!("<{tag}>");
  let close = format!("</{tag}>");
  let mut found = Vec::new();
  let mut rest = xml;

  while let Some(start) = rest.find(&open) {
    let after_open = &rest[start + open.len()..];
    let Some(end) = after_open.find(&close) else {
      break;
    };
    found.push(unescape_xml(&after_open[..end]));
    rest = &after_open[end + close.len()..];
  }

  found
}

fn unescape_xml(value: &str) -> String {
  value
    .replace("&lt;", "<")
    .replace("&gt;", ">")
    .replace("&quot;", "\"")
    .replace("&apos;", "'")
    // Ampersand last, so an escaped sequence is not double-decoded.
    .replace("&amp;", "&")
}

fn extract_error_code(body: &str) -> Option<String> {
  first_tag(body, "Code").or_else(|| {
    serde_json::from_str::<serde_json::Value>(body)
      .ok()
      .and_then(|value| {
        value
          .get("__type")
          .or_else(|| value.get("code"))
          .and_then(|found| found.as_str().map(|text| text.to_string()))
      })
  })
}

fn extract_error_message(body: &str) -> Option<String> {
  first_tag(body, "Message").or_else(|| {
    serde_json::from_str::<serde_json::Value>(body)
      .ok()
      .and_then(|value| {
        value
          .get("message")
          .or_else(|| value.get("Message"))
          .and_then(|found| found.as_str().map(|text| text.to_string()))
      })
  })
}

fn truncate(value: &str, limit: usize) -> String {
  let trimmed = value.trim();
  if trimmed.chars().count() <= limit {
    return trimmed.to_string();
  }
  trimmed.chars().take(limit).collect()
}

/// Pull the first instance out of a `RunInstances` or `DescribeInstances` document.
///
/// Both nest an `<instancesSet>` of `<item>` elements; the fields read here sit at the same names
/// in each, so one parser serves both.
pub(crate) fn parse_first_instance(xml: &str) -> Option<Ec2Instance> {
  let instance_id = first_tag(xml, "instanceId")?;

  // `<instanceState>` wraps its own `<name>`, which is the state we want — not the top-level
  // `<name>` that some documents also carry.
  let state = first_tag(xml, "instanceState")
    .and_then(|section| first_tag(&section, "name"))
    .or_else(|| {
      xml
        .find("<instanceState>")
        .and_then(|start| first_tag(&xml[start..], "name"))
    })
    .unwrap_or_else(|| "unknown".to_string());

  let public_ip = first_tag(xml, "ipAddress").filter(|value| !value.trim().is_empty());

  Some(Ec2Instance {
    instance_id,
    state,
    public_ip,
  })
}

/// Whether a failure body is AWS saying "slow down".
fn is_throttling_body(body: &str) -> bool {
  let code = extract_error_code(body).unwrap_or_default();
  matches!(
    code.as_str(),
    "RequestLimitExceeded" | "Throttling" | "ThrottlingException" | "TooManyRequestsException"
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn form_encoding_uses_percent_twenty_for_spaces() {
    // `+` here would break the SigV4 signature rather than the request body.
    let encoded = encode_form(&[("Description".to_string(), "GoatStare WireGuard".to_string())]);
    assert_eq!(encoded, "Description=GoatStare%20WireGuard");
  }

  #[test]
  fn form_encoding_escapes_reserved_characters() {
    let encoded = encode_form(&[("Name".to_string(), "a/b+c=d&e".to_string())]);
    assert_eq!(encoded, "Name=a%2Fb%2Bc%3Dd%26e");
  }

  #[test]
  fn run_instances_response_yields_the_instance_id_and_pending_state() {
    let xml = r#"<RunInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
      <instancesSet><item>
        <instanceId>i-0abc123def4567890</instanceId>
        <instanceState><code>0</code><name>pending</name></instanceState>
      </item></instancesSet>
    </RunInstancesResponse>"#;

    let instance = parse_first_instance(xml).expect("an instance should parse");
    assert_eq!(instance.instance_id, "i-0abc123def4567890");
    assert_eq!(instance.state, "pending");
    assert_eq!(instance.public_ip, None);
  }

  #[test]
  fn a_running_instance_reports_its_public_address() {
    let xml = r#"<DescribeInstancesResponse>
      <reservationSet><item><instancesSet><item>
        <instanceId>i-0abc123def4567890</instanceId>
        <instanceState><code>16</code><name>running</name></instanceState>
        <ipAddress>203.0.113.25</ipAddress>
      </item></instancesSet></item></reservationSet>
    </DescribeInstancesResponse>"#;

    let instance = parse_first_instance(xml).expect("an instance should parse");
    assert_eq!(instance.state, "running");
    assert_eq!(instance.public_ip.as_deref(), Some("203.0.113.25"));
  }

  /// An empty `DescribeInstances` result must not invent an instance.
  #[test]
  fn an_empty_reservation_set_parses_to_nothing() {
    let xml = r#"<DescribeInstancesResponse><reservationSet/></DescribeInstancesResponse>"#;
    assert_eq!(parse_first_instance(xml), None);
  }

  #[test]
  fn every_region_name_is_collected() {
    let xml = r#"<DescribeRegionsResponse><regionInfo>
      <item><regionName>us-east-1</regionName></item>
      <item><regionName>eu-west-2</regionName></item>
    </regionInfo></DescribeRegionsResponse>"#;

    assert_eq!(all_tags(xml, "regionName"), vec!["us-east-1", "eu-west-2"]);
  }

  #[test]
  fn xml_entities_in_values_are_unescaped_once() {
    let xml = "<Message>rate &amp;lt; limit &amp; retry</Message>";
    assert_eq!(
      first_tag(xml, "Message").as_deref(),
      Some("rate &lt; limit & retry")
    );
  }

  /// A bad key must be reported as a credential problem, so the user is told to re-enter it rather
  /// than to retry.
  #[test]
  fn an_auth_failure_is_classified_as_unauthorized() {
    let body = r#"<Response><Errors><Error><Code>AuthFailure</Code>
      <Message>AWS was not able to validate the provided access credentials</Message>
    </Error></Errors></Response>"#;

    let error = Ec2ApiClient::classify_failure("do a thing", StatusCode::BAD_REQUEST, body);
    assert!(matches!(error, Ec2Error::Unauthorized { .. }), "got {error:?}");
  }

  /// A security group that is still attached must be distinguishable, because the response is to
  /// wait and retry rather than to fail the destroy.
  #[test]
  fn a_dependency_violation_is_classified_separately() {
    let body = r#"<Response><Errors><Error><Code>DependencyViolation</Code>
      <Message>resource sg-123 has a dependent object</Message>
    </Error></Errors></Response>"#;

    let error = Ec2ApiClient::classify_failure("delete a group", StatusCode::BAD_REQUEST, body);
    assert!(matches!(error, Ec2Error::DependencyViolation { .. }), "got {error:?}");
  }

  #[test]
  fn a_missing_resource_is_classified_as_not_found() {
    let body = r#"<Response><Errors><Error><Code>InvalidInstanceID.NotFound</Code>
      <Message>The instance ID 'i-0' does not exist</Message></Error></Errors></Response>"#;

    let error = Ec2ApiClient::classify_failure("read state", StatusCode::BAD_REQUEST, body);
    assert!(matches!(error, Ec2Error::NotFound { .. }), "got {error:?}");
  }

  /// Throttling arrives as a 400 with a code in the body, so the status rule alone would not catch
  /// it and the call would fail instead of backing off.
  #[test]
  fn throttling_is_detected_from_the_body_not_the_status() {
    let body = r#"<Response><Errors><Error><Code>RequestLimitExceeded</Code>
      <Message>Request limit exceeded.</Message></Error></Errors></Response>"#;

    assert!(is_throttling_body(body));
    assert!(!is_throttling_body(
      r#"<Response><Errors><Error><Code>AuthFailure</Code></Error></Errors></Response>"#
    ));
  }

  #[test]
  fn tag_parameters_are_numbered_from_one() {
    let mut params = Vec::new();
    push_tag_params(
      &mut params,
      "TagSpecification.1",
      &[
        ("Name".to_string(), "node".to_string()),
        ("goatstare".to_string(), "true".to_string()),
      ],
    );

    assert_eq!(params[0].0, "TagSpecification.1.Tag.1.Key");
    assert_eq!(params[1].1, "node");
    assert_eq!(params[2].0, "TagSpecification.1.Tag.2.Key");
  }
}
