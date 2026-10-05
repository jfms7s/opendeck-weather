//! Cached Open-Meteo access shared by all three actions, so every key and
//! dial for the same place and units shares one request per refresh window.
//! Also holds the plugin-wide default location (global settings).

use crate::cache::{Cache, CachePolicy, Cached};
use crate::model::{AirQuality, Forecast, Location, Place, Settings, Units};
use crate::open_meteo::{FetchError, OpenMeteo};
use std::sync::RwLock;
use std::time::Duration;

pub const POLICY: CachePolicy = CachePolicy {
    // Open-Meteo's models update hourly at best; 10 minutes keeps "current"
    // conditions honest without hammering a free service.
    ttl: Duration::from_secs(10 * 60),
    // A little under the once-a-minute refresh, so after a failure the next
    // refresh always retries - the README promises recovery within a minute.
    retry_after: Duration::from_secs(55),
    stale_after: Duration::from_secs(3 * 60 * 60),
};

/// Coordinates rounded to ~100m, so two instances configured from slightly
/// different searches for the same town still share a cache entry.
type PlaceKey = (i64, i64);

#[allow(clippy::cast_possible_truncation)] // saturating; real coordinates are tiny
fn place_key(location: &Location) -> PlaceKey {
    (
        (location.latitude * 1000.0).round() as i64,
        (location.longitude * 1000.0).round() as i64,
    )
}

pub struct Services {
    api: OpenMeteo,
    forecasts: Cache<(PlaceKey, Units), Forecast, FetchError>,
    air: Cache<PlaceKey, AirQuality, FetchError>,
    default_location: RwLock<Option<Location>>,
}

impl Default for Services {
    fn default() -> Self {
        Self {
            api: OpenMeteo::default(),
            forecasts: Cache::new(POLICY),
            air: Cache::new(POLICY),
            default_location: RwLock::new(None),
        }
    }
}

impl Services {
    pub async fn forecast(
        &self,
        location: &Location,
        units: Units,
    ) -> Result<Cached<Forecast>, FetchError> {
        self.forecasts
            .get(&(place_key(location), units), || {
                self.api.forecast(location, units)
            })
            .await
    }

    pub async fn air_quality(&self, location: &Location) -> Result<Cached<AirQuality>, FetchError> {
        self.air
            .get(&place_key(location), || self.api.air_quality(location))
            .await
    }

    /// Uncached: only the property inspector's search box calls this.
    pub async fn search(&self, query: &str) -> Result<Vec<Place>, FetchError> {
        self.api.search(query).await
    }

    pub fn default_location(&self) -> Option<Location> {
        self.default_location
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Returns whether it changed.
    pub fn set_default_location(&self, location: Option<Location>) -> bool {
        let mut current = self
            .default_location
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let changed = *current != location;
        *current = location;
        changed
    }

    /// The instance's own location, else the plugin-wide default.
    pub fn location_for(&self, settings: &Settings) -> Option<Location> {
        settings
            .location
            .clone()
            .or_else(|| self.default_location())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(name: &str, latitude: f64, longitude: f64) -> Location {
        Location {
            name: name.into(),
            latitude,
            longitude,
            label: None,
        }
    }

    #[test]
    fn nearby_coordinates_share_a_cache_key() {
        let a = at("Lisbon", 38.72509, -9.1498);
        let b = at("Lisboa", 38.72531, -9.14962);
        assert_eq!(place_key(&a), place_key(&b));
    }

    #[test]
    fn distinct_places_do_not_share_a_cache_key() {
        let a = at("Lisbon", 38.725, -9.150);
        assert_ne!(place_key(&a), place_key(&at("North", 38.727, -9.150)));
        assert_ne!(place_key(&a), place_key(&at("East", 38.725, -9.148)));
    }

    #[test]
    fn an_instance_location_wins_over_the_default() {
        let services = Services::default();
        let own = Settings {
            location: Some(at("Porto", 41.15, -8.61)),
            ..Settings::default()
        };
        assert_eq!(services.location_for(&Settings::default()), None);
        assert!(services.set_default_location(Some(at("Lisbon", 38.7, -9.1))));
        assert!(!services.set_default_location(Some(at("Lisbon", 38.7, -9.1))));
        assert_eq!(
            services.location_for(&Settings::default()).unwrap().name,
            "Lisbon"
        );
        assert_eq!(services.location_for(&own).unwrap().name, "Porto");
    }
}
