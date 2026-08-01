mod backend;
mod commands;

use std::path::PathBuf;

use tauri::Manager;

pub fn run() {
  tauri::Builder::default()
    .setup(move |app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }

      let app_data_dir = app.path().app_data_dir().unwrap_or_else(|err| {
        log::warn!("Unable to resolve app data directory: {err}");
        PathBuf::from(".")
      });

      std::fs::create_dir_all(&app_data_dir)?;
      let database_path = app_data_dir.join("goatstare_phase1.sqlite3");
      let backend = backend::Backend::new(database_path);

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
            log::warn!(
              "Startup reconciliation completed with {} issue(s).",
              report.issues.len()
            );
            for issue in report.issues {
              log::warn!(
                "reconcile_issue kind={:?} node_id={:?} tunnel_id={:?} repaired={} detail={}",
                issue.kind,
                issue.node_id,
                issue.tunnel_id,
                issue.repaired,
                issue.detail
              );
            }
          }
        }
        Err(err) => {
          log::error!("Startup reconciliation failed: {}", err);
        }
      }

      app.manage(backend.clone());
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      commands::system::hello,
      commands::system::register_digitalocean_provider_token,
      commands::system::list_vpn_nodes,
      commands::system::list_tunnels_for_node,
      commands::system::destroy_vpn_node,
      commands::system::create_vpn_node_to_ready,
      commands::system::create_and_activate_client_tunnel,
      commands::system::activate_client_tunnel,
      commands::system::deactivate_client_tunnel,
      commands::system::connect_tunnel,
      commands::system::disconnect_tunnel,
      commands::system::get_health_panel_data,
      commands::system::get_diagnostics_bundle,
      commands::system::read_client_tunnel_status,
      commands::system::validate_provider_token,
      commands::system::provision_provider_droplet,
      commands::system::delete_provider_droplet
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
