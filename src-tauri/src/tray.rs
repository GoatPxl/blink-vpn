//! The notification-area icon and its menu.
//!
//! The icon is the pixel goat in the app's current colour — lime secure, ember working, stone off,
//! alert failed — so the state is readable from the taskbar. The frontend decides that state, with
//! the same function that colours the rail, and pushes it here with `update_tray`.
//!
//! The menu only names actions. A click is sent back to the frontend as a `tray-action` event and
//! runs through the same code the window's buttons use; nothing here calls a provider or touches a
//! tunnel. Quitting leaves a connected tunnel up: it is a Windows service and outlives the app.

use std::time::Duration;

use serde::Deserialize;
use tauri::image::Image;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

const TRAY_ID: &str = "main";
const MAIN_WINDOW_LABEL: &str = "main";
const MENU_OPEN: &str = "tray-open";
const MENU_QUIT: &str = "tray-quit";
const MENU_STATUS: &str = "tray-status";

/// Event carrying a menu action id to the frontend.
pub const ACTION_EVENT: &str = "tray-action";
/// Event asking the frontend to finish anything pending, then call `quit_app`.
pub const QUIT_EVENT: &str = "tray-quit";
/// How long Quit waits for the frontend before exiting anyway, so a stuck webview cannot make the
/// app unquittable. Long enough for a pending power-off to reach the provider.
const QUIT_FALLBACK: Duration = Duration::from_secs(30);

const ICON_SIZE: u32 = 32;
const MAX_LABEL_CHARS: usize = 80;
const MAX_PLACES: usize = 5;

/// The goat, 12×13, the same drawing as the rail logo.
const GOAT: [&str; 13] = [
  "#..........#",
  "#..........#",
  "##........##",
  ".##......##.",
  "..########..",
  "####.##.####",
  "...######...",
  "...######...",
  "....####....",
  "....####....",
  ".....##.....",
  ".....##.....",
  "............",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrayTone {
  Secure,
  Busy,
  Off,
  Failed,
}

impl TrayTone {
  /// The design tokens: lime, ember, stone-2, alert.
  fn rgb(self) -> [u8; 3] {
    match self {
      TrayTone::Secure => [0xd4, 0xff, 0x3a],
      TrayTone::Busy => [0xff, 0x8a, 0x3d],
      TrayTone::Off => [0x6e, 0x6b, 0x60],
      TrayTone::Failed => [0xff, 0x4d, 0x6d],
    }
  }
}

/// One menu entry the frontend asks for: an id it will get back, and the words to show.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TrayItem {
  pub id: String,
  pub label: String,
}

/// What the tray shows. Sent whole on every change.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TrayState {
  pub tone: TrayTone,
  /// First line, not clickable: "Secure · San Francisco · since 21:40".
  pub status: String,
  /// The one thing to do next: Disconnect, or Connect · <last place>.
  pub primary: Option<TrayItem>,
  /// "Change location" while connected, "Connect elsewhere" while off.
  pub places_label: String,
  pub places: Vec<TrayItem>,
  /// "Power off server ($0.02 so far)", while one is running.
  pub power_off: Option<TrayItem>,
}

impl TrayState {
  /// What the tray shows before the frontend has said anything.
  fn starting() -> Self {
    Self {
      tone: TrayTone::Off,
      status: "Blink".to_string(),
      primary: None,
      places_label: "Connect elsewhere".to_string(),
      places: Vec::new(),
      power_off: None,
    }
  }
}

/// A label fit for a menu: one line, bounded length. The frontend is trusted, but a menu is no
/// place for a paragraph or a newline.
fn clean_label(label: &str) -> String {
  let single_line: String = label.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
  let trimmed = single_line.trim();
  if trimmed.chars().count() <= MAX_LABEL_CHARS {
    trimmed.to_string()
  } else {
    let cut: String = trimmed.chars().take(MAX_LABEL_CHARS - 1).collect();
    format!("{cut}…")
  }
}

