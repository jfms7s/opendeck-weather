//! Cached Open-Meteo access shared by all three actions, so every key and
//! dial for the same place and units shares one request per refresh window.

use crate::cache::{Cache, CachePolicy};
use crate::model::{AirQuality, Forecast, Location, Place, Units};
use crate::open_meteo::{FetchError, OpenMeteo};
use chrono::{Duration as ChronoDuration, NaiveDateTime, Utc};
use std::sync::Arc;
use std::time::Duration;

const POLICY: CachePolicy = CachePolicy {
    // Open-Meteo's models update hourly at best; 10 minutes keeps "current"
    // conditions honest without hammering a free service.
    ttl: Duration::from_secs(10 * 60),
    retry_after: Duration::from_secs(60),
    stale_after: Duration::from_secs(3 * 60 * 60),
};

/// Coordinates rounded to ~100m, so two instances configured from slightly
/// different searches for the same town still share a cache entry.
type PlaceKey = (i64, i64);

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
}

impl Default for Services {
    fn default() -> Self {
        Self {
            api: OpenMeteo::default(),
            forecasts: Cache::new(POLICY),
            air: Cache::new(POLICY),
        }
    }
}

impl Services {
    pub async fn forecast(
        &self,
        location: &Location,
        units: Units,
    ) -> Result<Arc<Forecast>, FetchError> {
        self.forecasts
            .get(&(place_key(location), units), || {
                self.api.forecast(location, units)
            })
            .await
    }

    pub async fn air_quality(&self, location: &Location) -> Result<Arc<AirQuality>, FetchError> {
        self.air
            .get(&place_key(location), || self.api.air_quality(location))
            .await
    }

    /// Uncached: only the property inspector's search box calls this.
    pub async fn search(&self, query: &str) -> Result<Vec<Place>, FetchError> {
        self.api.search(query).await
    }
}

/// The location's current wall-clock time, from the offset its forecast
/// reported - independent of the machine's own timezone.
pub fn local_now(forecast: &Forecast) -> NaiveDateTime {
    Utc::now().naive_utc() + ChronoDuration::seconds(i64::from(forecast.utc_offset_seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_coordinates_share_a_cache_key() {
        let a = Location {
            name: "Lisbon".into(),
            latitude: 38.72509,
            longitude: -9.1498,
        };
        let b = Location {
            name: "Lisboa".into(),
            latitude: 38.72531,
            longitude: -9.14962,
        };
        assert_eq!(place_key(&a), place_key(&b));
    }
}
