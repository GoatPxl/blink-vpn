use std::time::Duration;

use super::provider::retry::RetryPolicy;

#[derive(Debug, Clone)]
pub struct ReliabilityPolicy {
  pub provider_retry: RetryPolicy,
  pub provider_request_timeout: Duration,
  pub droplet_ready_timeout: Duration,
  pub force_cleanup_timeout: Duration,
  pub reconcile_timeout: Duration,
}

impl ReliabilityPolicy {
  pub fn phase1_defaults() -> Self {
    Self {
      provider_retry: RetryPolicy::digitalocean_default(),
      provider_request_timeout: Duration::from_secs(20),
      droplet_ready_timeout: Duration::from_secs(240),
      force_cleanup_timeout: Duration::from_secs(30),
      reconcile_timeout: Duration::from_secs(45),
    }
  }
}