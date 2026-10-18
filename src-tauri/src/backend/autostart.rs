//! Launch at sign-in: an entry under the current user's `Run` key that starts Blink, hidden in the
//! tray, when the user signs in to Windows.
//!
//! The registry is the source of truth, not SQLite. Task Manager's Startup apps page can disable the
//! entry without touching the `Run` key — it writes a flag under `Explorer\StartupApproved\Run`
//! instead — so a copy in the database would drift the first time the user used it. Reading both
//! keys gives the answer Windows itself will act on at the next sign-in.
//!
//! Everything here is HKCU, so no elevation is ever needed (§4: elevate for connect and disconnect
//! only).
//!
//! Nothing is connected at launch. A tunnel left connected is an auto-start service that comes back
//! with Windows on its own, and `reconcile_startup` adopts it; connecting from here would mean a UAC
//! prompt the user did not ask for.

use std::path::Path;

use thiserror::Error;
use windows_sys::Win32::Foundation::{
  ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR,
};
use windows_sys::Win32::System::Registry::{
  RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_ANY,
};

/// Passed on the command line Windows runs at sign-in. The app starts hidden when it sees it.
pub const AUTOSTART_ARG: &str = "--autostart";

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_APPROVED_KEY: &str =
  r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const VALUE_NAME: &str = "Blink";

/// A value that keeps growing between the size query and the read is retried this many times.
const MAX_READ_ATTEMPTS: usize = 4;

#[derive(Debug, Error)]
pub enum AutostartError {
  #[error("Windows refused to {action} Blink's startup entry (error {code}).")]
  Registry { action: &'static str, code: WIN32_ERROR },
  #[error("Blink could not find the path of its own program file: {0}")]
  CurrentExe(#[source] std::io::Error),
}

/// Where the entry lives. Fixed in production; tests point it at a scratch key.
struct Location<'a> {
  run_key: &'a str,
  approved_key: &'a str,
  value_name: &'a str,
}

const SYSTEM: Location<'static> = Location {
  run_key: RUN_KEY,
  approved_key: STARTUP_APPROVED_KEY,
  value_name: VALUE_NAME,
};

/// Whether Windows will start Blink at the next sign-in.
pub fn is_enabled() -> Result<bool, AutostartError> {
  read_state(&SYSTEM)
}

/// Adds or removes the startup entry. Turning it on also clears a Task Manager "disabled" flag, so
/// the switch in Settings always wins over an earlier choice made elsewhere.
pub fn set_enabled(enabled: bool) -> Result<(), AutostartError> {
  let exe = std::env::current_exe().map_err(AutostartError::CurrentExe)?;
  write_state(&SYSTEM, enabled, &exe)
}

/// Points an existing entry at this executable, so it survives an update or a reinstall to another
/// folder. Leaves a missing entry missing and a disabled one disabled. Best-effort: a failure only
/// means the old path stays, and is logged.
pub fn refresh_path() {
  let result = std::env::current_exe()
    .map_err(AutostartError::CurrentExe)
    .and_then(|exe| refresh_location(&SYSTEM, &exe));
  match result {
    Ok(true) => log::info!("event=autostart.path_refreshed"),
    Ok(false) => {}
    Err(err) => log::warn!("event=autostart.refresh_failed error={err}"),
  }
}

/// The `Run` value: the quoted executable path, then the flag that keeps the window hidden.
fn run_command_line(exe: &Path) -> String {
  format!("\"{}\" {AUTOSTART_ARG}", exe.display())
}

/// Task Manager stores a 12-byte value whose first byte is even while the entry is enabled
/// (`0x02`, `0x06`) and odd once the user disables it (`0x03`, `0x07`).
fn approved_marks_disabled(bytes: &[u8]) -> bool {
  bytes.first().is_some_and(|flag| flag & 1 == 1)
}

fn read_state(location: &Location) -> Result<bool, AutostartError> {
  if read_value(location.run_key, location.value_name)?.is_none() {
    return Ok(false);
  }
  let approved = read_value(location.approved_key, location.value_name)?;
  Ok(!approved.is_some_and(|bytes| approved_marks_disabled(&bytes)))
}

fn write_state(location: &Location, enabled: bool, exe: &Path) -> Result<(), AutostartError> {
  if enabled {
    write_string(location.run_key, location.value_name, &run_command_line(exe))?;
  } else {
    delete_value(location.run_key, location.value_name)?;
  }
  delete_value(location.approved_key, location.value_name)
}

/// Returns whether the value was rewritten.
fn refresh_location(location: &Location, exe: &Path) -> Result<bool, AutostartError> {
  let Some(current) = read_value(location.run_key, location.value_name)? else {
    return Ok(false);
  };
  let expected = run_command_line(exe);
  if decode_string(&current) == expected {
    return Ok(false);
  }
  write_string(location.run_key, location.value_name, &expected)?;
  Ok(true)
}

