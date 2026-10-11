//! Where a datacentre city is, for drawing it on the map.
//!
//! Keyed by physical place — city and ISO country code — not by any provider's region slug. Slugs
//! are provider namespaces (AGENTS.md §3) and stay inside each adapter's own table; a city is a
//! fact about the world, and San Francisco is in the same place whichever cloud runs a datacentre
//! there. One table therefore serves every adapter, and a city shared by several providers has one
//! set of coordinates instead of a copy per provider.
//!
//! Precision is about a tenth of a degree: enough to put a dot on a 72×36 world map, and no claim
//! about the building. A city not listed here has no coordinates, and the map lists it without a
//! pin rather than guessing.

/// `(city, ISO-3166 alpha-2, latitude, longitude)`. Cities as the adapters' tables spell them.
const PLACES: &[(&str, &str, f64, f64)] = &[
  ("Amsterdam", "NL", 52.37, 4.90),
  ("Ashburn", "US", 39.04, -77.49),
  ("Atlanta", "US", 33.75, -84.39),
  ("Bangalore", "IN", 12.97, 77.59),
  ("Berlin", "DE", 52.52, 13.40),
  ("Calgary", "CA", 51.05, -114.07),
  ("Cape Town", "ZA", -33.92, 18.42),
  ("Changhua County", "TW", 24.07, 120.54),
  ("Chennai", "IN", 13.08, 80.27),
  ("Chicago", "US", 41.88, -87.63),
  ("Columbus", "US", 39.96, -83.00),
  ("Council Bluffs", "US", 41.26, -95.86),
  ("Dallas", "US", 32.78, -96.80),
  ("Dammam", "SA", 26.43, 50.10),
  ("Delhi", "IN", 28.70, 77.10),
  ("Doha", "QA", 25.29, 51.53),
  ("Dubai", "AE", 25.20, 55.27),
  ("Dublin", "IE", 53.35, -6.26),
  ("Eemshaven", "NL", 53.44, 6.83),
  ("Frankfurt", "DE", 50.11, 8.68),
  ("Fremont", "US", 37.55, -121.99),
  ("Hamina", "FI", 60.57, 27.20),
  ("Hong Kong", "HK", 22.32, 114.17),
  ("Hyderabad", "IN", 17.39, 78.49),
  ("Jakarta", "ID", -6.21, 106.85),
  ("Johannesburg", "ZA", -26.20, 28.05),
  ("Jurong West", "SG", 1.34, 103.71),
  ("Las Vegas", "US", 36.17, -115.14),
  ("London", "GB", 51.51, -0.13),
  ("Los Angeles", "US", 34.05, -118.24),
  ("Madrid", "ES", 40.42, -3.70),
  ("Manama", "BH", 26.23, 50.59),
  ("Melbourne", "AU", -37.81, 144.96),
  ("Miami", "US", 25.76, -80.19),
  ("Milan", "IT", 45.46, 9.19),
  ("Moncks Corner", "US", 33.20, -80.01),
  ("Montreal", "CA", 45.50, -73.57),
  ("Mumbai", "IN", 19.08, 72.88),
  ("New York City", "US", 40.71, -74.01),
  ("Newark", "US", 40.74, -74.17),
  ("Osaka", "JP", 34.69, 135.50),
  ("Paris", "FR", 48.86, 2.35),
  ("Portland", "US", 45.52, -122.68),
  ("Queretaro", "MX", 20.59, -100.39),
  ("Salt Lake City", "US", 40.76, -111.89),
  ("San Francisco", "US", 37.77, -122.42),
  ("San Jose", "US", 37.34, -121.89),
  ("Santiago", "CL", -33.45, -70.67),
  ("Sao Paulo", "BR", -23.55, -46.63),
  ("Seattle", "US", 47.61, -122.33),
  ("Seoul", "KR", 37.57, 126.98),
  ("Singapore", "SG", 1.35, 103.82),
  ("St. Ghislain", "BE", 50.45, 3.82),
  ("Stockholm", "SE", 59.33, 18.07),
  ("Sydney", "AU", -33.87, 151.21),
  ("Tel Aviv", "IL", 32.09, 34.78),
  ("The Dalles", "US", 45.59, -121.18),
  ("Tokyo", "JP", 35.68, 139.69),
  ("Toronto", "CA", 43.65, -79.38),
  ("Turin", "IT", 45.07, 7.69),
  ("Warsaw", "PL", 52.23, 21.01),
  ("Zaragoza", "ES", 41.65, -0.89),
  ("Zurich", "CH", 47.38, 8.54),
];

/// `(latitude, longitude)` for a city, or `None` when it is not listed.
pub fn place_coordinates(city: &str, country_code: &str) -> Option<(f64, f64)> {
  PLACES
    .iter()
    .find(|(name, code, ..)| name.eq_ignore_ascii_case(city) && code.eq_ignore_ascii_case(country_code))
    .map(|(_, _, latitude, longitude)| (*latitude, *longitude))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_place_is_on_the_globe() {
    for (city, code, latitude, longitude) in PLACES {
      assert!((-90.0..=90.0).contains(latitude), "{city}, {code}: latitude {latitude}");
      assert!((-180.0..=180.0).contains(longitude), "{city}, {code}: longitude {longitude}");
    }
  }

  #[test]
  fn places_are_listed_once() {
    let mut seen = std::collections::HashSet::new();
    for (city, code, ..) in PLACES {
      assert!(seen.insert((city.to_ascii_lowercase(), *code)), "{city}, {code} is listed twice");
    }
  }

  #[test]
  fn hemispheres_are_the_right_way_round() {
    // A swapped sign puts Sydney in the Pacific north of Japan; these catch that class of typo.
    let (sydney_lat, sydney_lon) = place_coordinates("Sydney", "AU").expect("Sydney is listed");
    assert!(sydney_lat < 0.0 && sydney_lon > 0.0);
    let (sf_lat, sf_lon) = place_coordinates("San Francisco", "US").expect("San Francisco is listed");
    assert!(sf_lat > 0.0 && sf_lon < 0.0);
  }

  #[test]
  fn an_unlisted_city_has_no_coordinates() {
    assert_eq!(place_coordinates("Atlantis", "GR"), None);
    assert_eq!(place_coordinates("Sydney", "CA"), None, "the country must match too");
  }
}
