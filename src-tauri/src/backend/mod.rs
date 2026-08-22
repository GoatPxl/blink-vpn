pub mod bootstrap;
pub mod crypto;
pub mod db;
pub mod feature_flags;
pub mod logging;
pub mod orchestrator;
pub mod provider;
pub mod qr;
pub mod secure_storage;
pub mod wireguard;

use std::path::PathBuf;

#[derive(Clone)]
pub struct Backend {
  pub orchestrator: orchestrator::Orchestrator,
  pub provider: provider::ProviderRegistry,
  pub bootstrap: bootstrap::Bootstrap,
  pub secure_storage: secure_storage::SecureStorage,
  pub wireguard: wireguard::WireGuardManager,
  pub feature_flags: feature_flags::FeatureFlags,
  pub db: db::Database,
}

impl Backend {
  pub fn new(database_path: PathBuf) -> Self {
    Self {
      orchestrator: orchestrator::Orchestrator::new(),
      provider: provider::ProviderRegistry::new(),
      bootstrap: bootstrap::Bootstrap::new(),
      secure_storage: secure_storage::SecureStorage::new(),
      wireguard: wireguard::WireGuardManager::new(),
      feature_flags: feature_flags::FeatureFlags::from_env(),
      db: db::Database::new(database_path),
    }
  }
}