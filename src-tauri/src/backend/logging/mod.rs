#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogVerbosity {
  Dev,
  Prod,
}

#[derive(Clone)]
pub struct Logger {
  verbosity: LogVerbosity,
}

impl Logger {
  pub fn new() -> Self {
    Self {
      verbosity: if cfg!(debug_assertions) {
        LogVerbosity::Dev
      } else {
        LogVerbosity::Prod
      },
    }
  }

  pub fn subsystem_name(&self) -> &'static str {
    "logging"
  }

  pub fn verbosity(&self) -> LogVerbosity {
    self.verbosity
  }

  pub fn level_filter(&self) -> log::LevelFilter {
    match self.verbosity {
      LogVerbosity::Dev => log::LevelFilter::Info,
      LogVerbosity::Prod => log::LevelFilter::Warn,
    }
  }

  pub fn log_structured(
    &self,
    level: log::Level,
    event: &str,
    correlation_id: Option<&str>,
    fields: &[(&str, String)],
  ) {
    let mut message = format!("event={event}");
    if let Some(correlation_id) = correlation_id {
      message.push_str(&format!(" correlation_id={correlation_id}"));
    }

    for (key, value) in fields {
      message.push(' ');
      message.push_str(key);
      message.push('=');
      message.push_str(&redact_sensitive_text(value));
    }

    log::log!(level, "{}", message);
  }

  pub fn redact_sensitive_text(input: &str) -> String {
    redact_sensitive_text(input)
  }

  pub fn sanitize_diagnostics_bundle_text(input: &str) -> String {
    redact_sensitive_text(input)
  }
}

pub fn redact_sensitive_text(input: &str) -> String {
  let mut output = input.to_string();

  output = redact_bearer_tokens(&output);
  output = redact_assignment_like(&output, "authorization");
  output = redact_assignment_like(&output, "token_ref");
  output = redact_assignment_like(&output, "api_token");
  output = redact_assignment_like(&output, "private_key");
  output = redact_assignment_like(&output, "PrivateKey");
  output = redact_assignment_like(&output, "Authorization");
  output = redact_assignment_like(&output, "Token");

  output
}

fn redact_bearer_tokens(input: &str) -> String {
  let mut output = String::with_capacity(input.len());
  for line in input.lines() {
    let lower = line.to_ascii_lowercase();
    if let Some(index) = lower.find("authorization:") {
      let prefix = &line[..index + "authorization:".len()];
      output.push_str(prefix);
      output.push_str(" [REDACTED]");
    } else if let Some(index) = lower.find("bearer ") {
      let prefix = &line[..index + "bearer ".len()];
      output.push_str(prefix);
      output.push_str("[REDACTED]");
    } else {
      output.push_str(line);
    }
    output.push('\n');
  }

  if input.ends_with('\n') {
    output
  } else {
    output.trim_end_matches('\n').to_string()
  }
}

fn redact_assignment_like(input: &str, key: &str) -> String {
  let mut output = input.to_string();
  let patterns = [
    format!("{key} ="),
    format!("{key}="),
    format!("{key}:"),
    format!("\"{key}\":"),
  ];

  for pattern in patterns {
    let mut search_start = 0;
    while let Some(found) = output[search_start..].find(&pattern) {
      let index = search_start + found;
      let value_start = index + pattern.len();
      let value_start = output[value_start..]
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(offset, _)| value_start + offset)
        .unwrap_or(output.len());
      let tail = &output[value_start..];
      let value_end = if tail.starts_with('"') {
        // A quoted value runs to its closing quote. Ending at the first space instead would cut
        // the redaction short and leave the remainder of the secret in the output — which is
        // exactly what happens to JSON-shaped diagnostics whose values contain spaces.
        tail[1..]
          .find('"')
          .map(|offset| value_start + 1 + offset + 1)
          .unwrap_or(output.len())
      } else {
        tail
          .find(['\n', ',', '}', ' '])
          .map(|offset| value_start + offset)
          .unwrap_or(output.len())
      };
      output.replace_range(value_start..value_end, " [REDACTED]");
      search_start = value_start + " [REDACTED]".len();
    }
  }

  output
}

#[cfg(test)]
mod tests {
  use super::{Logger, LogVerbosity, redact_sensitive_text};

  #[test]
  fn redacts_tokens_private_keys_and_authorization_headers() {
    let input = "Authorization: Bearer secret-token\nPrivateKey = very-secret-key\napi_token=abc123\n";
    let output = redact_sensitive_text(input);

    assert!(!output.contains("secret-token"));
    assert!(!output.contains("very-secret-key"));
    assert!(!output.contains("abc123"));
    assert!(output.contains("[REDACTED]"));
  }

  /// A quoted secret containing a space must be redacted whole.
  ///
  /// Terminating the value at the first space left the remainder of the secret in the output,
  /// which is the shape diagnostics bundles are serialized in.
  #[test]
  fn redacts_the_whole_of_a_quoted_value_containing_spaces() {
    let input = r#"{"api_token": "secret with spaces inside", "region": "nyc1"}"#;
    let output = redact_sensitive_text(input);

    assert!(!output.contains("secret"), "output still leaked: {output}");
    assert!(!output.contains("spaces"), "output still leaked: {output}");
    assert!(!output.contains("inside"), "output still leaked: {output}");
    assert!(output.contains("[REDACTED]"));
    // Non-secret fields must survive so the bundle stays useful.
    assert!(output.contains("nyc1"));
  }

  #[test]
  fn redacts_private_key_json_values_without_swallowing_the_rest_of_the_document() {
    let input = r#"{"private_key": "aGVsbG8gd29ybGQ=", "listen_port": 51820}"#;
    let output = redact_sensitive_text(input);

    assert!(!output.contains("aGVsbG8gd29ybGQ="));
    assert!(output.contains("51820"), "unrelated fields must not be swallowed: {output}");
  }

  #[test]
  fn logger_uses_dev_level_in_debug_builds() {
    let logger = Logger::new();
    if cfg!(debug_assertions) {
      assert_eq!(logger.verbosity(), LogVerbosity::Dev);
      assert_eq!(logger.level_filter(), log::LevelFilter::Info);
    } else {
      assert_eq!(logger.verbosity(), LogVerbosity::Prod);
      assert_eq!(logger.level_filter(), log::LevelFilter::Warn);
    }
  }
}