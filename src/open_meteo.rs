//! Open-Meteo client: forecast, air quality and geocoding. Free, no API key
//! or account needed - same promise as Elgato's own Weather plugin.
//!
//! The HTTP calls are thin; everything that can be wrong about a response
//! lives in the pure `parse_*` functions, which are tested against real
//! captured responses in `tests/fixtures/`.

use crate::model::{AirQuality, Current, Day, Forecast, Hour, Location, Place, Units};
use chrono::{NaiveDate, NaiveDateTime};
use reqwest::Url;
use serde::Deserialize;
use std::time::Duration;

const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";
const AIR_QUALITY_URL: &str = "https://air-quality-api.open-meteo.com/v1/air-quality";
const GEOCODING_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Real responses are a few KB; anything this big is not one of them.
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// How many days the Forecast action can show (today included).
pub const FORECAST_DAYS: u8 = 7;
const SEARCH_RESULTS: &str = "8";
/// Longest search text forwarded to the geocoder; place names are short.
pub const MAX_QUERY_CHARS: usize = 100;

/// Open-Meteo's own local-time format with `timezone=auto`: no seconds, no
/// offset (the offset comes separately as `utc_offset_seconds`).
const LOCAL_TIME_FORMAT: &str = "%Y-%m-%dT%H:%M";

#[derive(Debug, Clone, thiserror::Error)]
pub enum FetchError {
    #[error("request failed: {0}")]
    Http(String),
    #[error("unexpected response: {0}")]
    Parse(String),
}

pub struct OpenMeteo {
    client: reqwest::Client,
}

impl Default for OpenMeteo {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("opendeck-weather/", env!("CARGO_PKG_VERSION")))
            // The coordinates travel in the query string: never over plain
            // HTTP, and none of the endpoints is expected to redirect.
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client with static config");
        Self { client }
    }
}

