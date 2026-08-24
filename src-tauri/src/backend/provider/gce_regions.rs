//! Geography for Google Compute Engine **zone** names.
//!
//! Zones, not regions, because a GCE instance is created in a zone and the trait's `region`
//! parameter is what the user picks. Mapping regions instead would leave the adapter choosing a
//! zone on the user's behalf — a second hidden decision the UI has no slot for.
//!
//! A VPN user picks a place, not a datacentre code: `europe-west2-a` means nothing, "London,
//! United Kingdom" is the actual decision. The Compute API's zone resource carries no structured
//! geography, so the mapping is curated here.
//!
//! This table lives inside the GCE adapter deliberately. Zone names are a provider namespace, so a
//! shared global map keyed by slug alone would be wrong as soon as two providers collide
//! (AGENTS.md §3).
//!
//! Unknown zones resolve to geography-free entries rather than a guess. Google adds zones, and
//! inventing a city for an unrecognised one would state something false the user might act on.
//!
//! Note the standing caveat for this provider: Google's published region location is where the
//! hardware is, but geolocation databases frequently place GCE egress addresses somewhere else
//! entirely. What this table names is the datacentre, not necessarily what a geo-IP lookup will
//! report — see the adapter's docs.

use super::ProviderRegion;

/// `(region prefix, city, area, country, ISO-3166 alpha-2, continent)`
///
/// Keyed by region rather than zone: every zone in `us-east1` is in the same place, so listing
/// `us-east1-b`, `-c` and `-d` separately would triple the table to say one thing. A zone is
/// matched by stripping its trailing `-<letter>`.
///
/// `area` is only filled where it disambiguates for a reader.
const GCE_REGIONS: &[(&str, &str, &str, &str, &str, &str)] = &[
  ("us-central1", "Council Bluffs", "Iowa", "United States", "US", "North America"),
  ("us-east1", "Moncks Corner", "South Carolina", "United States", "US", "North America"),
  ("us-east4", "Ashburn", "Virginia", "United States", "US", "North America"),
  ("us-east5", "Columbus", "Ohio", "United States", "US", "North America"),
  ("us-south1", "Dallas", "Texas", "United States", "US", "North America"),
  ("us-west1", "The Dalles", "Oregon", "United States", "US", "North America"),
  ("us-west2", "Los Angeles", "California", "United States", "US", "North America"),
  ("us-west3", "Salt Lake City", "Utah", "United States", "US", "North America"),
  ("us-west4", "Las Vegas", "Nevada", "United States", "US", "North America"),
  ("northamerica-northeast1", "Montreal", "Quebec", "Canada", "CA", "North America"),
  ("northamerica-northeast2", "Toronto", "Ontario", "Canada", "CA", "North America"),
  ("northamerica-south1", "Queretaro", "", "Mexico", "MX", "North America"),
  ("southamerica-east1", "Sao Paulo", "", "Brazil", "BR", "South America"),
  ("southamerica-west1", "Santiago", "", "Chile", "CL", "South America"),
  ("europe-west1", "St. Ghislain", "", "Belgium", "BE", "Europe"),
  ("europe-west2", "London", "", "United Kingdom", "GB", "Europe"),
  ("europe-west3", "Frankfurt", "", "Germany", "DE", "Europe"),
  ("europe-west4", "Eemshaven", "", "Netherlands", "NL", "Europe"),
  ("europe-west6", "Zurich", "", "Switzerland", "CH", "Europe"),
  ("europe-west8", "Milan", "", "Italy", "IT", "Europe"),
  ("europe-west9", "Paris", "", "France", "FR", "Europe"),
  ("europe-west10", "Berlin", "", "Germany", "DE", "Europe"),
  ("europe-west12", "Turin", "", "Italy", "IT", "Europe"),
  ("europe-north1", "Hamina", "", "Finland", "FI", "Europe"),
  ("europe-north2", "Stockholm", "", "Sweden", "SE", "Europe"),
  ("europe-central2", "Warsaw", "", "Poland", "PL", "Europe"),
  ("europe-southwest1", "Madrid", "", "Spain", "ES", "Europe"),
  ("asia-east1", "Changhua County", "", "Taiwan", "TW", "Asia"),
  ("asia-east2", "Hong Kong", "", "Hong Kong", "HK", "Asia"),
  ("asia-northeast1", "Tokyo", "", "Japan", "JP", "Asia"),
  ("asia-northeast2", "Osaka", "", "Japan", "JP", "Asia"),
  ("asia-northeast3", "Seoul", "", "South Korea", "KR", "Asia"),
  ("asia-south1", "Mumbai", "Maharashtra", "India", "IN", "Asia"),
  ("asia-south2", "Delhi", "", "India", "IN", "Asia"),
  ("asia-southeast1", "Jurong West", "", "Singapore", "SG", "Asia"),
  ("asia-southeast2", "Jakarta", "", "Indonesia", "ID", "Asia"),
  ("australia-southeast1", "Sydney", "New South Wales", "Australia", "AU", "Oceania"),
  ("australia-southeast2", "Melbourne", "Victoria", "Australia", "AU", "Oceania"),
  ("me-west1", "Tel Aviv", "", "Israel", "IL", "Asia"),
  ("me-central1", "Doha", "", "Qatar", "QA", "Asia"),
  ("me-central2", "Dammam", "", "Saudi Arabia", "SA", "Asia"),
  ("africa-south1", "Johannesburg", "", "South Africa", "ZA", "Africa"),
];