/// The 32×32 RGBA icon: the goat in the tone's colour on a transparent ground.
fn goat_icon(tone: TrayTone) -> Image<'static> {
  let [r, g, b] = tone.rgb();
  let size = ICON_SIZE as usize;
  let mut rgba = vec![0u8; size * size * 4];
  let (cols, rows) = (GOAT[0].len(), GOAT.len());
  // Two device pixels per goat pixel, centred: 24×26 inside 32×32.
  let scale = 2;
  let left = (size - cols * scale) / 2;
  let top = (size - rows * scale) / 2;
  for (y, row) in GOAT.iter().enumerate() {
    for (x, cell) in row.chars().enumerate() {
      if cell != '#' {
        continue;
      }
      for dy in 0..scale {
        for dx in 0..scale {
          let px = left + x * scale + dx;
          let py = top + y * scale + dy;
          let offset = (py * size + px) * 4;
          rgba[offset..offset + 4].copy_from_slice(&[r, g, b, 0xff]);
        }
      }
    }
  }
  Image::new_owned(rgba, ICON_SIZE, ICON_SIZE)
}

fn build_menu(app: &AppHandle, state: &TrayState) -> tauri::Result<Menu<Wry>> {
  let status = MenuItem::with_id(app, MENU_STATUS, clean_label(&state.status), false, None::<&str>)?;
  let separator_top = PredefinedMenuItem::separator(app)?;
  let primary = state
    .primary
    .as_ref()
    .map(|item| MenuItem::with_id(app, item.id.as_str(), clean_label(&item.label), true, None::<&str>))
    .transpose()?;

  let place_items = state
    .places
    .iter()
    .take(MAX_PLACES)
    .map(|item| MenuItem::with_id(app, item.id.as_str(), clean_label(&item.label), true, None::<&str>))
    .collect::<tauri::Result<Vec<_>>>()?;
  let open_map = MenuItem::with_id(app, "open-map", "Open map…", true, None::<&str>)?;
  let mut place_refs: Vec<&dyn IsMenuItem<Wry>> = place_items.iter().map(|item| item as &dyn IsMenuItem<Wry>).collect();
  place_refs.push(&open_map);
  let places = Submenu::with_items(app, clean_label(&state.places_label), true, &place_refs)?;

  let power_off = state
    .power_off
    .as_ref()
    .map(|item| MenuItem::with_id(app, item.id.as_str(), clean_label(&item.label), true, None::<&str>))
    .transpose()?;
  let separator_bottom = PredefinedMenuItem::separator(app)?;
  let open = MenuItem::with_id(app, MENU_OPEN, "Open Blink", true, None::<&str>)?;
  let quit_label = if state.tone == TrayTone::Secure { "Quit (keeps tunnel up)" } else { "Quit" };
  let quit = MenuItem::with_id(app, MENU_QUIT, quit_label, true, None::<&str>)?;

  let mut items: Vec<&dyn IsMenuItem<Wry>> = vec![&status, &separator_top];
  if let Some(primary) = &primary {
    items.push(primary);
  }
  items.push(&places);
  if let Some(power_off) = &power_off {
    items.push(power_off);
  }
  items.push(&separator_bottom);
  items.push(&open);
  items.push(&quit);
  Menu::with_items(app, &items)
}

/// Creates the tray. It always exists — it is what a window closed to the tray comes back from —
/// so turning close-to-tray on later needs no restart.
pub fn init(app: &AppHandle) -> tauri::Result<()> {
  let state = TrayState::starting();
  TrayIconBuilder::with_id(TRAY_ID)
    .tooltip("Blink")
    .icon(goat_icon(state.tone))
    .menu(&build_menu(app, &state)?)
    // Left click restores the window; the menu is on right click, the Windows convention.
    .show_menu_on_left_click(false)
    .on_menu_event(|app, event| match event.id.as_ref() {
      MENU_OPEN => show_main_window(app),
      MENU_QUIT => request_quit(app),
      MENU_STATUS => {}
      id => {
        if let Err(err) = app.emit(ACTION_EVENT, id) {
          log::warn!("event=tray.action_emit_failed error={err}");
        }
      }
    })
    .on_tray_icon_event(|tray, event| match event {
      TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => {
        show_main_window(tray.app_handle());
      }
      // Double-click connects to the last place, without opening anything.
      TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => {
        if let Err(err) = tray.app_handle().emit(ACTION_EVENT, "connect-last") {
          log::warn!("event=tray.action_emit_failed error={err}");
        }
      }
      _ => {}
    })
    .build(app)?;
  Ok(())
}

