//! Cached Open-Meteo access shared by all three actions, so every key and
//! dial for the same place and units shares one request per refresh window.
//! Also holds the plugin-wide default location (global settings), and
//! IPMA's warnings for Portugal (one cached feed for the whole country).

use crate::cache::{Cache, CachePolicy, Cached};
use crate::ipma::{self, Warning};
use crate::model::{AirQuality, Forecast, Location, Place, Settings, Units};
use crate::open_meteo::{FetchError, OpenMeteo};
use chrono::{DateTime, Utc};
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Fetches IPMA's warnings feed. A seam so tests never reach the network.
pub type WarningsFetch = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<Vec<Warning>, FetchError>> + Send>>
        + Send
        + Sync,
>;

#[cfg(not(test))]
fn default_warnings_fetch(api: OpenMeteo) -> WarningsFetch {
    Arc::new(move || {
        let api = api.clone();
        Box::pin(async move { ipma::fetch(&api).await })
    })
}

/// The suite runs offline: by default a test's `Services` sees no warnings
/// (tests that need some use `with_warnings_fetch`).
#[cfg(test)]
fn default_warnings_fetch(_: OpenMeteo) -> WarningsFetch {
    Arc::new(|| Box::pin(async { Ok(Vec::new()) }))
}

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
    fetch_warnings: WarningsFetch,
    default_location: RwLock<Option<Location>>,
}

impl Default for Services {
    fn default() -> Self {
        let api = OpenMeteo::default();
        Self::with_warnings_fetch(api.clone(), default_warnings_fetch(api))
    }
}

impl Services {
    fn with_warnings_fetch(api: OpenMeteo, fetch_warnings: WarningsFetch) -> Self {
        Self {
            api,
            forecasts: Cache::new(POLICY),
            air: Cache::new(POLICY),
            warnings: Arc::new(Cache::new(POLICY)),
            fetch_warnings,
            default_location: RwLock::new(None),
        }
    }

    /// A `Services` whose IPMA feed comes from `fetch` (tests elsewhere).
    #[cfg(test)]
    pub fn with_warnings(fetch: WarningsFetch) -> Self {
        Self::with_warnings_fetch(OpenMeteo::default(), fetch)
    }

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
    /// lands, or once a refresh has failed (an old feed could still show a
    /// warning IPMA has lifted, so an outage shows none). Never waits on
    /// the network: when a fetch is due it runs in the background, and the
    /// next render shows the result.
    pub fn warning(&self, location: &Location, utc_now: DateTime<Utc>) -> Option<ipma::Active> {
        let area = ipma::area_for(location)?;
        let (cached, due) = self.warnings.peek(&());
        if due {
            let warnings = self.warnings.clone();
            let fetch = self.fetch_warnings.clone();
            // A failure is logged once per attempt by the cache.
            tokio::spawn(async move { drop(warnings.get(&(), || fetch()).await) });
        }
        let feed = cached.filter(|c| !c.stale)?;
        ipma::active(&feed.value, area, utc_now)
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

    /// A fetcher that counts its calls and answers from `results` in turn
    /// (the last one repeats); `None` hangs forever.
    fn counting(
        calls: &Arc<AtomicU32>,
        results: Vec<Option<Result<Vec<Warning>, FetchError>>>,
    ) -> WarningsFetch {
        let calls = calls.clone();
        Arc::new(move || {
            let n = calls.fetch_add(1, Ordering::SeqCst) as usize;
            let result = results[n.min(results.len() - 1)].clone();
            Box::pin(async move {
                match result {
                    Some(r) => r,
                    None => std::future::pending().await,
                }
            })
        })
    }

    /// Lets spawned background fetches run to completion (or to a hang).
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    fn lisbon() -> Location {
        at("Lisbon", 38.72509, -9.1498)
    }

    #[tokio::test(start_paused = true)]
    async fn a_location_outside_portugal_makes_no_ipma_request() {
        let calls = Arc::new(AtomicU32::new(0));
        let services =
            Services::with_warnings(counting(&calls, vec![Some(Ok(vec![rain_warning("LSB")]))]));
        let madrid = at("Madrid", 40.4168, -3.7038);
        for _ in 0..3 {
            assert_eq!(services.warning(&madrid, noon()), None);
            settle().await;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn warnings_are_fetched_in_the_background_and_shown_next_time() {
        let calls = Arc::new(AtomicU32::new(0));
        let feed = vec![rain_warning("LSB"), rain_warning("PTO")];
        let services = Services::with_warnings(counting(&calls, vec![Some(Ok(feed))]));
        // The first render doesn't wait: no badge yet, a fetch starts.
        assert_eq!(services.warning(&lisbon(), noon()), None);
        settle().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // The next one shows it, from the cache.
        let w = services.warning(&lisbon(), noon()).unwrap();
        assert_eq!((w.kind, w.level), (Kind::Rain, Level::Orange));
        // Another area of the same feed: no new request, no warning.
        assert_eq!(
            services.warning(&at("Faro", 37.0194, -7.9304), noon()),
            None
        );
        settle().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn ipma_unreachable_means_no_warning() {
        let calls = Arc::new(AtomicU32::new(0));
        let down = Err(FetchError::Http("could not connect".into()));
        let services = Services::with_warnings(counting(&calls, vec![Some(down)]));
        for _ in 0..3 {
            assert_eq!(services.warning(&lisbon(), noon()), None);
            settle().await;
        }
        // Backing off: one attempt, not one per render.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_refresh_drops_the_old_feed_rather_than_show_it() {
        // IPMA may have lifted the warning meanwhile: an outage shows none.
        let calls = Arc::new(AtomicU32::new(0));
        let services = Services::with_warnings(counting(
            &calls,
            vec![
                Some(Ok(vec![rain_warning("LSB")])),
                Some(Err(FetchError::Http("timed out".into()))),
            ],
        ));
        services.warning(&lisbon(), noon());
        settle().await;
        assert!(services.warning(&lisbon(), noon()).is_some());
        // Past the TTL the old feed still shows while the refresh runs...
        tokio::time::advance(POLICY.ttl).await;
        assert!(services.warning(&lisbon(), noon()).is_some());
        settle().await;
        // ...but not once it has failed.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(services.warning(&lisbon(), noon()), None);
        tokio::time::advance(POLICY.retry_after).await;
        assert_eq!(services.warning(&lisbon(), noon()), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hanging_ipma_neither_blocks_nor_piles_up_requests() {
        let calls = Arc::new(AtomicU32::new(0));
        let services = Services::with_warnings(counting(&calls, vec![None]));
        for _ in 0..5 {
            assert_eq!(services.warning(&lisbon(), noon()), None);
            settle().await;
            tokio::time::advance(POLICY.retry_after).await;
        }
        // One request in flight, hanging; no second one while it does.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
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
