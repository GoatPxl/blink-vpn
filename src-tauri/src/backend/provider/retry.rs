use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
  pub max_retries: usize,
  pub base_delay_ms: u64,
  pub max_delay_ms: u64,
  pub jitter_max_ms: u64,
}

impl RetryPolicy {
  pub fn cloud_default() -> Self {
    Self {
      max_retries: 5,
      base_delay_ms: 250,
      max_delay_ms: 5_000,
      jitter_max_ms: 200,
    }
  }

  pub fn backoff_for_attempt(&self, attempt: usize) -> Duration {
    let exponent = attempt.min(16) as u32;
    let exp_multiplier = 1_u64 << exponent;
    let bounded = self
      .base_delay_ms
      .saturating_mul(exp_multiplier)
      .min(self.max_delay_ms);

    let jitter = if self.jitter_max_ms == 0 {
      0
    } else {
      pseudo_jitter_ms(self.jitter_max_ms, attempt)
    };

    Duration::from_millis(bounded.saturating_add(jitter))
  }

  pub fn should_retry_status(&self, status_code: u16) -> bool {
    status_code == 429 || (500..=599).contains(&status_code)
  }
}

fn pseudo_jitter_ms(jitter_max_ms: u64, attempt: usize) -> u64 {
  use std::time::{SystemTime, UNIX_EPOCH};

  let nanos = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|duration| duration.subsec_nanos() as u64)
    .unwrap_or(0);

  let mixed = nanos ^ ((attempt as u64).wrapping_mul(1103515245));
  if jitter_max_ms == 0 {
    0
  } else {
    mixed % (jitter_max_ms + 1)
  }
}

#[cfg(test)]
mod tests {
  use super::RetryPolicy;

  #[test]
  fn retry_policy_identifies_retryable_statuses() {
    let policy = RetryPolicy::cloud_default();
    assert!(policy.should_retry_status(429));
    assert!(policy.should_retry_status(500));
    assert!(policy.should_retry_status(503));
    assert!(!policy.should_retry_status(401));
    assert!(!policy.should_retry_status(404));
  }

  #[test]
  fn retry_policy_backoff_is_capped() {
    let policy = RetryPolicy {
      max_retries: 2,
      base_delay_ms: 250,
      max_delay_ms: 300,
      jitter_max_ms: 0,
    };

    assert_eq!(policy.backoff_for_attempt(0).as_millis(), 250);
    assert_eq!(policy.backoff_for_attempt(1).as_millis(), 300);
    assert_eq!(policy.backoff_for_attempt(8).as_millis(), 300);
  }
}
