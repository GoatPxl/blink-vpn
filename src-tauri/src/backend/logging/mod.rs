#[derive(Clone)]
pub struct Logger;

impl Logger {
  pub fn new() -> Self {
    Self
  }

  pub fn subsystem_name(&self) -> &'static str {
    "logging"
  }
}