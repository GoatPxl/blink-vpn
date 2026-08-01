#[derive(Clone)]
pub struct DigitalOceanProviderAdapter;

pub trait ProviderAdapter {
  fn provider_name(&self) -> &'static str;
  fn default_regions(&self) -> &'static [&'static str];
}

impl ProviderAdapter for DigitalOceanProviderAdapter {
  fn provider_name(&self) -> &'static str {
    "digitalocean"
  }

  fn default_regions(&self) -> &'static [&'static str] {
    &["nyc1", "sfo3", "fra1"]
  }
}

#[derive(Clone)]
pub struct ProviderRegistry {
  digitalocean: DigitalOceanProviderAdapter,
}

impl ProviderRegistry {
  pub fn new() -> Self {
    Self {
      digitalocean: DigitalOceanProviderAdapter,
    }
  }

  pub fn active(&self) -> &dyn ProviderAdapter {
    &self.digitalocean
  }
}