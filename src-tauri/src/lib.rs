mod backend;
mod commands;

use std::path::PathBuf;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};

use backend::app_preferences::{self, CloseToTray};

const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_MENU_OPEN: &str = "tray-open";
const TRAY_MENU_QUIT: &str = "tray-quit";

pub fn run() {
  // One builder, no test-only variant.
  //
  // `tauri-plugin-playwright` used to be registered behind an `e2e-testing` feature. The E2E lane
  // drives the WebView2 devtools protocol directly instead, so the plugin was an unreachable
  // second mechanism kept alive by a feature flag — and it shipped a capability reference that a
  // production build could not resolve.
  tauri::Builder::default()
    // Must be the first plugin registered: it decides, before anything else starts, whether this
    // process is a relaunch that should hand off to the window already running in the tray.
    .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
      show_main_window(app);
    }))
    .setup(move |app| {
      app.handle().plugin(
        tauri_plugin_log::Builder::default()
          .level(backend::logging::default_level_filter())
          .build(),
      )?;

      let app_data_dir = app.path().app_data_dir().unwrap_or_else(|err| {
        log::warn!(
          "Unable to resolve app data directory: {}",
          crate::backend::logging::redact_sensitive_text(&err.to_string())
        );
        PathBuf::from(".")
      });

      std::fs::create_dir_all(&app_data_dir)?;
      let database_path = app_data_dir.join(backend::db::DATABASE_FILE_NAME);
      let backend = backend::Backend::new(database_path);

      let wireguard_diagnostics = backend.wireguard.resolution_diagnostics();
      let path_summary = summarize_path_for_log(wireguard_diagnostics.process_path.as_deref());
      log::info!(
        "event=wireguard.resolution_diagnostics path_summary={} wireguard_resolved_path={:?}",
        path_summary,
        wireguard_diagnostics.wireguard_resolved_path
      );

      if let Err(err) = backend.bootstrap.validate_environment() {
        log::warn!("bootstrap validation warning: {err}");
      }

      backend.db.run_migrations()?;

      let close_to_tray = app_preferences::get(&backend.db)
        .unwrap_or_else(|err| {
          log::warn!(
            "event=app_preferences.load_failed error={}",
            crate::backend::logging::redact_sensitive_text(&err.to_string())
          );
          app_preferences::AppPreferences::default()
        })
        .close_to_tray;
      app.manage(CloseToTray::new(close_to_tray));
      build_tray(app.handle())?;

      match backend.orchestrator.reconcile_startup(
        &backend.provider,
        &backend.wireguard,
        &backend.db,
        &backend.secure_storage,
      ) {
        Ok(report) => {
          if !report.issues.is_empty() {
            log::warn!("event=startup.reconcile.completed issue_count={}", report.issues.len());
            for issue in report.issues {
              log::warn!(
                "event=startup.reconcile.issue kind={:?} node_id={:?} tunnel_id={:?} repaired={} detail={}",
                issue.kind,
                issue.node_id,
                issue.tunnel_id,
                issue.repaired,
                crate::backend::logging::redact_sensitive_text(&issue.detail)
              );
            }
          }
        }
        Err(err) => {
          log::error!(
            "event=startup.reconcile.failed error={}",
            crate::backend::logging::redact_sensitive_text(&err.to_string())
          );
        }
      }

      app.manage(backend.clone());
      Ok(())
    })
    .on_window_event(|window, event| {
      if let WindowEvent::CloseRequested { api, .. } = event {
        let hide_to_tray = window.label() == MAIN_WINDOW_LABEL
          && window.try_state::<CloseToTray>().is_some_and(|state| state.enabled());
        if hide_to_tray {
          api.prevent_close();
          if let Err(err) = window.hide() {
            log::warn!("event=window.hide_failed error={err}");
          }
        }
      }
    })
    .invoke_handler(tauri::generate_handler![
      commands::system::open_wireguard_install_page,
      commands::system::install_wireguard_with_prerequisites,
      commands::system::check_wireguard_readiness,
      commands::system::check_docker_readiness,
      commands::system::register_provider_credential,
      commands::system::list_provider_credential_fields,
      commands::system::list_provider_regions,
      commands::system::list_provider_pricing,
      commands::system::list_providers,
      commands::system::suspend_provider,
      commands::system::resume_provider,
      commands::system::delete_provider,
      commands::system::list_vpn_nodes,
      commands::system::get_current_node_id,
      commands::system::set_current_node_id,
      commands::system::get_tunnel_settings,
      commands::system::set_tunnel_settings,
      commands::system::get_app_preferences,
      commands::system::set_app_preferences,
      commands::system::list_tunnels_for_node,
      commands::system::destroy_vpn_node,
      commands::system::force_cleanup_vpn_node,
      commands::system::list_saved_locations,
      commands::system::turn_on_saved_location,
      commands::system::turn_off_saved_location,
      commands::system::forget_saved_location,
      commands::system::create_vpn_node_to_ready,
      commands::system::create_and_activate_client_tunnel,
      commands::system::activate_client_tunnel,
      commands::system::deactivate_client_tunnel,
      commands::system::connect_tunnel,
      commands::system::disconnect_tunnel,
      commands::system::force_disconnect_node_service,
      commands::system::get_health_panel_data,
      commands::system::get_tunnel_phase,
      commands::system::list_node_peers,
      commands::system::claim_node_peer,
      commands::system::get_peer_config,
      commands::system::export_peer_config,
      commands::system::get_diagnostics_bundle,
      commands::system::export_diagnostics_bundle,
      commands::system::list_app_locations,
      commands::system::reveal_app_location,
      commands::system::validate_provider_token,
      commands::system::provision_provider_node,
      commands::system::delete_provider_node
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

/// The notification-area icon. It is what a window closed to the tray comes back from, so it
/// exists whether or not close-to-tray is on — turning the setting on later must not need a restart.
///
/// "Quit" exits the app only. A connected tunnel is a Windows service and keeps running, exactly as
/// it does when the window is closed with the setting off.
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
  let open = MenuItem::with_id(app, TRAY_MENU_OPEN, "Open Blink", true, None::<&str>)?;
  let quit = MenuItem::with_id(app, TRAY_MENU_QUIT, "Quit", true, None::<&str>)?;
  let menu = Menu::with_items(app, &[&open, &quit])?;

  let mut builder = TrayIconBuilder::with_id("main")
    .tooltip("Blink VPN")
    .menu(&menu)
    // Left click restores the window; the menu is on right click, the Windows convention.
    .show_menu_on_left_click(false)
    .on_menu_event(|app, event| match event.id.as_ref() {
      TRAY_MENU_OPEN => show_main_window(app),
      TRAY_MENU_QUIT => app.exit(0),
      _ => {}
    })
    .on_tray_icon_event(|tray, event| {
      if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
      } = event
      {
        show_main_window(tray.app_handle());
      }
    });

  if let Some(icon) = app.default_window_icon() {
    builder = builder.icon(icon.clone());
  }

  builder.build(app)?;
  Ok(())
}

fn show_main_window(app: &AppHandle) {
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

fn summarize_path_for_log(path_value: Option<&str>) -> String {
  let Some(raw) = path_value else {
    return "<unset>".to_string();
  };

  let entries = raw
    .split(';')
    .map(|value| value.trim())
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>();

  let mut wireguard_entries = entries
    .iter()
    .copied()
    .filter(|entry| entry.to_ascii_lowercase().contains("wireguard"))
    .take(3)
    .map(|entry| entry.to_string())
    .collect::<Vec<_>>();

  if wireguard_entries.is_empty() {
    wireguard_entries.push("none".to_string());
  }

  format!(
    "entries={} wireguard_entries={}",
    entries.len(),
    wireguard_entries.join("|")
  )
}
