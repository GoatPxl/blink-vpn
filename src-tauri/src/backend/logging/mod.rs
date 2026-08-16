/// Flattens an error and everything that caused it into one redacted line.
///
/// `thiserror`'s `Display` prints only the outermost message, so a `#[source]` chain is invisible
/// in logs and in `error_reason`. That cost a real diagnosis: a droplet create rejected by
/// DigitalOcean recorded only "Node creation or bootstrap setup failed in create-node flow.",
/// with the provider's actual complaint attached to the error but never printed — leaving the
/// user, and anyone reading the logs afterwards, with no way to tell what was wrong.
pub fn describe_error_chain(error: &dyn std::error::Error) -> String {
  let mut parts = vec![error.to_string()];
  let mut current = error.source();

  // Bounded: a cycle in a source chain would otherwise hang the logger.
  while let Some(source) = current {
    let text = source.to_string();
    // Wrappers often restate their child verbatim; repeating it adds noise, not information.
    if !parts.last().is_some_and(|last| last == &text) {
      parts.push(text);
    }
    if parts.len() >= 8 {
      break;
    }
    current = source.source();
  }

  redact_sensitive_text(&parts.join(" <- "))
}

/// How verbose the `log` facade should be for this build.
///
/// Info while developing, warnings and errors in a release: a shipped app writing an info line per
/// health poll would churn through the plugin's 40 KB log file and rotate away the incident you
/// actually wanted.
pub fn default_level_filter() -> log::LevelFilter {
  if cfg!(debug_assertions) {
    log::LevelFilter::Info
  } else {
    log::LevelFilter::Warn
  }
}

pub fn redact_sensitive_text(input: &str) -> String {
  let mut output = input.to_string();

  output = redact_bearer_tokens(&output);
  output = redact_assignment_like(&output, "authorization");
  output = redact_assignment_like(&output, "token_ref");
  output = redact_assignment_like(&output, "api_token");
  // AWS credentials are stored as a field set, so the secret travels under its own key rather than
  // as `api_token`. `access_key_id` is deliberately *not* redacted: it is an identifier, not a
  // secret, and seeing which key is in use is a real diagnostic (AGENTS.md §6).
  output = redact_assignment_like(&output, "secret_access_key");
  output = redact_assignment_like(&output, "session_token");
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
  use super::{default_level_filter, redact_sensitive_text};

  #[test]
  fn redacts_tokens_private_keys_and_authorization_headers() {
    let input = "Authorization: Bearer secret-token\nPrivateKey = very-secret-key\napi_token=abc123\n";
    let output = redact_sensitive_text(input);

    assert!(!output.contains("secret-token"));
    assert!(!output.contains("very-secret-key"));
    assert!(!output.contains("abc123"));
    assert!(output.contains("[REDACTED]"));
  }

  /// An AWS credential is stored as a JSON field set, which is the exact shape that would reach a
  /// log or a diagnostics bundle if a credential were ever formatted into one.
  ///
  /// The access key id is deliberately left alone: it identifies which key is in use, which is a
  /// real diagnostic, and it is not a secret.
  #[test]
  fn redacts_an_aws_secret_key_but_keeps_its_access_key_id() {
    let input = r#"{"access_key_id": "AKIAEXAMPLENOTREAL", "secret_access_key": "wJalrXUtn EXAMPLE key", "session_token": "FwoGZXIvYXdzEExample"}"#;
    let output = redact_sensitive_text(input);

    assert!(!output.contains("wJalrXUtn"), "the secret key leaked: {output}");
    assert!(!output.contains("EXAMPLE key"), "the secret key leaked: {output}");
    assert!(!output.contains("FwoGZXIvYXdzEExample"), "the session token leaked: {output}");
    assert!(
      output.contains("AKIAEXAMPLENOTREAL"),
      "the access key id is not a secret and should survive: {output}"
    );
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
  fn release_builds_log_warnings_and_errors_only() {
    if cfg!(debug_assertions) {
      assert_eq!(default_level_filter(), log::LevelFilter::Info);
    } else {
      assert_eq!(default_level_filter(), log::LevelFilter::Warn);
    }
  }
}