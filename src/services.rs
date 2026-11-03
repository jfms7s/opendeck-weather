//! Cached Open-Meteo access shared by all three actions, so every key and
//! dial for the same place and units shares one request per refresh window.
//! Also holds the plugin-wide default location (global settings), and
//! IPMA's warnings for Portugal (one cached feed for the whole country).

use crate::cache::{Cache, CachePolicy, Cached};
use crate::ipma::{self, Warning};
use crate::model::{AirQuality, Forecast, Location, Place, Settings, Units};
use crate::open_meteo::{FetchError, OpenMeteo};
use chrono::{DateTime, Utc};
use std::sync::{Arc, RwLock};
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
    /// Shared with the background task that refreshes it.
    warnings: Arc<Cache<(), Vec<Warning>, FetchError>>,
    default_location: RwLock<Option<Location>>,
}

impl Default for Services {
    fn default() -> Self {
        Self {
            api: OpenMeteo::default(),
            forecasts: Cache::new(POLICY),
            air: Cache::new(POLICY),
            warnings: Arc::new(Cache::new(POLICY)),
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

    /// Fetches the forecast now even if the cached one is fresh (still
    /// backing off after a failure).
    pub async fn refresh_forecast(
        &self,
        location: &Location,
        units: Units,
    ) -> Result<Cached<Forecast>, FetchError> {
        self.forecasts
            .refresh(&(place_key(location), units), || {
                self.api.forecast(location, units)
            })
            .await
    }

    /// `refresh_forecast` for air quality.
    pub async fn refresh_air_quality(
        &self,
        location: &Location,
    ) -> Result<Cached<AirQuality>, FetchError> {
        self.air
            .refresh(&place_key(location), || self.api.air_quality(location))
            .await
    }

    /// The IPMA warning active at `location` now, from what is cached:
    /// `None` outside Portugal (no request is made), before the first fetch
    /// lands, or while IPMA is unreachable with nothing younger than
    /// `stale_after`. Never waits on the network: when a fetch is due it
    /// runs in the background, and the next render shows the result.
    pub fn warning(&self, location: &Location, utc_now: DateTime<Utc>) -> Option<ipma::Active> {
        let api = self.api.clone();
        self.warning_from(
            location,
            utc_now,
            move || async move { ipma::fetch(&api).await },
        )
    }

    fn warning_from<F, Fut>(
        &self,
        location: &Location,
        utc_now: DateTime<Utc>,
        fetch: F,
    ) -> Option<ipma::Active>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<Vec<Warning>, FetchError>> + Send,
    {
        let area = ipma::area_for(location)?;
        let (cached, due) = self.warnings.peek(&());
        if due {
            let warnings = self.warnings.clone();
            // A failure is logged once per attempt by the cache.
            tokio::spawn(async move { drop(warnings.get(&(), fetch).await) });
        }
        ipma::active(&cached?.value, area, utc_now)
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

    use crate::ipma::{Kind, Level};
    use chrono::TimeZone;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn rain_warning(area: &str) -> Warning {
        Warning {
            area: area.into(),
            kind: Kind::Rain,
            level: Level::Orange,
            start: Utc.with_ymd_and_hms(2026, 11, 3, 6, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 11, 3, 18, 0, 0).unwrap(),
        }
    }

    fn noon() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 3, 12, 0, 0).unwrap()
    }

    /// `warning_from` with a fetcher that counts its calls, then lets the
    /// background fetch finish.
    async fn warning_counted(
        services: &Services,
        location: &Location,
        calls: &Arc<AtomicU32>,
        result: Result<Vec<Warning>, FetchError>,
    ) -> Option<ipma::Active> {
        let counter = calls.clone();
        let w = services.warning_from(location, noon(), move || async move {
            counter.fetch_add(1, Ordering::SeqCst);
            result
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        w
    }

    #[tokio::test(start_paused = true)]
    async fn a_location_outside_portugal_makes_no_ipma_request() {
        let services = Services::default();
        let calls = Arc::new(AtomicU32::new(0));
        let madrid = at("Madrid", 40.4168, -3.7038);
        for _ in 0..3 {
            let w =
                warning_counted(&services, &madrid, &calls, Ok(vec![rain_warning("LSB")])).await;
            assert_eq!(w, None);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn warnings_are_fetched_in_the_background_and_shown_next_time() {
        let services = Services::default();
        let calls = Arc::new(AtomicU32::new(0));
        let lisbon = at("Lisbon", 38.72509, -9.1498);
        let feed = || Ok(vec![rain_warning("LSB"), rain_warning("PTO")]);
        // The first render doesn't wait: no badge yet, a fetch starts.
        assert_eq!(
            warning_counted(&services, &lisbon, &calls, feed()).await,
            None
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // The next one shows it, from the cache.
        let w = warning_counted(&services, &lisbon, &calls, feed())
            .await
            .unwrap();
        assert_eq!((w.kind, w.level), (Kind::Rain, Level::Orange));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Another area of the same feed: no new request, no warning.
        let faro = at("Faro", 37.0194, -7.9304);
        assert_eq!(
            warning_counted(&services, &faro, &calls, feed()).await,
            None
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn ipma_unreachable_means_no_warning() {
        let services = Services::default();
        let calls = Arc::new(AtomicU32::new(0));
        let lisbon = at("Lisbon", 38.72509, -9.1498);
        for _ in 0..3 {
            let down = Err(FetchError::Http("could not connect".into()));
            assert_eq!(
                warning_counted(&services, &lisbon, &calls, down).await,
                None
            );
        }
        // Backing off: one attempt, not one per render.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hanging_ipma_never_delays_the_answer() {
        let services = Services::default();
        let lisbon = at("Lisbon", 38.72509, -9.1498);
        let w = services.warning_from(&lisbon, noon(), std::future::pending);
        assert_eq!(w, None);
        // Nor does a later render start another fetch while that one hangs.
        let calls = Arc::new(AtomicU32::new(0));
        for _ in 0..3 {
            assert_eq!(
                warning_counted(&services, &lisbon, &calls, Ok(vec![])).await,
                None
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
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
