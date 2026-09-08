//! Child process construction.
//!
//! Every child process this app spawns is built here, and nowhere else. See
//! [`no_window_command`] for why that matters; `process_spawns_go_through_helper` in the
//! tests below is what keeps it true.

use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::Command;

/// Win32 `CREATE_NO_WINDOW`.
///
/// Spelled out rather than pulled from `windows-sys`, which would mean enabling the
/// `Win32_System_Threading` feature for a single integer.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Builds a [`Command`] that does not flash a console window.
///
/// Release builds are GUI subsystem (see `main.rs`), so the app owns no console. That
/// attribute suppresses a console for *our own* process only — it does nothing for
/// children. A console-subsystem child (`sc`, `wg`, `wireguard`, `powershell`, `docker`,
/// `cmd`) is handed a brand-new console window by Windows unless `CREATE_NO_WINDOW` is
/// set, and that window appears and vanishes as the child exits.
///
/// Status polling spawns several processes a second — `sc query` at 10 Hz while a tunnel
/// activates, then once a second while the connect is watched — so without this flag the
/// user watches a strobe of black boxes for the entire connect.
///
/// This is invisible in development: debug builds have a console that children inherit
/// instead of allocating their own. Only a release build reproduces it.
pub fn no_window_command(program: impl AsRef<OsStr>) -> Command {
  let mut command = Command::new(program);
  #[cfg(windows)]
  command.creation_flags(CREATE_NO_WINDOW);
  command
}

#[cfg(test)]
mod tests {
  use std::fs;
  use std::path::{Path, PathBuf};

  /// `Command` exposes no getter for its creation flags, so the invariant is asserted at
  /// the source level instead: `Command::new` may appear only in this module.
  ///
  /// A new spawn site that bypasses `no_window_command` reintroduces flashing console
  /// windows that no debug build and no headless test can show you. This is the only thing
  /// that catches it.
  #[test]
  fn process_spawns_go_through_helper() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    collect_offenders(&src, &src, &mut offenders);

    assert!(
      offenders.is_empty(),
      "`Command::new` must only be used in backend/process.rs; use \
       `backend::process::no_window_command` instead so the child does not flash a console \
       window. Offending sites:\n{}",
      offenders.join("\n")
    );
  }

  fn collect_offenders(root: &Path, dir: &Path, offenders: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
      return;
    };

    for entry in entries.flatten() {
      let path = entry.path();

      if path.is_dir() {
        collect_offenders(root, &path, offenders);
        continue;
      }

      if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
        continue;
      }

      // This module is the one place allowed to construct a `Command`.
      if path.file_name().and_then(|name| name.to_str()) == Some("process.rs")
        && path.parent() == Some(&root.join("backend"))
      {
        continue;
      }

      let Ok(contents) = fs::read_to_string(&path) else {
        continue;
      };

      for (index, line) in contents.lines().enumerate() {
        if line.contains("Command::new(") {
          let relative = path.strip_prefix(root).unwrap_or(&path);
          offenders.push(format!("  {}:{}", relative.display(), index + 1));
        }
      }
    }
  }
}
