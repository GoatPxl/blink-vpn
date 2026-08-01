#[derive(Clone)]
pub struct SecureStorage;

impl SecureStorage {
  pub fn new() -> Self {
    Self
  }

  pub fn backend_name(&self) -> &'static str {
    "windows-credential-manager"
  }
}