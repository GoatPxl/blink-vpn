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
  // Google's credential is a whole service account key file, so the blob needs covering, as does
  // the signed JWT sent to the token endpoint. `client_email` and `project_id` are deliberately
  // left readable for the same reason as `access_key_id`: they name which credential is in use,
  // which is a diagnostic rather than a secret.
  output = redact_assignment_like(&output, "service_account_json");
  output = redact_assignment_like(&output, "assertion");
  // OpenStack: the application credential's secret, and the Keystone token issued for it. The
  // auth URL and the credential id stay readable — they name which cloud and which key are in
  // use, which is the same diagnostic `access_key_id` provides.
  output = redact_assignment_like(&output, "application_credential_secret");
  output = redact_assignment_like(&output, "X-Subject-Token");
  output = redact_assignment_like(&output, "x-subject-token");
  // The RSA key inside that file is matched by *content*, not by field name.
  //
  // Google names it `private_key` — the same key WireGuard client configs use for material this
  // app deliberately does not redact (§6). Redacting that name would blank the one line worth
  // reading when a tunnel comes up wrong. The two are unambiguous by shape instead: a service
  // account key is PEM, a WireGuard key is a bare base64 blob and never carries PEM markers.
  output = redact_pem_private_keys(&output);
  // WireGuard key material is deliberately absent from this list. A server lives for hours and is
  // then destroyed, and its keys die with it, so a client config is not a secret here (AGENTS.md
  // §6). Redacting the key blanked the one line most worth reading when a tunnel came up wrong,
  // and a config a phone can import has to carry it.
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

