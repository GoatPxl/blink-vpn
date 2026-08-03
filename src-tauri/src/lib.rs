mod backend;
mod commands;

use std::path::PathBuf;

use tauri::Manager;

pub fn run() {
  #[cfg(feature = "e2e-testing")]
  let builder = tauri::Builder::default().plugin(tauri_plugin_playwright::init());

  #[cfg(not(feature = "e2e-testing"))]
  let builder = tauri::Builder::default();

  builder
    .setup(move |app| {
      let log_level = if cfg!(debug_assertions) {
        log::LevelFilter::Info
      } else {
        log::LevelFilter::Warn
      };

      app.handle().plugin(
        tauri_plugin_log::Builder::default()
          .level(log_level)
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
      let database_path = app_data_dir.join("goatstare_phase1.sqlite3");
      let backend = backend::Backend::new(database_path);

      let wireguard_diagnostics = backend.wireguard.resolution_diagnostics();
      let path_summary = summarize_path_for_log(wireguard_diagnostics.process_path.as_deref());
      log::info!(
        "event=wireguard.resolution_diagnostics path_summary={} wg_resolved_path={:?} wireguard_resolved_path={:?}",
        path_summary,
        wireguard_diagnostics.wg_resolved_path,
        wireguard_diagnostics.wireguard_resolved_path
      );

      if let Err(err) = backend.bootstrap.validate_environment() {
        log::warn!("bootstrap validation warning: {err}");
      }

      backend.db.run_migrations()?;

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
    .invoke_handler(tauri::generate_handler![
      commands::system::hello,
      commands::system::open_wireguard_install_page,
      commands::system::install_wireguard_with_prerequisites,
      commands::system::check_wireguard_readiness,
      commands::system::check_docker_readiness,
      commands::system::register_digitalocean_provider_token,
      commands::system::register_docker_provider_token,
      commands::system::list_provider_regions,
      commands::system::list_provider_pricing,
      commands::system::list_providers,
      commands::system::suspend_provider,
      commands::system::resume_provider,
      commands::system::delete_provider,
      commands::system::list_vpn_nodes,
      commands::system::list_tunnels_for_node,
      commands::system::destroy_vpn_node,
      commands::system::force_cleanup_vpn_node,
      commands::system::create_vpn_node_to_ready,
      commands::system::create_and_activate_client_tunnel,
      commands::system::activate_client_tunnel,
      commands::system::deactivate_client_tunnel,
      commands::system::connect_tunnel,
      commands::system::disconnect_tunnel,
      commands::system::get_health_panel_data,
      commands::system::get_diagnostics_bundle,
      commands::system::export_diagnostics_bundle,
      commands::system::read_client_tunnel_status,
      commands::system::validate_provider_token,
      commands::system::provision_provider_droplet,
      commands::system::delete_provider_droplet
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
