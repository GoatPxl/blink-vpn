//! Desktop-shell preferences: how the window behaves, as opposed to what a tunnel does.
//!
//! Stored in `app_settings` like the tunnel settings, and defaulted in code when unset, so adding
//! one needs no migration.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::backend::db::{Database, DbError};

const CLOSE_TO_TRAY_SETTING_KEY: &str = "close_to_tray";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AppPreferences {
  /// Closing the window hides it to the system tray instead of exiting.
  pub close_to_tray: bool,
}

impl Default for AppPreferences {
  fn default() -> Self {
    Self { close_to_tray: true }
  }
}

/// Reads the preferences. A key that is unset, or holds a value that does not parse, falls back to
/// its default rather than failing — a corrupt row must not stop the window from closing.
pub fn get(db: &Database) -> Result<AppPreferences, DbError> {
  let defaults = AppPreferences::default();
  let close_to_tray = db
    .get_setting(CLOSE_TO_TRAY_SETTING_KEY)?
    .and_then(|value| parse_bool(&value))
    .unwrap_or(defaults.close_to_tray);

  Ok(AppPreferences { close_to_tray })
}

pub fn set(db: &Database, preferences: &AppPreferences) -> Result<(), DbError> {
  let value = if preferences.close_to_tray { "true" } else { "false" };
  db.set_setting(CLOSE_TO_TRAY_SETTING_KEY, Some(value))
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
  use std::path::PathBuf;
  use std::time::{SystemTime, UNIX_EPOCH};

  fn preferences_db(label: &str) -> Database {
    let mut path: PathBuf = std::env::temp_dir();
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("clock must be after unix epoch")
      .as_nanos();
    path.push(format!("goatstare_preferences_{label}_{nanos}.sqlite3"));
    let db = Database::new(path);
    db.run_migrations().expect("migrations should succeed");
    db
  }

  #[test]
  fn close_to_tray_defaults_on_when_unset() {
    let db = preferences_db("defaults");
    assert!(get(&db).expect("read should succeed").close_to_tray);
  }

  #[test]
  fn preferences_round_trip() {
    let db = preferences_db("roundtrip");

    set(&db, &AppPreferences { close_to_tray: false }).expect("write should succeed");
    assert!(!get(&db).expect("read should succeed").close_to_tray);

    set(&db, &AppPreferences { close_to_tray: true }).expect("write should succeed");
    assert!(get(&db).expect("read should succeed").close_to_tray);
  }

  #[test]
  fn corrupt_value_falls_back_to_default() {
    let db = preferences_db("corrupt");
    db.set_setting(CLOSE_TO_TRAY_SETTING_KEY, Some("maybe")).expect("write should succeed");
    assert!(get(&db).expect("read should succeed").close_to_tray);
  }

  #[test]
  fn close_to_tray_cache_tracks_writes() {
    let cache = CloseToTray::new(true);
    assert!(cache.enabled());
    cache.set(false);
    assert!(!cache.enabled());
  }
}
