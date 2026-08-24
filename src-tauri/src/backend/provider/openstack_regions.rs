//! Geography for OpenStack region names — deliberately none.
//!
//! Every other adapter curates a table mapping the provider's region codes to a city and country,
//! because those codes come from one vendor and mean one fixed set of places. OpenStack is not a
//! vendor: it is software that anyone can run, and region names are chosen by whoever deployed it.
//! `RegionOne` is the install default and says nothing at all; `DFW` means Dallas on one cloud and
//! could mean anything on the next.
//!
//! So there is no honest table to write, and `describe_region` reports the slug with no geography
//! attached. That is the same answer every other adapter gives for a code it does not recognise,
//! and the contract prefers "not known" over a guess: inventing a city here would state a location
//! the user might act on when choosing where their traffic appears to originate.
//!
//! The file exists rather than being skipped so the adapter layout stays uniform — a missing
//! `<slug>_regions.rs` reads as an oversight, and the next person would add the table this comment
//! exists to argue against.

use super::ProviderRegion;

/// The slug, carrying no geography. See the module comment for why there is nothing to look up.
pub fn describe_region(slug: &str) -> ProviderRegion {
  ProviderRegion::unknown(slug)
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Names that mean a place on one cloud and something else on another must not be decorated
  /// with a location the operator never claimed.
  #[test]
  fn region_names_carry_no_invented_geography() {
    for slug in ["RegionOne", "DFW", "us-east-1", "nyc1"] {
      let region = describe_region(slug);
      assert_eq!(region.slug, slug, "the operator's own name must survive verbatim");
      assert_eq!(region.city, None);
      assert_eq!(region.area, None);
      assert_eq!(region.country, None);
      assert_eq!(region.country_code, None);
      assert_eq!(region.continent, None);
    }
  }

  /// `us-east-1` and `nyc1` are region codes that *do* mean something on AWS and DigitalOcean. An
  /// OpenStack cloud is free to reuse either name for somewhere else entirely, which is exactly
  /// why a global slug-to-place map would be wrong (AGENTS.md §3).
  #[test]
  fn a_name_another_provider_owns_is_not_borrowed_from_that_provider() {
    assert_eq!(describe_region("us-east-1").city, None);
    assert_eq!(super::super::ec2_regions::describe_region("us-east-1").city.as_deref(), Some("Ashburn"));
  }
}
