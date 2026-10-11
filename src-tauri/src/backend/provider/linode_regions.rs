//! Geography for Linode (Akamai) region codes.
//!
//! A VPN user picks a place, not a datacentre code — `us-ord` means nothing, "Chicago, United
//! States" is the actual decision. `GET /v4/regions` does return a human `label`, but it is a
//! single free-text field ("Chicago, IL") rather than the structured geography the UI groups by,
//! so the mapping is curated here like every other adapter's.
//!
//! This table lives inside the Linode adapter deliberately. Region slugs are provider namespaces,
//! so a shared global map keyed by slug alone would be wrong as soon as two providers use the same
//! code (AGENTS.md §3).
//!
//! Unknown codes resolve to geography-free entries rather than a guess. Akamai adds regions, and
//! inventing a city for an unrecognised code would state something false the user might act on.

use super::place_coordinates::place_coordinates;
use super::ProviderRegion;

/// `(code, city, area, country, ISO-3166 alpha-2, continent)`
///
/// `area` is only filled where it disambiguates for a reader; countries where the city is
/// unambiguous leave it empty.
///
/// Regions without the Metadata service never reach the UI (`list_regions` filters on the
/// capability), but they are still named here: an existing node's row keeps its slug forever, and
/// the fleet list must be able to label it offline.
const LINODE_REGIONS: &[(&str, &str, &str, &str, &str, &str)] = &[
  ("us-east", "Newark", "New Jersey", "United States", "US", "North America"),
  ("us-ord", "Chicago", "Illinois", "United States", "US", "North America"),
  ("us-central", "Dallas", "Texas", "United States", "US", "North America"),
  ("us-west", "Fremont", "California", "United States", "US", "North America"),
  ("us-lax", "Los Angeles", "California", "United States", "US", "North America"),
  ("us-sea", "Seattle", "Washington", "United States", "US", "North America"),
  ("us-mia", "Miami", "Florida", "United States", "US", "North America"),
  ("us-southeast", "Atlanta", "Georgia", "United States", "US", "North America"),
  ("us-iad", "Ashburn", "Virginia", "United States", "US", "North America"),
  ("ca-central", "Toronto", "Ontario", "Canada", "CA", "North America"),
  ("br-gru", "Sao Paulo", "", "Brazil", "BR", "South America"),
  ("eu-west", "London", "", "United Kingdom", "GB", "Europe"),
  ("gb-lon", "London", "", "United Kingdom", "GB", "Europe"),
  ("eu-central", "Frankfurt", "", "Germany", "DE", "Europe"),
  ("de-fra-2", "Frankfurt", "", "Germany", "DE", "Europe"),
  ("fr-par", "Paris", "", "France", "FR", "Europe"),
  ("nl-ams", "Amsterdam", "", "Netherlands", "NL", "Europe"),
  ("se-sto", "Stockholm", "", "Sweden", "SE", "Europe"),
  ("es-mad", "Madrid", "", "Spain", "ES", "Europe"),
  ("it-mil", "Milan", "", "Italy", "IT", "Europe"),
  ("ap-south", "Singapore", "", "Singapore", "SG", "Asia"),
  ("sg-sin-2", "Singapore", "", "Singapore", "SG", "Asia"),
  ("ap-northeast", "Tokyo", "", "Japan", "JP", "Asia"),
  ("jp-tyo-3", "Tokyo", "", "Japan", "JP", "Asia"),
  ("jp-osa", "Osaka", "", "Japan", "JP", "Asia"),
  ("ap-west", "Mumbai", "Maharashtra", "India", "IN", "Asia"),
  ("in-bom-2", "Mumbai", "Maharashtra", "India", "IN", "Asia"),
  ("in-maa", "Chennai", "Tamil Nadu", "India", "IN", "Asia"),
  ("id-cgk", "Jakarta", "", "Indonesia", "ID", "Asia"),
  ("ap-southeast", "Sydney", "New South Wales", "Australia", "AU", "Oceania"),
  ("au-mel", "Melbourne", "Victoria", "Australia", "AU", "Oceania"),
];

