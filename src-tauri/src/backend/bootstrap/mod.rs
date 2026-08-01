#[derive(Clone)]
pub struct Bootstrap;

impl Bootstrap {
  pub fn new() -> Self {
    Self
  }

  pub fn validate_environment(&self) -> Result<(), String> {
    Ok(())
  }
}