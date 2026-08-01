mod backend;
mod commands;

use tauri::Manager;

pub fn run() {
  let backend = backend::Backend::new();

  tauri::Builder::default()
    .setup(move |app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }

      if let Err(err) = backend.bootstrap.validate_environment() {
        log::warn!("bootstrap validation warning: {err}");
      }

      app.manage(backend.clone());
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![commands::system::hello])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
