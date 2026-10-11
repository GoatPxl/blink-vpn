//! App-wide preferences that are not tunnel parameters: how the window behaves, which mobile
//! devices a new server should come with, and small pieces of interface state that must survive a
//! restart (favourite locations, whether the first connect has been explained, the game's score).
//!
//! Stored in `app_settings` like the tunnel settings, and defaulted in code when unset, so adding
//! one needs no migration.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::backend::bootstrap::DEFAULT_PEER_SLOTS;
use crate::backend::db::{Database, DbError};
use crate::backend::orchestrator::validate_peer_label;

const CLOSE_TO_TRAY_SETTING_KEY: &str = "close_to_tray";
const MOBILE_DEVICES_SETTING_KEY: &str = "mobile_devices";
const AUTO_CONNECT_SETTING_KEY: &str = "auto_connect_after_provision";
const FIRST_CONNECT_DONE_SETTING_KEY: &str = "first_connect_done";
const EXPLAIN_UAC_SETTING_KEY: &str = "explain_uac";
const FAVOURITES_SETTING_KEY: &str = "favourites";
const GAME_HIGH_SCORE_SETTING_KEY: &str = "game_high_score";

/// More favourites than any map can show; the cap only stops a runaway list.
pub const MAX_FAVOURITES: usize = 64;
/// `<provider_id>:<region>`. Provider ids and region slugs are both short.
const MAX_FAVOURITE_CHARS: usize = 160;

/// Every slot after the desktop's can be pre-named, and no more: a server cannot gain slots.
pub const MAX_MOBILE_DEVICES: usize = DEFAULT_PEER_SLOTS - 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPreferences {
  /// Closing the window hides it to the system tray instead of exiting.
  pub close_to_tray: bool,
  /// Device names given to a new server's spare peer slots, in slot order, so their QR codes are
  /// ready the moment the server is. Servers that already exist are not affected.
  pub mobile_devices: Vec<String>,
  /// Connect as soon as a server the user built reaches ready, instead of waiting for a second
  /// click. Still exactly one UAC prompt: the connect is the same command either way.
  pub auto_connect_after_provision: bool,
  /// Set after the first connect that reached a working tunnel. Until then the HOLD ON screen
  /// explains each step, since the user is about to lose their internet for ~15 s.
  pub first_connect_done: bool,
  /// Show the "one Windows prompt is coming" explainer before a connect.
  pub explain_uac: bool,
  /// Favourite locations as `<provider_id>:<region>` keys, in the order they were added.
  pub favourites: Vec<String>,
  /// Best score in the build-screen game. Local only.
  pub game_high_score: u32,
}

