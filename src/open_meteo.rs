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
const FORECAST_DAYS: &str = "7";
const SEARCH_RESULTS: &str = "8";

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
            .build()
            .expect("reqwest client with static config");
        Self { client }
    }
}

impl OpenMeteo {
    async fn get(&self, url: Url) -> Result<String, FetchError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| FetchError::Http(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(FetchError::Http(format!("HTTP {status}")));
        }
        response
            .text()
            .await
            .map_err(|e| FetchError::Http(e.to_string()))
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
        ("forecast_days", FORECAST_DAYS.into()),
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
    current: RawCurrent,
    hourly: RawHourly,
    daily: RawDaily,
}

#[derive(Deserialize)]
struct RawCurrent {
    time: String,
    temperature_2m: f64,
    apparent_temperature: f64,
    relative_humidity_2m: f64,
    wind_speed_10m: f64,
    weather_code: u8,
    is_day: u8,
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

pub fn parse_forecast(json: &str) -> Result<Forecast, FetchError> {
    let raw: RawForecast =
        serde_json::from_str(json).map_err(|e| FetchError::Parse(e.to_string()))?;

    let current = Current {
        time: local_time(&raw.current.time)?,
        temperature: raw.current.temperature_2m,
        apparent_temperature: raw.current.apparent_temperature,
        humidity: raw.current.relative_humidity_2m,
        wind_speed: raw.current.wind_speed_10m,
        code: raw.current.weather_code,
        is_day: raw.current.is_day != 0,
    };

    // An hour or day missing its temperature or code can't be drawn, so it
    // is dropped rather than failing the whole forecast.
    let h = &raw.hourly;
    let mut hourly = Vec::with_capacity(h.time.len());
    for (i, time) in h.time.iter().enumerate() {
        let (Some(temperature), Some(code)) = (at(&h.temperature_2m, i), at(&h.weather_code, i))
        else {
            continue;
        };
        hourly.push(Hour {
            time: local_time(time)?,
            temperature,
            code,
            precipitation_probability: at(&h.precipitation_probability, i),
            is_day: at(&h.is_day, i).is_none_or(|d| d != 0),
        });
    }

    let d = &raw.daily;
    let mut daily = Vec::with_capacity(d.time.len());
    for (i, date) in d.time.iter().enumerate() {
        let (Some(code), Some(max), Some(min)) = (
            at(&d.weather_code, i),
            at(&d.temperature_2m_max, i),
            at(&d.temperature_2m_min, i),
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
            date: local_date(date)?,
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

    const FORECAST: &str = include_str!("../tests/fixtures/forecast.json");
    const AIR_QUALITY: &str = include_str!("../tests/fixtures/airquality.json");
    const GEOCODE: &str = include_str!("../tests/fixtures/geocode.json");

    fn lisbon() -> Location {
        Location {
            name: "Lisbon".into(),
            latitude: 38.72,
            longitude: -9.13,
        }
    }

    #[test]
    fn parses_a_real_forecast_response() {
        let f = parse_forecast(FORECAST).unwrap();
        assert_eq!(f.utc_offset_seconds, 3600);
        assert_eq!(f.current.code, 2);
        assert!(!f.current.is_day);
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

    #[test]
    fn geocoding_url_encodes_the_query() {
        let url = geocoding_url(" São Paulo ").to_string();
        assert!(url.contains("name=S%C3%A3o+Paulo&"), "{url}");
    }
}