/// Replace the body of any PEM private key block, keeping the markers so the reader can see that
/// something was removed and what kind of thing it was.
///
/// Matched on the `BEGIN`/`END` markers rather than a field name because the same field name
/// (`private_key`) carries WireGuard material this app declassifies on purpose. Handles the
/// escaped form too — a key file stored as a JSON *string* has its newlines written `\n`, and the
/// markers survive that intact.
fn redact_pem_private_keys(input: &str) -> String {
  const BEGIN: &str = "-----BEGIN";
  const END: &str = "-----END";
  const DASHES: &str = "-----";

  let mut output = String::with_capacity(input.len());
  let mut rest = input;

  loop {
    let Some(begin) = rest.find(BEGIN) else {
      output.push_str(rest);
      return output;
    };

    let after_begin = &rest[begin..];
    // The header runs from `-----BEGIN` to the dashes that close it.
    let Some(header_end) = after_begin[BEGIN.len()..]
      .find(DASHES)
      .map(|offset| BEGIN.len() + offset + DASHES.len())
    else {
      output.push_str(rest);
      return output;
    };

    // Only private-key blocks are secrets; `-----BEGIN CERTIFICATE-----` is a real diagnostic.
    if !after_begin[..header_end].contains("PRIVATE KEY") {
      let consumed = begin + header_end;
      output.push_str(&rest[..consumed]);
      rest = &rest[consumed..];
      continue;
    }

    output.push_str(&rest[..begin]);
    output.push_str("[REDACTED PRIVATE KEY]");

    // Search for the terminator *past* the header — `-----BEGIN PRIVATE KEY-----` itself ends in
    // the same dashes, so scanning from the start of the block matches its own opening marker and
    // leaves the key body in the output.
    let body = &after_begin[header_end..];
    let terminated = body.find(END).and_then(|end| {
      body[end + END.len()..]
        .find(DASHES)
        .map(|offset| header_end + end + END.len() + offset + DASHES.len())
    });

    match terminated {
      Some(block_end) => rest = &after_begin[block_end..],
      // An unterminated block is still a key: drop the remainder rather than emitting it.
      None => return output,
    }
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

  /// Provider credentials are redacted; WireGuard key material is not.
  ///
  /// They are different classes of thing. A provider token is long-lived and grants
  /// account-wide access to create billable resources. A WireGuard key belongs to a server
  /// that exists for hours and is then destroyed (AGENTS.md §6).
  #[test]
  fn redacts_provider_credentials_but_keeps_wireguard_key_material() {
    let input = "Authorization: Bearer secret-token\nPrivateKey = very-secret-key\napi_token=abc123\n";
    let output = redact_sensitive_text(input);

    assert!(!output.contains("secret-token"));
    assert!(
      output.contains("very-secret-key"),
      "the WireGuard key is not a secret and must stay readable: {output}"
    );
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

  /// The serialized shape a keypair reaches a diagnostics bundle in. Nothing here is redacted.
  #[test]
  fn keeps_wireguard_key_material_in_a_serialized_document() {
    let input = r#"{"private_key": "aGVsbG8gd29ybGQ=", "listen_port": 51820}"#;
    let output = redact_sensitive_text(input);

    assert!(output.contains("aGVsbG8gd29ybGQ="), "the key must survive: {output}");
    assert!(output.contains("51820"), "unrelated fields must not be swallowed: {output}");
  }

  /// A Google service account key is a long-lived credential that can create billable resources,
  /// so no part of it may reach a log or a diagnostics bundle.
  #[test]
  fn redacts_a_google_service_account_key_but_keeps_its_identity() {
    let input = r#"{"service_account_json": "{\"project_id\": \"blink-test\"}", "client_email": "vpn@blink-test.iam.gserviceaccount.com", "project_id": "blink-test"}"#;
    let output = redact_sensitive_text(input);

    assert!(!output.contains("\\\"project_id\\\""), "the key blob leaked: {output}");
    assert!(output.contains("[REDACTED]"));
    // Which credential is in use is a diagnostic, not a secret — the `access_key_id` precedent.
    assert!(
      output.contains("vpn@blink-test.iam.gserviceaccount.com"),
      "the service account identity should survive: {output}"
    );
    assert!(
      output.contains("\"project_id\": \"blink-test\""),
      "the project id should survive: {output}"
    );
  }

  /// The RSA key inside a service account file, in the escaped form a JSON-serialized key uses.
  #[test]
  fn redacts_a_pem_private_key_in_either_escaped_or_literal_form() {
    let escaped = r#"{"private_key": "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqSECRETBODY\n-----END PRIVATE KEY-----\n"}"#;
    let output = redact_sensitive_text(escaped);
    assert!(!output.contains("SECRETBODY"), "the RSA key leaked: {output}");
    assert!(output.contains("[REDACTED PRIVATE KEY]"), "was: {output}");

    let literal = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKSECRETBODY\n-----END RSA PRIVATE KEY-----";
    let output = redact_sensitive_text(literal);
    assert!(!output.contains("SECRETBODY"), "the RSA key leaked: {output}");
    assert!(output.contains("[REDACTED PRIVATE KEY]"), "was: {output}");
  }

  /// The signed assertion is a bearer credential for the token endpoint until it expires.
  #[test]
  fn redacts_the_google_token_assertion() {
    let input = "grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer&assertion=eyJhbGciOiSECRETJWT";
    let output = redact_sensitive_text(input);

    assert!(!output.contains("SECRETJWT"), "the assertion leaked: {output}");
    assert!(output.contains("[REDACTED]"));
  }

  /// An OpenStack application credential secret authorises creating billable servers until it is
  /// revoked; the Keystone token it yields does so until it expires.
  #[test]
  fn redacts_an_openstack_secret_but_keeps_the_cloud_it_names() {
    let input = r#"{"auth_url": "https://identity.example.com/v3", "application_credential_id": "8ab1cd", "application_credential_secret": "s3cr3t not real"}"#;
    let output = redact_sensitive_text(input);

    assert!(!output.contains("s3cr3t"), "the credential secret leaked: {output}");
    assert!(!output.contains("not real"), "the credential secret leaked: {output}");
    // Which cloud and which credential are diagnostics, not secrets.
    assert!(output.contains("https://identity.example.com/v3"), "the auth URL should survive: {output}");
    assert!(output.contains("8ab1cd"), "the credential id should survive: {output}");

    let header = "X-Subject-Token: gAAAAABlSECRETTOKEN";
    let output = redact_sensitive_text(header);
    assert!(!output.contains("SECRETTOKEN"), "the Keystone token leaked: {output}");
  }

  /// Not every PEM block is a secret, and blanking a certificate would remove a real diagnostic.
  #[test]
  fn leaves_a_non_private_pem_block_alone() {
    let input = "-----BEGIN CERTIFICATE-----\nMIIDPUBLICCERT\n-----END CERTIFICATE-----";
    let output = redact_sensitive_text(input);

    assert!(output.contains("MIIDPUBLICCERT"), "a certificate is not a secret: {output}");
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