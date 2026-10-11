mod backend;
mod commands;
mod tray;

use std::path::PathBuf;

use tauri::{Manager, WindowEvent};

use backend::app_preferences::{self, CloseToTray};

const MAIN_WINDOW_LABEL: &str = "main";

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
      tray::show_main_window(app);
    }))
    // Native toasts while the window is hidden in the tray. Sent from the frontend, which knows
    // what happened; see `notification-bridge.tsx`.
    .plugin(tauri_plugin_notification::init())
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
      tray::init(app.handle())?;

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
      commands::system::open_provider_token_page,
      commands::system::install_wireguard_with_prerequisites,
      commands::system::check_wireguard_readiness,
      commands::system::check_docker_readiness,
      commands::system::register_provider_credential,
      commands::system::list_provider_credential_fields,
      commands::system::list_provider_regions,
      commands::system::list_known_provider_regions,
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
      commands::system::delete_provider_node,
      commands::system::update_tray,
      commands::system::quit_app
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
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
