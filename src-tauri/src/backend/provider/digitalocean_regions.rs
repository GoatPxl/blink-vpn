//! Geography for DigitalOcean region slugs.
//!
//! A VPN user picks a place, not a datacentre code — `sfo3` means nothing, "San Francisco,
//! California" is the actual decision. The provider API does not supply structured geography:
//! `GET /v2/regions` returns only a display name like "San Francisco 3", which carries a
//! datacentre ordinal and cannot be grouped or sorted by country. So the mapping is curated here.
//!
//! This table lives inside the DigitalOcean adapter deliberately. Region slugs are provider
//! namespaces — another provider may well use `sfo3` for somewhere else — so a shared global map
//! keyed by slug alone would be wrong as soon as a second provider exists (AGENTS.md §3).
//!
//! Unknown slugs resolve to geography-free entries rather than a guess. DigitalOcean adds regions,
//! and inventing a city for an unrecognised code would state something false.

use super::ProviderRegion;

/// `(slug, city, area, country, ISO-3166 alpha-2, continent)`
///
/// `area` is only filled where it disambiguates for a reader; countries where the city is
/// unambiguous leave it empty.
const DIGITALOCEAN_REGIONS: &[(&str, &str, &str, &str, &str, &str)] = &[
  ("nyc1", "New York City", "New York", "United States", "US", "North America"),
  ("nyc2", "New York City", "New York", "United States", "US", "North America"),
  ("nyc3", "New York City", "New York", "United States", "US", "North America"),
  ("sfo1", "San Francisco", "California", "United States", "US", "North America"),
  ("sfo2", "San Francisco", "California", "United States", "US", "North America"),
  ("sfo3", "San Francisco", "California", "United States", "US", "North America"),
  ("tor1", "Toronto", "Ontario", "Canada", "CA", "North America"),
  ("ams2", "Amsterdam", "", "Netherlands", "NL", "Europe"),
  ("ams3", "Amsterdam", "", "Netherlands", "NL", "Europe"),
  ("lon1", "London", "", "United Kingdom", "GB", "Europe"),
  ("fra1", "Frankfurt", "", "Germany", "DE", "Europe"),
  ("blr1", "Bangalore", "Karnataka", "India", "IN", "Asia"),
  ("sgp1", "Singapore", "", "Singapore", "SG", "Asia"),
  ("syd1", "Sydney", "New South Wales", "Australia", "AU", "Oceania"),
  ("atl1", "Atlanta", "Georgia", "United States", "US", "North America"),
];

/// Geography for a slug, or a slug-only entry when it is not recognised.
pub fn describe_region(slug: &str) -> ProviderRegion {
  let normalized = slug.trim().to_ascii_lowercase();

  DIGITALOCEAN_REGIONS
    .iter()
    .find(|(candidate, ..)| *candidate == normalized)
    .map(|(slug, city, area, country, country_code, continent)| ProviderRegion {
      slug: (*slug).to_string(),
      city: Some((*city).to_string()),
      area: optional(area),
      country: Some((*country).to_string()),
      country_code: Some((*country_code).to_string()),
      continent: Some((*continent).to_string()),
    })
    .unwrap_or_else(|| ProviderRegion::unknown(slug))
}

fn optional(value: &str) -> Option<String> {
  if value.is_empty() {
    None
  } else {
    Some(value.to_string())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn known_slugs_resolve_to_a_city_and_country() {
    let region = describe_region("sfo3");
    assert_eq!(region.slug, "sfo3");
    assert_eq!(region.city.as_deref(), Some("San Francisco"));
    assert_eq!(region.area.as_deref(), Some("California"));
    assert_eq!(region.country_code.as_deref(), Some("US"));
  }

  #[test]
  fn lookup_ignores_case_and_surrounding_space() {
    assert_eq!(describe_region("  FRA1 ").city.as_deref(), Some("Frankfurt"));
  }

  /// DigitalOcean adds regions. Guessing a city for a code we do not know would state a location
  /// the user might act on, so an unknown slug carries no geography at all.
  #[test]
  fn unknown_slugs_carry_no_invented_geography() {
    let region = describe_region("mars1");
    assert_eq!(region.slug, "mars1");
    assert_eq!(region.city, None);
    assert_eq!(region.area, None);
    assert_eq!(region.country, None);
    assert_eq!(region.country_code, None);
    assert_eq!(region.continent, None);
  }

  #[test]
  fn every_entry_is_complete_and_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for (slug, city, _area, country, country_code, continent) in DIGITALOCEAN_REGIONS {
      assert!(seen.insert(*slug), "duplicate region slug {slug}");
      assert!(!city.is_empty(), "{slug} is missing a city");
      assert!(!country.is_empty(), "{slug} is missing a country");
      assert_eq!(country_code.len(), 2, "{slug} needs an ISO-3166 alpha-2 code");
      assert!(!continent.is_empty(), "{slug} is missing a continent");
    }
  }

  /// The adapter's offline fallback list must be describable, or the create form would offer
  /// regions it cannot name.
  #[test]
  fn adapter_default_regions_are_all_mapped() {
    for slug in ["nyc1", "sfo3", "fra1"] {
      assert!(describe_region(slug).city.is_some(), "{slug} should be mapped");
    }
  }
}