impl Default for AppPreferences {
  fn default() -> Self {
    Self {
      close_to_tray: true,
      mobile_devices: Vec::new(),
      auto_connect_after_provision: true,
      first_connect_done: false,
      explain_uac: true,
      favourites: Vec::new(),
      game_high_score: 0,
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

  let flag = |key: &str, default: bool| -> Result<bool, DbError> {
    Ok(db.get_setting(key)?.and_then(|value| parse_bool(&value)).unwrap_or(default))
  };

  let favourites = db
    .get_setting(FAVOURITES_SETTING_KEY)?
    .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
    .and_then(|keys| normalize_favourites(&keys).ok())
    .unwrap_or(defaults.favourites);

  let game_high_score = db
    .get_setting(GAME_HIGH_SCORE_SETTING_KEY)?
    .and_then(|value| value.trim().parse::<u32>().ok())
    .unwrap_or(defaults.game_high_score);

  Ok(AppPreferences {
    close_to_tray,
    mobile_devices,
    auto_connect_after_provision: flag(AUTO_CONNECT_SETTING_KEY, defaults.auto_connect_after_provision)?,
    first_connect_done: flag(FIRST_CONNECT_DONE_SETTING_KEY, defaults.first_connect_done)?,
    explain_uac: flag(EXPLAIN_UAC_SETTING_KEY, defaults.explain_uac)?,
    favourites,
    game_high_score,
  })
}

/// Validates and stores the preferences. Device names are stored trimmed.
///
/// Everything is validated before anything is written, so a refused write stores nothing.
pub fn set(db: &Database, preferences: &AppPreferences) -> Result<AppPreferences, PreferencesError> {
  let mobile_devices = normalize_mobile_devices(&preferences.mobile_devices)?;
  let favourites = normalize_favourites(&preferences.favourites)?;
  let encode = |what: &str, value: &Vec<String>| {
    serde_json::to_string(value).map_err(|error| PreferencesError::Invalid {
      detail: format!("The {what} could not be saved: {error}"),
    })
  };
  let encoded_devices = encode("device list", &mobile_devices)?;
  let encoded_favourites = encode("favourites", &favourites)?;
  let as_flag = |value: bool| Some(if value { "true" } else { "false" });

  db.set_setting(CLOSE_TO_TRAY_SETTING_KEY, as_flag(preferences.close_to_tray))?;
  db.set_setting(MOBILE_DEVICES_SETTING_KEY, Some(&encoded_devices))?;
  db.set_setting(AUTO_CONNECT_SETTING_KEY, as_flag(preferences.auto_connect_after_provision))?;
  db.set_setting(FIRST_CONNECT_DONE_SETTING_KEY, as_flag(preferences.first_connect_done))?;
  db.set_setting(EXPLAIN_UAC_SETTING_KEY, as_flag(preferences.explain_uac))?;
  db.set_setting(FAVOURITES_SETTING_KEY, Some(&encoded_favourites))?;
  db.set_setting(GAME_HIGH_SCORE_SETTING_KEY, Some(&preferences.game_high_score.to_string()))?;

  Ok(AppPreferences {
    mobile_devices,
    favourites,
    ..preferences.clone()
  })
}

/// Trims each key, drops duplicates (keeping the first), and refuses anything that is not a
/// `<provider_id>:<region>` pair of sane length.
fn normalize_favourites(keys: &[String]) -> Result<Vec<String>, PreferencesError> {
  if keys.len() > MAX_FAVOURITES {
    return Err(PreferencesError::Invalid {
      detail: format!("At most {MAX_FAVOURITES} favourite locations can be saved."),
    });
  }

  let mut seen = HashSet::new();
  let mut normalized = Vec::with_capacity(keys.len());
  for key in keys {
    let key = key.trim();
    let well_formed = key.len() <= MAX_FAVOURITE_CHARS
      && key
        .split_once(':')
        .is_some_and(|(provider, region)| !provider.is_empty() && !region.is_empty());
    if !well_formed {
      return Err(PreferencesError::Invalid {
        detail: format!("\"{key}\" is not a location. Favourites are saved as provider:region."),
      });
    }
    if seen.insert(key.to_string()) {
      normalized.push(key.to_string());
    }
  }
  Ok(normalized)
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
      mobile_devices: names.iter().map(|name| name.to_string()).collect(),
      ..AppPreferences::default()
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
        auto_connect_after_provision: false,
        first_connect_done: true,
        explain_uac: false,
        favourites: vec![
          " provider-1:fra1 ".to_string(),
          "provider-1:fra1".to_string(),
          "provider-2:us-east".to_string(),
        ],
        game_high_score: 8810,
      },
    )
    .expect("write should succeed");
    assert_eq!(stored.mobile_devices, vec!["Pixel 9", "iPad"], "names are stored trimmed");
    assert_eq!(
      stored.favourites,
      vec!["provider-1:fra1", "provider-2:us-east"],
      "favourites are trimmed and deduplicated"
    );

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
    db.set_setting(AUTO_CONNECT_SETTING_KEY, Some("yes")).expect("write should succeed");
    db.set_setting(FAVOURITES_SETTING_KEY, Some(r#"["no-colon"]"#)).expect("write should succeed");
    db.set_setting(GAME_HIGH_SCORE_SETTING_KEY, Some("-4")).expect("write should succeed");
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
      AppPreferences { mobile_devices: too_many, ..AppPreferences::default() },
      with_devices(&["Pixel 9", "   "]),
      with_devices(&[&long_name]),
      with_devices(&["Pixel 9", "pixel 9 "]),
      AppPreferences { favourites: vec!["fra1".to_string()], ..AppPreferences::default() },
      AppPreferences { favourites: vec![":fra1".to_string()], ..AppPreferences::default() },
      AppPreferences {
        favourites: (0..=MAX_FAVOURITES).map(|index| format!("p:{index}")).collect(),
        ..AppPreferences::default()
      },
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
