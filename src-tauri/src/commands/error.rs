//! Structured error envelope for every Tauri command.
//!
//! Commands used to return `Result<T, String>`, and half of them formatted the string as
//! `"{message} Next: {next_action}"` while the rest returned a bare `to_string()`. The frontend
//! then recovered the structure by matching on substrings — `"OpenService FAILED 1060"`,
//! `"Provider orchestration failed"`, and so on — so every reworded backend message silently
//! broke UI recovery paths, and half the commands never produced recovery text at all.
//!
//! Errors now cross the IPC boundary with their shape intact: the UI switches on `kind` and
//! displays `message` and `next_action`, and copy edits are just copy edits.

use serde::Serialize;

use crate::backend::db::DbError;
use crate::backend::logging;
use crate::backend::orchestrator::{OrchestratorError, UserFacingErrorKind};
use crate::backend::provider::ProviderFlowError;
use crate::backend::secure_storage::SecretStoreError;
use crate::backend::wireguard::WireGuardError;

/// Machine-readable failure category. The UI branches on this and never on message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandErrorKind {
  /// The requested operation conflicts with current local state.
  InvalidState,
  /// A referenced node, tunnel, or provider does not exist.
  MissingResource,
  /// The provider API rejected or could not service the request.
  Provider,
  /// A local WireGuard operation failed.
  WireGuard,
  /// Local database read/write failure.
  Persistence,
  /// Secure storage (Windows Credential Manager) read/write failure.
  SecretStorage,
  /// Caller supplied something the command cannot accept.
  InvalidInput,
  /// A required local dependency is missing or unusable.
  Dependency,
  Unknown,
}

impl From<UserFacingErrorKind> for CommandErrorKind {
  fn from(kind: UserFacingErrorKind) -> Self {
    match kind {
      UserFacingErrorKind::InvalidState => Self::InvalidState,
      UserFacingErrorKind::InvalidInput => Self::InvalidInput,
      UserFacingErrorKind::MissingResource => Self::MissingResource,
      UserFacingErrorKind::Provider => Self::Provider,
      UserFacingErrorKind::WireGuard => Self::WireGuard,
      UserFacingErrorKind::Persistence => Self::Persistence,
      UserFacingErrorKind::Unknown => Self::Unknown,
    }
  }
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
  pub kind: CommandErrorKind,
  /// What went wrong, in the user's terms. Already redacted.
  pub message: String,
  /// The single next step most likely to resolve it. Never empty.
  pub next_action: String,
}

impl CommandError {
  pub fn new(
    kind: CommandErrorKind,
    message: impl Into<String>,
    next_action: impl Into<String>,
  ) -> Self {
    Self {
      kind,
      message: logging::redact_sensitive_text(&message.into()),
      next_action: next_action.into(),
    }
  }

  pub fn invalid_input(message: impl Into<String>, next_action: impl Into<String>) -> Self {
    Self::new(CommandErrorKind::InvalidInput, message, next_action)
  }

  pub fn dependency(message: impl Into<String>, next_action: impl Into<String>) -> Self {
    Self::new(CommandErrorKind::Dependency, message, next_action)
  }

  pub fn unknown(message: impl Into<String>) -> Self {
    Self::new(
      CommandErrorKind::Unknown,
      message,
      "Retry the action. If it keeps failing, export a diagnostics bundle from the Diagnostics page.",
    )
  }
}

impl From<OrchestratorError> for CommandError {
  fn from(error: OrchestratorError) -> Self {
    let user_facing = error.to_user_facing();
    Self::new(
      user_facing.kind.into(),
      user_facing.message,
      user_facing.next_action,
    )
  }
}

impl From<DbError> for CommandError {
  fn from(error: DbError) -> Self {
    let kind = if matches!(error, DbError::NotFound { .. }) {
      CommandErrorKind::MissingResource
    } else {
      CommandErrorKind::Persistence
    };

    Self::new(
      kind,
      error.to_string(),
      "Refresh the dashboard. If local state still looks stale, use Force Cleanup on the affected node.",
    )
  }
}

impl From<SecretStoreError> for CommandError {
  fn from(error: SecretStoreError) -> Self {
    Self::new(
      CommandErrorKind::SecretStorage,
      error.to_string(),
      "Re-save the credential in Settings. If it keeps failing, check Windows Credential Manager health.",
    )
  }
}

impl From<ProviderFlowError> for CommandError {
  fn from(error: ProviderFlowError) -> Self {
    Self::new(
      CommandErrorKind::Provider,
      error.to_string(),
      "Validate the provider credential in Settings, then retry.",
    )
  }
}

impl From<WireGuardError> for CommandError {
  fn from(error: WireGuardError) -> Self {
    Self::new(
      CommandErrorKind::WireGuard,
      error.to_string(),
      "Confirm WireGuard for Windows is installed, then retry. Approve the elevation prompt if one appears.",
    )
  }
}

impl From<serde_json::Error> for CommandError {
  fn from(error: serde_json::Error) -> Self {
    Self::new(
      CommandErrorKind::Unknown,
      format!("Could not serialize the diagnostics bundle: {error}"),
      "Retry the export. If it keeps failing, report the diagnostics page as unusable.",
    )
  }
}

impl From<tauri::Error> for CommandError {
  fn from(error: tauri::Error) -> Self {
    Self::new(
      CommandErrorKind::Dependency,
      format!("The desktop shell rejected the request: {error}"),
      "Restart GoatStare and retry.",
    )
  }
}

impl From<std::io::Error> for CommandError {
  fn from(error: std::io::Error) -> Self {
    Self::new(
      CommandErrorKind::Dependency,
      format!("A local file operation failed: {error}"),
      "Check that the app data directory is writable, then retry.",
    )
  }
}

/// Fallback for call sites that still surface a plain string.
impl From<String> for CommandError {
  fn from(message: String) -> Self {
    Self::unknown(message)
  }
}

impl From<&str> for CommandError {
  fn from(message: &str) -> Self {
    Self::unknown(message.to_string())
  }
}