/// Replaces the icon, tooltip and menu.
pub fn apply(app: &AppHandle, state: &TrayState) -> tauri::Result<()> {
  let Some(tray) = app.tray_by_id(TRAY_ID) else {
    return Ok(());
  };
  tray.set_icon(Some(goat_icon(state.tone)))?;
  tray.set_tooltip(Some(clean_label(&state.status)))?;
  tray.set_menu(Some(build_menu(app, state)?))?;
  Ok(())
}

/// Asks the frontend to finish up — a power-off still in its undo window must reach the provider,
/// or the server keeps billing — and exits anyway if it does not answer in time.
fn request_quit(app: &AppHandle) {
  if app.emit(QUIT_EVENT, ()).is_err() {
    app.exit(0);
    return;
  }
  let handle = app.clone();
  std::thread::spawn(move || {
    std::thread::sleep(QUIT_FALLBACK);
    log::warn!("event=tray.quit_fallback reason=frontend_did_not_quit");
    handle.exit(0);
  });
}

pub fn show_main_window(app: &AppHandle) {
  let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
    log::warn!("event=window.restore_failed reason=main_window_missing");
    return;
  };

  let restore = window
    .show()
    .and_then(|()| window.unminimize())
    .and_then(|()| window.set_focus());
  if let Err(err) = restore {
    log::warn!("event=window.restore_failed error={err}");
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_icon_is_the_goat_in_the_tone_colour() {
    let icon = goat_icon(TrayTone::Secure);
    assert_eq!((icon.width(), icon.height()), (ICON_SIZE, ICON_SIZE));
    let rgba = icon.rgba();
    let lit: Vec<&[u8]> = rgba.chunks(4).filter(|pixel| pixel[3] == 0xff).collect();
    let goat_pixels = GOAT.iter().map(|row| row.matches('#').count()).sum::<usize>();
    assert_eq!(lit.len(), goat_pixels * 4, "every goat pixel drawn 2×2");
    assert!(lit.iter().all(|pixel| pixel[..3] == TrayTone::Secure.rgb()));
    assert_ne!(TrayTone::Off.rgb(), TrayTone::Secure.rgb());
  }

  #[test]
  fn the_tray_goat_is_the_rail_goat() {
    // The logo is drawn once, in the frontend; this copy must not drift from it.
    let source = include_str!("../../src/components/ui/pixel-icon.tsx");
    let start = source.find("const GOAT = [").expect("GOAT grid in pixel-icon.tsx");
    let end = start + source[start..].find("] as const").expect("end of GOAT grid");
    let rows: Vec<&str> = source[start..end]
      .split('"')
      .filter(|part| !part.is_empty() && part.chars().all(|c| c == '#' || c == '.'))
      .collect();
    assert_eq!(rows, GOAT.to_vec());
  }

  #[test]
  fn labels_are_one_bounded_line() {
    assert_eq!(clean_label("  Secure · Frankfurt  "), "Secure · Frankfurt");
    assert_eq!(clean_label("two\nlines"), "two lines");
    let long = "x".repeat(200);
    assert_eq!(clean_label(&long).chars().count(), MAX_LABEL_CHARS);
  }

  #[test]
  fn state_parses_from_the_frontend_shape() {
    let state: TrayState = serde_json::from_str(
      r#"{"tone":"busy","status":"Building Frankfurt","primary":null,"places_label":"Connect elsewhere",
          "places":[{"id":"place:provider-1:fra1","label":"Frankfurt"}],"power_off":null}"#,
    )
    .expect("the frontend's shape must parse");
    assert_eq!(state.tone, TrayTone::Busy);
    assert_eq!(state.places.len(), 1);
  }
}
