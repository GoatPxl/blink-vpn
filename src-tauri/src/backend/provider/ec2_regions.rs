//! Geography for Amazon EC2 region codes.
//!
//! A VPN user picks a place, not a datacentre code — `eu-west-2` means nothing, "London, United
//! Kingdom" is the actual decision. `DescribeRegions` returns only the code and an endpoint host,
//! with no structured geography, so the mapping is curated here.
//!
//! This table lives inside the EC2 adapter deliberately. Region slugs are provider namespaces, so
//! a shared global map keyed by slug alone would be wrong as soon as two providers use the same
//! code (AGENTS.md §3).
//!
//! Unknown codes resolve to geography-free entries rather than a guess. AWS adds regions, and
//! inventing a city for an unrecognised code would state something false the user might act on.

use super::ProviderRegion;

/// `(code, city, area, country, ISO-3166 alpha-2, continent)`
///
/// `area` is only filled where it disambiguates for a reader; countries where the city is
/// unambiguous leave it empty.
///
/// Opt-in regions (`af-south-1`, `me-south-1`, and similar) are listed because `DescribeRegions`
/// reports them for accounts that have enabled them. An account that has not will simply never see
/// the code, so listing it costs nothing and naming it correctly when it does appear is free.
const EC2_REGIONS: &[(&str, &str, &str, &str, &str, &str)] = &[
  ("us-east-1", "Ashburn", "Virginia", "United States", "US", "North America"),
  ("us-east-2", "Columbus", "Ohio", "United States", "US", "North America"),
  ("us-west-1", "San Jose", "California", "United States", "US", "North America"),
  ("us-west-2", "Portland", "Oregon", "United States", "US", "North America"),
  ("ca-central-1", "Montreal", "Quebec", "Canada", "CA", "North America"),
  ("ca-west-1", "Calgary", "Alberta", "Canada", "CA", "North America"),
  ("sa-east-1", "Sao Paulo", "", "Brazil", "BR", "South America"),
  ("eu-west-1", "Dublin", "", "Ireland", "IE", "Europe"),
  ("eu-west-2", "London", "", "United Kingdom", "GB", "Europe"),
  ("eu-west-3", "Paris", "", "France", "FR", "Europe"),
  ("eu-central-1", "Frankfurt", "", "Germany", "DE", "Europe"),
  ("eu-central-2", "Zurich", "", "Switzerland", "CH", "Europe"),
  ("eu-north-1", "Stockholm", "", "Sweden", "SE", "Europe"),
  ("eu-south-1", "Milan", "", "Italy", "IT", "Europe"),
  ("eu-south-2", "Zaragoza", "", "Spain", "ES", "Europe"),
  ("ap-south-1", "Mumbai", "Maharashtra", "India", "IN", "Asia"),
  ("ap-south-2", "Hyderabad", "Telangana", "India", "IN", "Asia"),
  ("ap-southeast-1", "Singapore", "", "Singapore", "SG", "Asia"),
  ("ap-southeast-2", "Sydney", "New South Wales", "Australia", "AU", "Oceania"),
  ("ap-southeast-3", "Jakarta", "", "Indonesia", "ID", "Asia"),
  ("ap-southeast-4", "Melbourne", "Victoria", "Australia", "AU", "Oceania"),
  ("ap-northeast-1", "Tokyo", "", "Japan", "JP", "Asia"),
  ("ap-northeast-2", "Seoul", "", "South Korea", "KR", "Asia"),
  ("ap-northeast-3", "Osaka", "", "Japan", "JP", "Asia"),
  ("ap-east-1", "Hong Kong", "", "Hong Kong", "HK", "Asia"),
  ("me-south-1", "Manama", "", "Bahrain", "BH", "Asia"),
  ("me-central-1", "Dubai", "", "United Arab Emirates", "AE", "Asia"),
  ("il-central-1", "Tel Aviv", "", "Israel", "IL", "Asia"),
  ("af-south-1", "Cape Town", "", "South Africa", "ZA", "Africa"),
];

/// Geography for a region code, or a code-only entry when it is not recognised.
pub fn describe_region(slug: &str) -> ProviderRegion {
  let normalized = slug.trim().to_ascii_lowercase();

  EC2_REGIONS
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
  fn known_codes_resolve_to_a_city_and_country() {
    let region = describe_region("us-east-1");
    assert_eq!(region.slug, "us-east-1");
    assert_eq!(region.city.as_deref(), Some("Ashburn"));
    assert_eq!(region.area.as_deref(), Some("Virginia"));
    assert_eq!(region.country_code.as_deref(), Some("US"));
  }

  #[test]
  fn lookup_ignores_case_and_surrounding_space() {
    assert_eq!(describe_region("  EU-WEST-2 ").city.as_deref(), Some("London"));
  }

  /// AWS adds regions. Guessing a city for a code we do not know would state a location the user
  /// might act on, so an unknown code carries no geography at all.
  #[test]
  fn unknown_codes_carry_no_invented_geography() {
    let region = describe_region("mars-north-1");
    assert_eq!(region.slug, "mars-north-1");
    assert_eq!(region.city, None);
    assert_eq!(region.area, None);
    assert_eq!(region.country, None);
    assert_eq!(region.country_code, None);
    assert_eq!(region.continent, None);
  }

  #[test]
  fn every_entry_is_complete_and_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for (slug, city, _area, country, country_code, continent) in EC2_REGIONS {
      assert!(seen.insert(*slug), "duplicate region code {slug}");
      assert!(!city.is_empty(), "{slug} is missing a city");
      assert!(!country.is_empty(), "{slug} is missing a country");
      assert_eq!(country_code.len(), 2, "{slug} needs an ISO-3166 alpha-2 code");
      assert!(!continent.is_empty(), "{slug} is missing a continent");
    }
  }

  /// The signing region the adapter falls back to for account-wide calls must itself be nameable,
  /// or a fresh account would be offered a region the UI cannot label.
  #[test]
  fn the_default_signing_region_is_mapped() {
    assert!(describe_region(super::super::ec2::DEFAULT_EC2_REGION).city.is_some());
  }
}
