//! App-wide preferences that are not tunnel parameters: how the window behaves, and which mobile
//! devices a new server should come with.
//!
//! Stored in `app_settings` like the tunnel settings, and defaulted in code when unset, so adding
//! one needs no migration.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use thiserror::Error;

use crate::backend::bootstrap::DEFAULT_PEER_SLOTS;
use crate::backend::db::{Database, DbError};
use crate::backend::orchestrator::validate_peer_label;

const CLOSE_TO_TRAY_SETTING_KEY: &str = "close_to_tray";
const MOBILE_DEVICES_SETTING_KEY: &str = "mobile_devices";

/// Every slot after the desktop's can be pre-named, and no more: a server cannot gain slots.
pub const MAX_MOBILE_DEVICES: usize = DEFAULT_PEER_SLOTS - 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppPreferences {
  /// Closing the window hides it to the system tray instead of exiting.
  pub close_to_tray: bool,
  /// Device names given to a new server's spare peer slots, in slot order, so their QR codes are
  /// ready the moment the server is. Servers that already exist are not affected.
  pub mobile_devices: Vec<String>,
}

impl Default for AppPreferences {
  fn default() -> Self {
    Self {
      close_to_tray: true,
      mobile_devices: Vec::new(),
    }
  }
}

#[derive(Debug, Error)]
pub enum PreferencesError {
  #[error("{detail}")]
  Invalid { detail: String },
  #[error(transparent)]
  Database(#[from] DbError),
}

/// Reads the preferences. A key that is unset, or holds a value that does not parse, falls back to
/// its default rather than failing — a corrupt row must not stop the window from closing or a
/// server from being created.
pub fn get(db: &Database) -> Result<AppPreferences, DbError> {
  let defaults = AppPreferences::default();

  let close_to_tray = db
    .get_setting(CLOSE_TO_TRAY_SETTING_KEY)?
    .and_then(|value| parse_bool(&value))
    .unwrap_or(defaults.close_to_tray);

  // Re-validated on read as well as on write, so a row edited by hand cannot hand the create flow
  // a name the claim path would refuse.
  let mobile_devices = db
    .get_setting(MOBILE_DEVICES_SETTING_KEY)?
    .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
    .and_then(|names| normalize_mobile_devices(&names).ok())
    .unwrap_or(defaults.mobile_devices);

  Ok(AppPreferences {
    close_to_tray,
    mobile_devices,
  })
}

/// Validates and stores the preferences. Device names are stored trimmed.
pub fn set(db: &Database, preferences: &AppPreferences) -> Result<AppPreferences, PreferencesError> {
  let mobile_devices = normalize_mobile_devices(&preferences.mobile_devices)?;
  let encoded = serde_json::to_string(&mobile_devices).map_err(|error| PreferencesError::Invalid {
    detail: format!("The device list could not be saved: {error}"),
  })?;

  db.set_setting(
    CLOSE_TO_TRAY_SETTING_KEY,
    Some(if preferences.close_to_tray { "true" } else { "false" }),
  )?;
  db.set_setting(MOBILE_DEVICES_SETTING_KEY, Some(&encoded))?;

  Ok(AppPreferences {
    close_to_tray: preferences.close_to_tray,
    mobile_devices,
  })
}

/// Trims each name and applies the device-name rule, plus the two rules a list adds: it fits in the
/// spare slots, and no two names collide (ignoring case), since a list of identical names cannot be
/// told apart on the dashboard.
fn normalize_mobile_devices(names: &[String]) -> Result<Vec<String>, PreferencesError> {
  if names.len() > MAX_MOBILE_DEVICES {
    return Err(PreferencesError::Invalid {
      detail: format!("A server has room for {MAX_MOBILE_DEVICES} devices besides this PC."),
    });
  }

  let mut seen = HashSet::new();
  let mut normalized = Vec::with_capacity(names.len());

  for name in names {
    let name = validate_peer_label(name).map_err(|error| PreferencesError::Invalid {
      detail: error.to_string(),
    })?;

    if !seen.insert(name.to_lowercase()) {
      return Err(PreferencesError::Invalid {
        detail: format!("\"{name}\" is in the list twice. Give each device its own name."),
      });
    }

    normalized.push(name.to_string());
  }

  Ok(normalized)
}

fn parse_bool(value: &str) -> Option<bool> {
  match value.trim() {
    "true" => Some(true),
    "false" => Some(false),
    _ => None,
  }
}

/// In-memory copy of `close_to_tray`, read by the window close handler.
///
/// That handler runs on the event-loop thread, which must not wait on SQLite. The command that
/// writes the setting updates this copy in the same call, so the two cannot drift.
#[derive(Debug)]
pub struct CloseToTray(AtomicBool);

impl CloseToTray {
  pub fn new(enabled: bool) -> Self {
    Self(AtomicBool::new(enabled))
  }

