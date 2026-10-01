//! Domain types shared by every action: the per-instance settings the
//! property inspector writes, and the weather/air-quality snapshots the
//! Open-Meteo client parses its responses into.

use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    /// Celsius, km/h.
    #[default]
    Metric,
    /// Fahrenheit, mph.
    Imperial,
}

impl Units {
    pub fn wind_label(self) -> &'static str {
        match self {
            Units::Metric => "km/h",
            Units::Imperial => "mph",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AqiScale {
    #[default]
    Us,
    European,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    /// Short display name, e.g. "Lisbon".
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// One settings shape for all three actions - they share a property
/// inspector, and each action simply ignores the fields it doesn't use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub location: Option<Location>,
    pub units: Units,
    /// Forecast only: which day the key shows at rest, 0 = today.
    pub day: u8,
    /// Air Quality only.
    pub aqi_scale: AqiScale,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            location: None,
            units: Units::default(),
            day: 1,
            aqi_scale: AqiScale::default(),
        }
    }
}

/// All times are the location's local wall-clock time, exactly as
/// Open-Meteo returns them with `timezone=auto`.
#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    pub utc_offset_seconds: i32,
    pub current: Current,
    pub hourly: Vec<Hour>,
    pub daily: Vec<Day>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Current {
    pub time: NaiveDateTime,
    pub temperature: f64,
    pub apparent_temperature: f64,
    pub humidity: f64,
    pub wind_speed: f64,
    pub code: u8,
    pub is_day: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hour {
    pub time: NaiveDateTime,
    pub temperature: f64,
    pub code: u8,
    pub precipitation_probability: Option<f64>,
    pub is_day: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub code: u8,
    pub max: f64,
    pub min: f64,
    pub precipitation_probability: Option<f64>,
    pub sunrise: Option<NaiveDateTime>,
    pub sunset: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AirQuality {
    pub us_aqi: Option<f64>,
    pub european_aqi: Option<f64>,
    pub pm2_5: Option<f64>,
    pub pm10: Option<f64>,
    pub ozone: Option<f64>,
    pub nitrogen_dioxide: Option<f64>,
}

/// A geocoding match, as shown in the property inspector's result list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Place {
    pub name: String,
    /// "Lisbon, Lisbon District, Portugal" - disambiguates same-named places.
    pub label: String,
    pub latitude: f64,
    pub longitude: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_missing_key_deserialization() {
        // openaction falls back to Default::default() when settings JSON
        // fails to deserialize at all, not just on missing fields - both
        // paths must land on the same value.
        let from_missing_keys: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(from_missing_keys, Settings::default());
    }

    #[test]
    fn settings_round_trip_the_property_inspector_wire_format() {
        let json = r#"{"location":{"name":"Lisbon","latitude":38.7,"longitude":-9.1},"units":"imperial","day":3,"aqi_scale":"european"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.units, Units::Imperial);
        assert_eq!(s.day, 3);
        assert_eq!(s.aqi_scale, AqiScale::European);
        assert_eq!(s.location.unwrap().name, "Lisbon");
    }
}