impl OpenMeteo {
    async fn get(&self, url: Url) -> Result<String, FetchError> {
        let mut response = self.client.get(url).send().await.map_err(http_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(FetchError::Http(format!("HTTP {status}")));
        }
        if !fits(response.content_length(), MAX_BODY_BYTES) {
            return Err(FetchError::Parse(format!(
                "response larger than {MAX_BODY_BYTES} bytes"
            )));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(http_error)? {
            append_capped(&mut body, &chunk, MAX_BODY_BYTES)?;
        }
        String::from_utf8(body).map_err(|e| FetchError::Parse(e.to_string()))
    }

    pub async fn forecast(
        &self,
        location: &Location,
        units: Units,
    ) -> Result<Forecast, FetchError> {
        self.get(forecast_url(location, units))
            .await
            .and_then(|body| parse_forecast(&body))
    }

    pub async fn air_quality(&self, location: &Location) -> Result<AirQuality, FetchError> {
        self.get(air_quality_url(location))
            .await
            .and_then(|body| parse_air_quality(&body))
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Place>, FetchError> {
        self.get(geocoding_url(query))
            .await
            .and_then(|body| parse_places(&body))
    }
}

/// `reqwest` errors end with "for url (...)", and the URL carries the
/// coordinates or the search text: keep that out of logs and the inspector.
fn http_error(e: reqwest::Error) -> FetchError {
    let what = if e.is_timeout() {
        "timed out".to_string()
    } else if e.is_connect() {
        "could not connect".to_string()
    } else {
        e.without_url().to_string()
    };
    FetchError::Http(what)
}

/// Whether a declared body length is within `cap` (unknown lengths are
/// checked while reading instead).
fn fits(content_length: Option<u64>, cap: usize) -> bool {
    content_length.is_none_or(|n| n <= cap as u64)
}

fn append_capped(body: &mut Vec<u8>, chunk: &[u8], cap: usize) -> Result<(), FetchError> {
    if body.len() + chunk.len() > cap {
        return Err(FetchError::Parse(format!(
            "response larger than {cap} bytes"
        )));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn coords(location: &Location) -> [(&'static str, String); 2] {
    [
        ("latitude", location.latitude.to_string()),
        ("longitude", location.longitude.to_string()),
    ]
}

fn forecast_url(location: &Location, units: Units) -> Url {
    let mut params: Vec<(&str, String)> = coords(location).into();
    params.extend([
        (
            "current",
            "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,is_day,wind_speed_10m".into(),
        ),
        (
            "hourly",
            "temperature_2m,weather_code,precipitation_probability,is_day".into(),
        ),
        (
            "daily",
            "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset".into(),
        ),
        ("timezone", "auto".into()),
        ("forecast_days", FORECAST_DAYS.to_string()),
    ]);
    if units == Units::Imperial {
        params.push(("temperature_unit", "fahrenheit".into()));
        params.push(("wind_speed_unit", "mph".into()));
    }
    Url::parse_with_params(FORECAST_URL, &params).expect("static base URL")
}

fn air_quality_url(location: &Location) -> Url {
    let mut params: Vec<(&str, String)> = coords(location).into();
    params.extend([
        (
            "current",
            "us_aqi,european_aqi,pm2_5,pm10,ozone,nitrogen_dioxide".into(),
        ),
        ("timezone", "auto".into()),
    ]);
    Url::parse_with_params(AIR_QUALITY_URL, &params).expect("static base URL")
}

fn geocoding_url(query: &str) -> Url {
    Url::parse_with_params(
        GEOCODING_URL,
        &[
            ("name", query.trim()),
            ("count", SEARCH_RESULTS),
            ("language", "en"),
            ("format", "json"),
        ],
    )
    .expect("static base URL")
}

fn local_time(s: &str) -> Result<NaiveDateTime, FetchError> {
    NaiveDateTime::parse_from_str(s, LOCAL_TIME_FORMAT)
        .map_err(|e| FetchError::Parse(format!("bad time {s:?}: {e}")))
}

fn local_date(s: &str) -> Result<NaiveDate, FetchError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| FetchError::Parse(format!("bad date {s:?}: {e}")))
}

#[derive(Deserialize)]
struct RawForecast {
    utc_offset_seconds: i32,
    #[serde(default)]
    current: RawCurrent,
    hourly: RawHourly,
    daily: RawDaily,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawCurrent {
    temperature_2m: Option<f64>,
    apparent_temperature: Option<f64>,
    relative_humidity_2m: Option<f64>,
    wind_speed_10m: Option<f64>,
    weather_code: Option<u8>,
    is_day: Option<u8>,
}

/// Every series is nullable per entry: Open-Meteo pads models that don't
/// reach the full horizon (precipitation probability especially) with
/// `null` rather than shortening the arrays.
#[derive(Deserialize)]
struct RawHourly {
    time: Vec<String>,
    temperature_2m: Vec<Option<f64>>,
    weather_code: Vec<Option<u8>>,
    precipitation_probability: Vec<Option<f64>>,
    is_day: Vec<Option<u8>>,
}

#[derive(Deserialize)]
struct RawDaily {
    time: Vec<String>,
    weather_code: Vec<Option<u8>>,
    temperature_2m_max: Vec<Option<f64>>,
    temperature_2m_min: Vec<Option<f64>>,
    precipitation_probability_max: Vec<Option<f64>>,
    sunrise: Vec<Option<String>>,
    sunset: Vec<Option<String>>,
}

fn at<T: Copy>(series: &[Option<T>], i: usize) -> Option<T> {
    series.get(i).copied().flatten()
}

/// One rule throughout: a problem with one entry (a null value, a malformed
/// timestamp) drops that entry - or, for `current`, just the current
/// conditions - and only a response that isn't the expected shape at all
/// fails the parse.
pub fn parse_forecast(json: &str) -> Result<Forecast, FetchError> {
    let raw: RawForecast =
        serde_json::from_str(json).map_err(|e| FetchError::Parse(e.to_string()))?;

    let c = &raw.current;
    let current = match (
        c.temperature_2m,
        c.apparent_temperature,
        c.relative_humidity_2m,
        c.wind_speed_10m,
        c.weather_code,
    ) {
        (
            Some(temperature),
            Some(apparent_temperature),
            Some(humidity),
            Some(wind_speed),
            Some(code),
        ) => Some(Current {
            temperature,
            apparent_temperature,
            humidity,
            wind_speed,
            code,
            is_day: c.is_day.is_none_or(|d| d != 0),
        }),
        _ => None,
    };

    // An hour or day missing its temperature or code can't be drawn, so it
    // is dropped rather than failing the whole forecast.
    let h = &raw.hourly;
    let mut hourly = Vec::with_capacity(h.time.len());
    for (i, time) in h.time.iter().enumerate() {
        let (Some(temperature), Some(code), Ok(time)) = (
            at(&h.temperature_2m, i),
            at(&h.weather_code, i),
            local_time(time),
        ) else {
            continue;
        };
        hourly.push(Hour {
            time,
            temperature,
            code,
            precipitation_probability: at(&h.precipitation_probability, i),
            is_day: at(&h.is_day, i).is_none_or(|d| d != 0),
        });
    }

    let d = &raw.daily;
    let mut daily = Vec::with_capacity(d.time.len());
    for (i, date) in d.time.iter().enumerate() {
        let (Some(code), Some(max), Some(min), Ok(date)) = (
            at(&d.weather_code, i),
            at(&d.temperature_2m_max, i),
            at(&d.temperature_2m_min, i),
            local_date(date),
        ) else {
            continue;
        };
        let optional_time = |series: &[Option<String>]| {
            series
                .get(i)
                .and_then(|s| s.as_deref())
                .and_then(|s| local_time(s).ok())
        };
        daily.push(Day {
            date,
            code,
            max,
            min,
            precipitation_probability: at(&d.precipitation_probability_max, i),
            sunrise: optional_time(&d.sunrise),
            sunset: optional_time(&d.sunset),
        });
    }

    Ok(Forecast {
        utc_offset_seconds: raw.utc_offset_seconds,
        current,
        hourly,
        daily,
    })
}

#[derive(Deserialize)]
struct RawAirQualityResponse {
    current: RawAirQuality,
}

#[derive(Deserialize)]
struct RawAirQuality {
    us_aqi: Option<f64>,
    european_aqi: Option<f64>,
    pm2_5: Option<f64>,
    pm10: Option<f64>,
    ozone: Option<f64>,
    nitrogen_dioxide: Option<f64>,
}

pub fn parse_air_quality(json: &str) -> Result<AirQuality, FetchError> {
    let raw: RawAirQualityResponse =
        serde_json::from_str(json).map_err(|e| FetchError::Parse(e.to_string()))?;
    let c = raw.current;
    Ok(AirQuality {
        us_aqi: c.us_aqi,
        european_aqi: c.european_aqi,
        pm2_5: c.pm2_5,
        pm10: c.pm10,
        ozone: c.ozone,
        nitrogen_dioxide: c.nitrogen_dioxide,
    })
}

#[derive(Deserialize)]
struct RawSearch {
    /// Absent entirely (not an empty array) when nothing matches.
    #[serde(default)]
    results: Vec<RawPlace>,
}

#[derive(Deserialize)]
struct RawPlace {
    name: String,
    latitude: f64,
    longitude: f64,
    admin1: Option<String>,
    country: Option<String>,
}

pub fn parse_places(json: &str) -> Result<Vec<Place>, FetchError> {
    let raw: RawSearch =
        serde_json::from_str(json).map_err(|e| FetchError::Parse(e.to_string()))?;
    Ok(raw
        .results
        .into_iter()
        .map(|p| {
            let mut parts = vec![p.name.as_str()];
            // "Lisbon, Lisbon District" reads fine, but skip an admin1 that
            // just repeats the name ("Singapore, Singapore, Singapore").
            if let Some(admin1) = p.admin1.as_deref().filter(|a| *a != p.name) {
                parts.push(admin1);
            }
            if let Some(country) = p.country.as_deref() {
                parts.push(country);
            }
            Place {
                label: parts.join(", "),
                name: p.name.clone(),
                latitude: p.latitude,
                longitude: p.longitude,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    const FORECAST: &str = include_str!("../tests/fixtures/forecast.json");
    const AIR_QUALITY: &str = include_str!("../tests/fixtures/airquality.json");
    const GEOCODE: &str = include_str!("../tests/fixtures/geocode.json");

    fn lisbon() -> Location {
        Location {
            name: "Lisbon".into(),
            latitude: 38.72,
            longitude: -9.13,
            label: None,
        }
    }

    #[test]
    fn parses_a_real_forecast_response() {
        let f = parse_forecast(FORECAST).unwrap();
        assert_eq!(f.utc_offset_seconds, 3600);
        let current = f.current.as_ref().unwrap();
        assert_eq!(current.code, 2);
        assert!(!current.is_day);
        assert_eq!(f.hourly.len(), 7 * 24);
        assert_eq!(f.daily.len(), 7);
        assert!(f.daily[0].max >= f.daily[0].min);
        assert!(f.daily[0].sunrise.unwrap() < f.daily[0].sunset.unwrap());
    }

    #[test]
    fn forecast_drops_entries_missing_required_values_but_keeps_null_probabilities() {
        let json = r#"{
            "utc_offset_seconds": 0,
            "current": {"time":"2026-09-25T10:00","temperature_2m":20,"apparent_temperature":19,
                        "relative_humidity_2m":50,"wind_speed_10m":5,"weather_code":0,"is_day":1},
            "hourly": {"time":["2026-09-25T10:00","2026-09-25T11:00"],
                       "temperature_2m":[20,null],"weather_code":[0,1],
                       "precipitation_probability":[null,null],"is_day":[1,1]},
            "daily": {"time":["2026-09-25"],"weather_code":[0],"temperature_2m_max":[25],
                      "temperature_2m_min":[15],"precipitation_probability_max":[null],
                      "sunrise":[null],"sunset":["2026-09-25T19:00"]}
        }"#;
        let f = parse_forecast(json).unwrap();
        assert_eq!(f.hourly.len(), 1);
        assert_eq!(f.hourly[0].precipitation_probability, None);
        assert_eq!(f.daily[0].sunrise, None);
        assert!(f.daily[0].sunset.is_some());
    }

    #[test]
    fn one_bad_entry_skips_that_entry_instead_of_failing_the_forecast() {
        // Per-entry problems drop the entry; a null in `current` drops only
        // the current conditions (the views fall back to the hourly entry).
        let json = r#"{
            "utc_offset_seconds": 0,
            "current": {"time":"2026-09-25T10:00","temperature_2m":null,"apparent_temperature":19,
                        "relative_humidity_2m":50,"wind_speed_10m":5,"weather_code":0,"is_day":1},
            "hourly": {"time":["garbage","2026-09-25T11:00"],
                       "temperature_2m":[20,21],"weather_code":[0,1],
                       "precipitation_probability":[null,null],"is_day":[1,1]},
            "daily": {"time":["2026-09-25","not-a-date"],"weather_code":[0,1],"temperature_2m_max":[25,26],
                      "temperature_2m_min":[15,16],"precipitation_probability_max":[null,null],
                      "sunrise":[null,null],"sunset":[null,null]}
        }"#;
        let f = parse_forecast(json).unwrap();
        assert_eq!(f.current, None);
        assert_eq!(f.hourly.len(), 1);
        assert_eq!(f.hourly[0].temperature, 21.0);
        assert_eq!(f.daily.len(), 1);
        assert_eq!(f.daily[0].max, 25.0);
    }

    #[test]
    fn parses_a_real_air_quality_response() {
        let aq = parse_air_quality(AIR_QUALITY).unwrap();
        assert_eq!(aq.us_aqi, Some(43.0));
        assert_eq!(aq.european_aqi, Some(24.0));
        assert_eq!(aq.pm2_5, Some(6.5));
    }

    #[test]
    fn parses_geocoding_matches_with_disambiguating_labels() {
        let places = parse_places(GEOCODE).unwrap();
        assert_eq!(places[0].name, "Lisbon");
        assert_eq!(places[0].label, "Lisbon, Lisbon District, Portugal");
        assert_eq!(places[1].label, "Lisbon, Ohio, United States");
    }

    #[test]
    fn no_geocoding_matches_is_an_empty_list_not_an_error() {
        let places = parse_places(r#"{"generationtime_ms":0.47}"#).unwrap();
        assert!(places.is_empty());
    }

    #[test]
    fn malformed_bodies_are_parse_errors() {
        assert!(matches!(parse_forecast("{}"), Err(FetchError::Parse(_))));
        assert!(matches!(
            parse_air_quality("nope"),
            Err(FetchError::Parse(_))
        ));
    }

    #[test]
    fn imperial_forecast_url_requests_fahrenheit_and_mph() {
        let url = forecast_url(&lisbon(), Units::Imperial).to_string();
        assert!(url.contains("temperature_unit=fahrenheit"), "{url}");
        assert!(url.contains("wind_speed_unit=mph"), "{url}");
        let metric = forecast_url(&lisbon(), Units::Metric).to_string();
        assert!(!metric.contains("temperature_unit"), "{metric}");
    }

    /// A local plain-HTTP server that answers every request with a tiny
    /// JSON body, and counts the connections it accepted.
    async fn plain_http_server() -> (Url, Arc<AtomicU32>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicU32::new(0));
        let counter = accepted.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}")
                    .await;
            }
        });
        let url = format!(
            "http://{addr}/v1/forecast?latitude=38.72509&longitude=-9.1498&name=Rua+Augusta"
        );
        (Url::parse(&url).unwrap(), accepted)
    }

    #[tokio::test]
    async fn plain_http_is_refused_and_errors_do_not_echo_the_url() {
        // Coordinates and search text live in the query string; a redirect
        // to plain HTTP must not send them in the clear, and error messages
        // (logged, and shown in the inspector) must not repeat them.
        let (url, accepted) = plain_http_server().await;
        let err = OpenMeteo::default().get(url).await.unwrap_err().to_string();
        assert_eq!(
            accepted.load(Ordering::SeqCst),
            0,
            "connected over plain HTTP"
        );
        for secret in ["38.72509", "-9.1498", "Rua", "127.0.0.1"] {
            assert!(!err.contains(secret), "{secret} leaked into {err:?}");
        }
    }

    #[test]
    fn response_bodies_are_capped() {
        let mut body = Vec::new();
        append_capped(&mut body, &[b'x'; 10], 16).unwrap();
        assert!(matches!(
            append_capped(&mut body, &[b'x'; 7], 16),
            Err(FetchError::Parse(_))
        ));
        assert!(fits(Some(16), 16) && fits(None, 16) && !fits(Some(17), 16));
    }

    #[test]
    fn geocoding_url_encodes_the_query() {
        let url = geocoding_url(" São Paulo ").to_string();
        assert!(url.contains("name=S%C3%A3o+Paulo&"), "{url}");
    }
}