fn wide(value: &str) -> Vec<u16> {
  value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A `REG_SZ` payload: little-endian UTF-16, up to the first NUL.
fn decode_string(bytes: &[u8]) -> String {
  let units: Vec<u16> = bytes
    .chunks_exact(2)
    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
    .take_while(|unit| *unit != 0)
    .collect();
  String::from_utf16_lossy(&units)
}

fn is_missing(status: WIN32_ERROR) -> bool {
  status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND
}

/// The raw bytes of a value under HKCU, or `None` when the value or its key does not exist.
fn read_value(subkey: &str, name: &str) -> Result<Option<Vec<u8>>, AutostartError> {
  let subkey = wide(subkey);
  let name = wide(name);
  let read_failed = |code| AutostartError::Registry { action: "read", code };

  let mut size: u32 = 0;
  // SAFETY: both strings are NUL-terminated and outlive the call; a null data pointer asks only
  // for the size, which is written to `size`.
  let status = unsafe {
    RegGetValueW(
      HKEY_CURRENT_USER,
      subkey.as_ptr(),
      name.as_ptr(),
      RRF_RT_ANY,
      std::ptr::null_mut(),
      std::ptr::null_mut(),
      &mut size,
    )
  };
  if is_missing(status) {
    return Ok(None);
  }
  if status != ERROR_SUCCESS {
    return Err(read_failed(status));
  }

  for _ in 0..MAX_READ_ATTEMPTS {
    if size == 0 {
      return Ok(Some(Vec::new()));
    }
    let mut buffer = vec![0u8; size as usize];
    let mut filled = size;
    // SAFETY: `buffer` holds `filled` writable bytes, and the call writes at most that many.
    let status = unsafe {
      RegGetValueW(
        HKEY_CURRENT_USER,
        subkey.as_ptr(),
        name.as_ptr(),
        RRF_RT_ANY,
        std::ptr::null_mut(),
        buffer.as_mut_ptr().cast(),
        &mut filled,
      )
    };
    match status {
      ERROR_SUCCESS => {
        buffer.truncate(filled as usize);
        return Ok(Some(buffer));
      }
      // Grew since the size query; `filled` now holds the new size.
      ERROR_MORE_DATA => size = filled,
      status if is_missing(status) => return Ok(None),
      status => return Err(read_failed(status)),
    }
  }
  Err(read_failed(ERROR_MORE_DATA))
}

/// Writes a `REG_SZ` value under HKCU, creating the key if it does not exist.
fn write_string(subkey: &str, name: &str, value: &str) -> Result<(), AutostartError> {
  let subkey = wide(subkey);
  let name = wide(name);
  let data = wide(value);
  let byte_len = u32::try_from(data.len() * 2)
    .map_err(|_| AutostartError::Registry { action: "write", code: ERROR_MORE_DATA })?;
  // SAFETY: all three buffers are NUL-terminated and outlive the call; `byte_len` is the exact size
  // of `data` in bytes, terminator included, as REG_SZ requires.
  let status = unsafe {
    RegSetKeyValueW(
      HKEY_CURRENT_USER,
      subkey.as_ptr(),
      name.as_ptr(),
      REG_SZ,
      data.as_ptr().cast(),
      byte_len,
    )
  };
  if status != ERROR_SUCCESS {
    return Err(AutostartError::Registry { action: "write", code: status });
  }
  Ok(())
}

/// Deletes a value under HKCU. A value that is already gone is not an error.
fn delete_value(subkey: &str, name: &str) -> Result<(), AutostartError> {
  let subkey = wide(subkey);
  let name = wide(name);
  // SAFETY: both strings are NUL-terminated and outlive the call.
  let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), name.as_ptr()) };
  if status != ERROR_SUCCESS && !is_missing(status) {
    return Err(AutostartError::Registry { action: "remove", code: status });
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;
  use windows_sys::Win32::System::Registry::{RegDeleteKeyW, RegDeleteTreeW, REG_BINARY};

  /// A throwaway pair of keys under HKCU, removed on every exit path including a failed assertion.
  ///
  /// Each test gets its own top-level key. A shared parent let one test's cleanup delete the
  /// parent while another was writing beneath it (`ERROR_KEY_DELETED`).
  struct ScratchKeys {
    root: String,
    run_key: String,
    approved_key: String,
  }

  impl ScratchKeys {
    fn new() -> Self {
      let root = format!(r"Software\Blink-test-{}", uuid::Uuid::new_v4());
      Self {
        run_key: format!(r"{root}\Run"),
        approved_key: format!(r"{root}\StartupApproved\Run"),
        root,
      }
    }

    fn location(&self) -> Location<'_> {
      Location { run_key: &self.run_key, approved_key: &self.approved_key, value_name: VALUE_NAME }
    }

    fn mark_disabled_in_task_manager(&self) {
      let subkey = wide(&self.approved_key);
      let name = wide(VALUE_NAME);
      let data = [0x03u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
      // SAFETY: NUL-terminated strings and a 12-byte buffer, all outliving the call.
      let status = unsafe {
        RegSetKeyValueW(
          HKEY_CURRENT_USER,
          subkey.as_ptr(),
          name.as_ptr(),
          REG_BINARY,
          data.as_ptr().cast(),
          data.len() as u32,
        )
      };
      assert_eq!(status, ERROR_SUCCESS);
    }
  }

  impl Drop for ScratchKeys {
    fn drop(&mut self) {
      let root = wide(&self.root);
      // SAFETY: a NUL-terminated string outliving the calls. The tree delete empties the key and
      // the key delete removes it; either failing leaves only an empty scratch key behind.
      unsafe {
        RegDeleteTreeW(HKEY_CURRENT_USER, root.as_ptr());
        RegDeleteKeyW(HKEY_CURRENT_USER, root.as_ptr());
      }
    }
  }

  #[test]
  fn command_line_quotes_the_path_and_asks_for_a_hidden_start() {
    let exe = PathBuf::from(r"C:\Program Files\Blink\blink-vpn-app.exe");
    assert_eq!(
      run_command_line(&exe),
      r#""C:\Program Files\Blink\blink-vpn-app.exe" --autostart"#
    );
  }

  #[test]
  fn task_manager_flag_is_disabled_only_when_odd() {
    assert!(!approved_marks_disabled(&[0x02, 0, 0]));
    assert!(!approved_marks_disabled(&[0x06, 0, 0]));
    assert!(approved_marks_disabled(&[0x03, 0, 0]));
    assert!(approved_marks_disabled(&[0x07, 0, 0]));
    assert!(!approved_marks_disabled(&[]));
  }

  #[test]
  fn string_decoding_stops_at_the_terminator() {
    let mut bytes: Vec<u8> = wide("abc").iter().flat_map(|unit| unit.to_le_bytes()).collect();
    bytes.extend_from_slice(&[b'x', 0]);
    assert_eq!(decode_string(&bytes), "abc");
  }

  #[test]
  fn enabling_writes_the_entry_and_disabling_removes_it() {
    let keys = ScratchKeys::new();
    let location = keys.location();
    let exe = PathBuf::from(r"C:\Blink\blink.exe");

    let before = read_state(&location).expect("read absent");
    write_state(&location, true, &exe).expect("enable");
    let enabled = read_state(&location).expect("read enabled");
    let stored = read_value(&keys.run_key, VALUE_NAME).expect("read value").map(|b| decode_string(&b));
    write_state(&location, false, &exe).expect("disable");
    let disabled = read_state(&location).expect("read disabled");
    let disable_again = write_state(&location, false, &exe);

    assert!(!before);
    assert!(enabled);
    assert_eq!(stored.as_deref(), Some(run_command_line(&exe).as_str()));
    assert!(!disabled);
    assert!(disable_again.is_ok(), "removing an absent entry is not an error");
  }

  #[test]
  fn a_task_manager_disable_reads_as_off_until_enabled_again() {
    let keys = ScratchKeys::new();
    let location = keys.location();
    let exe = PathBuf::from(r"C:\Blink\blink.exe");

    write_state(&location, true, &exe).expect("enable");
    keys.mark_disabled_in_task_manager();
    let after_task_manager = read_state(&location).expect("read");
    write_state(&location, true, &exe).expect("enable again");
    let after_reenable = read_state(&location).expect("read");
    let flag_left = read_value(&keys.approved_key, VALUE_NAME).expect("read flag");

    assert!(!after_task_manager);
    assert!(after_reenable);
    assert!(flag_left.is_none(), "enabling clears Task Manager's flag");
  }

  #[test]
  fn refresh_rewrites_a_stale_path_and_leaves_a_missing_entry_alone() {
    let keys = ScratchKeys::new();
    let location = keys.location();
    let old = PathBuf::from(r"C:\Old\blink.exe");
    let new = PathBuf::from(r"D:\New\blink.exe");

    let refreshed_absent = refresh_location(&location, &new).expect("refresh absent");
    let still_absent = read_value(&keys.run_key, VALUE_NAME).expect("read");
    write_state(&location, true, &old).expect("enable");
    let refreshed_stale = refresh_location(&location, &new).expect("refresh stale");
    let refreshed_current = refresh_location(&location, &new).expect("refresh current");
    let stored = read_value(&keys.run_key, VALUE_NAME).expect("read").map(|b| decode_string(&b));

    assert!(!refreshed_absent);
    assert!(still_absent.is_none());
    assert!(refreshed_stale);
    assert!(!refreshed_current);
    assert_eq!(stored.as_deref(), Some(run_command_line(&new).as_str()));
  }
}