/// Geography for a zone name, or a code-only entry when it is not recognised.
pub fn describe_region(slug: &str) -> ProviderRegion {
  let normalized = slug.trim().to_ascii_lowercase();
  let region = region_of_zone(&normalized);

  GCE_REGIONS
    .iter()
    .find(|(candidate, ..)| *candidate == region)
    .map(|(_, city, area, country, country_code, continent)| ProviderRegion {
      // The zone the caller asked about, not the region it belongs to: the node row stores a zone
      // and the fleet list must label the exact place the instance runs.
      slug: normalized.clone(),
      city: Some((*city).to_string()),
      area: optional(area),
      country: Some((*country).to_string()),
      country_code: Some((*country_code).to_string()),
      continent: Some((*continent).to_string()),
    })
    .unwrap_or_else(|| ProviderRegion::unknown(slug))
}

/// `europe-west2-a` -> `europe-west2`. A value that is already a region is returned unchanged, so
/// the lookup works for both shapes.
fn region_of_zone(zone: &str) -> String {
  match zone.rsplit_once('-') {
    // Only a single-letter final segment is a zone suffix; `europe-west2` must not become
    // `europe`, and `northamerica-northeast1` must not become `northamerica`.
    Some((region, suffix)) if suffix.len() == 1 && suffix.chars().all(|c| c.is_ascii_alphabetic()) => {
      region.to_string()
    }
    _ => zone.to_string(),
  }
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
  fn known_zones_resolve_to_their_regions_geography() {
    let region = describe_region("us-central1-a");
    assert_eq!(region.slug, "us-central1-a", "the zone is what the node row stores");
    assert_eq!(region.city.as_deref(), Some("Council Bluffs"));
    assert_eq!(region.area.as_deref(), Some("Iowa"));
    assert_eq!(region.country_code.as_deref(), Some("US"));
  }

  #[test]
  fn every_zone_in_a_region_shares_its_place() {
    for zone in ["europe-west2-a", "europe-west2-b", "europe-west2-c"] {
      assert_eq!(describe_region(zone).city.as_deref(), Some("London"));
    }
  }

  #[test]
  fn lookup_ignores_case_and_surrounding_space() {
    assert_eq!(describe_region("  ASIA-NORTHEAST1-B ").city.as_deref(), Some("Tokyo"));
  }

  /// Splitting on the last hyphen unconditionally would turn `europe-west2` into `europe` and
  /// `northamerica-northeast1` into `northamerica`, losing every multi-word region.
  #[test]
  fn a_bare_region_resolves_without_being_mistaken_for_a_zone() {
    assert_eq!(describe_region("europe-west2").city.as_deref(), Some("London"));
    assert_eq!(
      describe_region("northamerica-northeast1").city.as_deref(),
      Some("Montreal")
    );
    assert_eq!(region_of_zone("northamerica-northeast1"), "northamerica-northeast1");
    assert_eq!(region_of_zone("northamerica-northeast1-a"), "northamerica-northeast1");
  }

  /// Google adds zones. Guessing a city for one we do not know would state a location the user
  /// might act on, so an unknown zone carries no geography at all.
  #[test]
  fn unknown_zones_carry_no_invented_geography() {
    let region = describe_region("mars-north1-a");
    assert_eq!(region.slug, "mars-north1-a");
    assert_eq!(region.city, None);
    assert_eq!(region.country, None);
    assert_eq!(region.country_code, None);
    assert_eq!(region.continent, None);
  }

  #[test]
  fn every_entry_is_complete_and_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for (slug, city, _area, country, country_code, continent) in GCE_REGIONS {
      assert!(seen.insert(*slug), "duplicate region {slug}");
      assert!(!city.is_empty(), "{slug} is missing a city");
      assert!(!country.is_empty(), "{slug} is missing a country");
      assert_eq!(country_code.len(), 2, "{slug} needs an ISO-3166 alpha-2 code");
      assert!(!continent.is_empty(), "{slug} is missing a continent");
    }
  }

  /// The zone the adapter falls back to must itself be nameable, or a fresh project would be
  /// offered a zone the UI cannot label.
  #[test]
  fn the_default_zone_is_mapped() {
    assert!(describe_region(super::super::gce::DEFAULT_GCE_ZONE).city.is_some());
  }
}