/// Every region in this table, without contacting the provider.
///
/// For the map's "needs a key" pins: with no credential there is no live catalog to ask, but the
/// places this provider runs are still worth showing. Building anywhere still goes through the live
/// list, which is what decides what can actually be provisioned.
pub fn known_regions() -> Vec<ProviderRegion> {
  LINODE_REGIONS.iter().map(|(slug, ..)| describe_region(slug)).collect()
}

/// Geography for a region code, or a code-only entry when it is not recognised.
pub fn describe_region(slug: &str) -> ProviderRegion {
  let normalized = slug.trim().to_ascii_lowercase();

  LINODE_REGIONS
    .iter()
    .find(|(candidate, ..)| *candidate == normalized)
    .map(|(slug, city, area, country, country_code, continent)| {
      let coordinates = place_coordinates(city, country_code);
      ProviderRegion {
        slug: (*slug).to_string(),
        city: Some((*city).to_string()),
        area: optional(area),
        country: Some((*country).to_string()),
        country_code: Some((*country_code).to_string()),
        continent: Some((*continent).to_string()),
        latitude: coordinates.map(|(latitude, _)| latitude),
        longitude: coordinates.map(|(_, longitude)| longitude),
      }
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
  fn known_regions_cover_the_whole_table() {
    let known = known_regions();
    assert_eq!(known.len(), LINODE_REGIONS.len());
    assert!(known.iter().all(|region| region.city.is_some() && region.latitude.is_some()));
  }

  #[test]
  fn every_listed_region_can_be_drawn_on_the_map() {
    for (_, city, _, _, country_code, _) in LINODE_REGIONS {
      assert!(
        place_coordinates(city, country_code).is_some(),
        "{city}, {country_code} has no coordinates; add it to place_coordinates.rs"
      );
    }
  }

  #[test]
  fn known_codes_resolve_to_a_city_and_country() {
    let region = describe_region("us-ord");
    assert_eq!(region.slug, "us-ord");
    assert_eq!(region.city.as_deref(), Some("Chicago"));
    assert_eq!(region.area.as_deref(), Some("Illinois"));
    assert_eq!(region.country_code.as_deref(), Some("US"));
  }

  #[test]
  fn lookup_ignores_case_and_surrounding_space() {
    assert_eq!(describe_region("  EU-WEST ").city.as_deref(), Some("London"));
  }

  /// Akamai adds regions. Guessing a city for a code we do not know would state a location the
  /// user might act on, so an unknown code carries no geography at all.
  #[test]
  fn unknown_codes_carry_no_invented_geography() {
    let region = describe_region("mars-north");
    assert_eq!(region.slug, "mars-north");
    assert_eq!(region.city, None);
    assert_eq!(region.area, None);
    assert_eq!(region.country, None);
    assert_eq!(region.country_code, None);
    assert_eq!(region.continent, None);
  }

  #[test]
  fn every_entry_is_complete_and_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for (slug, city, _area, country, country_code, continent) in LINODE_REGIONS {
      assert!(seen.insert(*slug), "duplicate region code {slug}");
      assert!(!city.is_empty(), "{slug} is missing a city");
      assert!(!country.is_empty(), "{slug} is missing a country");
      assert_eq!(country_code.len(), 2, "{slug} needs an ISO-3166 alpha-2 code");
      assert!(!continent.is_empty(), "{slug} is missing a continent");
    }
  }

  /// The region the adapter falls back to when none is chosen must itself be nameable, or a fresh
  /// account would be offered a region the UI cannot label.
  #[test]
  fn the_default_region_is_mapped() {
    assert!(describe_region(super::super::linode::DEFAULT_LINODE_REGION).city.is_some());
  }
}
