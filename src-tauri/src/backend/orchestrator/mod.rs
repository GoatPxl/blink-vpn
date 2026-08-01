#[derive(Clone)]
pub struct Orchestrator;

impl Orchestrator {
  pub fn new() -> Self {
    Self
  }

  pub fn status(&self) -> &'static str {
    "idle"
  }
}