  pub fn enabled(&self) -> bool {
    self.0.load(Ordering::Relaxed)
  }

  pub fn set(&self, enabled: bool) {
    self.0.store(enabled, Ordering::Relaxed);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::backend::orchestrator::MAX_PEER_LABEL_CHARS;
  use std::path::PathBuf;
  use std::time::{SystemTime, UNIX_EPOCH};

  fn preferences_db(label: &str) -> Database {
    let mut path: PathBuf = std::env::temp_dir();
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("clock must be after unix epoch")
      .as_nanos();
    path.push(format!("blink_preferences_{label}_{nanos}.sqlite3"));
    let db = Database::new(path);
    db.run_migrations().expect("migrations should succeed");
    db
  }

  fn with_devices(names: &[&str]) -> AppPreferences {
    AppPreferences {
      close_to_tray: true,
      mobile_devices: names.iter().map(|name| name.to_string()).collect(),
    }
  }

  #[test]
  fn defaults_when_unset() {
    let db = preferences_db("defaults");
    assert_eq!(get(&db).expect("read should succeed"), AppPreferences::default());
  }

  #[test]
  fn preferences_round_trip() {
    let db = preferences_db("roundtrip");

    let stored = set(
      &db,
      &AppPreferences {
        close_to_tray: false,
        mobile_devices: vec!["  Pixel 9 ".to_string(), "iPad".to_string()],
      },
    )
    .expect("write should succeed");
    assert_eq!(stored.mobile_devices, vec!["Pixel 9", "iPad"], "names are stored trimmed");

    let read = get(&db).expect("read should succeed");
    assert_eq!(read, stored);

    set(&db, &AppPreferences::default()).expect("write should succeed");
    assert_eq!(get(&db).expect("read should succeed"), AppPreferences::default());
  }

  #[test]
  fn corrupt_values_fall_back_to_defaults() {
    let db = preferences_db("corrupt");
    db.set_setting(CLOSE_TO_TRAY_SETTING_KEY, Some("maybe")).expect("write should succeed");
    db.set_setting(MOBILE_DEVICES_SETTING_KEY, Some("not json")).expect("write should succeed");
    assert_eq!(get(&db).expect("read should succeed"), AppPreferences::default());

    // Valid JSON holding a name the claim path would refuse is just as unusable.
    db.set_setting(MOBILE_DEVICES_SETTING_KEY, Some(r#"["   "]"#)).expect("write should succeed");
    assert!(get(&db).expect("read should succeed").mobile_devices.is_empty());
  }

  #[test]
  fn rejects_a_device_list_that_cannot_be_applied() {
    let db = preferences_db("invalid");

    let too_many: Vec<String> = (0..=MAX_MOBILE_DEVICES).map(|index| format!("Phone {index}")).collect();
    let long_name = "n".repeat(MAX_PEER_LABEL_CHARS + 1);
    let cases = [
      AppPreferences { close_to_tray: true, mobile_devices: too_many },
      with_devices(&["Pixel 9", "   "]),
      with_devices(&[&long_name]),
      with_devices(&["Pixel 9", "pixel 9 "]),
    ];

    for case in cases {
      let error = set(&db, &case).expect_err("an unusable list must be refused");
      assert!(matches!(error, PreferencesError::Invalid { .. }), "got: {error:?}");
    }

    // A refused write stores nothing.
    assert_eq!(get(&db).expect("read should succeed"), AppPreferences::default());
  }

  #[test]
  fn close_to_tray_cache_tracks_writes() {
    let cache = CloseToTray::new(true);
    assert!(cache.enabled());
    cache.set(false);
    assert!(!cache.enabled());
  }
}
