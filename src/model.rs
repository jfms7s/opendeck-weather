//! Domain types shared by every action: the per-instance settings the
//! property inspector writes, and the weather/air-quality snapshots the
//! Open-Meteo client parses its responses into.

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeDelta, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

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
    /// The full search-result label ("Lisbon, Ohio, United States"), so the
    /// property inspector can show which "Lisbon" this is. Display only;
    /// absent in settings saved by v0.1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// One settings shape for all three actions - they share a property
/// inspector, and each action simply ignores the fields it doesn't use.
///
/// Deserialization never fails: each field that is missing or unreadable
/// falls back to its default on its own. openaction replaces the *whole*
/// settings object with `Default` when deserializing fails, so one renamed
/// variant or mistyped field would otherwise wipe a saved location.
#[derive(Debug, Clone, PartialEq, Serialize)]
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

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Settings::from_json(&Value::deserialize(deserializer)?))
    }
}

type Fields = serde_json::Map<String, Value>;

/// The object's fields, or none (with a warning) if it isn't an object.
fn fields(json: &Value) -> Fields {
    match json {
        Value::Object(fields) => fields.clone(),
        Value::Null => Fields::new(),
        _ => {
            log::warn!("settings are not an object; using defaults");
            Fields::new()
        }
    }
}

/// One field, or `None` when it is missing, null or unreadable.
fn field<T: DeserializeOwned>(fields: &Fields, key: &str) -> Option<T> {
    let value = fields.get(key).filter(|v| !v.is_null())?;
    T::deserialize(value)
        .inspect_err(|e| log::warn!("ignoring unreadable setting {key:?}: {e}"))
        .ok()
}

impl Settings {
    fn from_json(json: &Value) -> Self {
        let defaults = Settings::default();
        let fields = &fields(json);
        Settings {
            location: field(fields, "location"),
            units: field(fields, "units").unwrap_or(defaults.units),
            day: field(fields, "day").unwrap_or(defaults.day),
            aqi_scale: field(fields, "aqi_scale").unwrap_or(defaults.aqi_scale),
        }
    }
}

/// Plugin-wide settings (OpenDeck's global settings): a default location
/// for every instance that has none of its own. Parsed as leniently as
/// `Settings`.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct GlobalSettings {
    pub default_location: Option<Location>,
}

impl<'de> Deserialize<'de> for GlobalSettings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields = &fields(&Value::deserialize(deserializer)?);
        Ok(GlobalSettings {
            default_location: field(fields, "default_location"),
        })
    }
}

/// All times are the location's local wall-clock time, exactly as
/// Open-Meteo returns them with `timezone=auto`.
#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    pub utc_offset_seconds: i32,
    /// `None` when the response's current conditions were incomplete.
    pub current: Option<Current>,
    pub hourly: Vec<Hour>,
    pub daily: Vec<Day>,
}

impl Forecast {
    /// The location's wall-clock time at `utc`, from the offset this
    /// forecast reported - independent of the machine's own timezone.
    pub fn local_now(&self, utc: DateTime<Utc>) -> NaiveDateTime {
        utc.naive_utc() + TimeDelta::seconds(i64::from(self.utc_offset_seconds))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Current {
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

    fn lisbon() -> Location {
        Location {
            name: "Lisbon".into(),
            latitude: 38.72509,
            longitude: -9.1498,
            label: None,
        }
    }

    fn parse(json: &str) -> Settings {
        serde_json::from_str(json).expect("settings deserialization never fails")
    }

    #[test]
    fn the_default_forecast_day_is_tomorrow() {
        // The property inspector shows the same default; the contract test
        // in `actions` checks its fallbacks against `Settings::default()`.
        assert_eq!(Settings::default().day, 1);
        assert_eq!(Settings::default().units, Units::Metric);
        assert_eq!(Settings::default().aqi_scale, AqiScale::Us);
        assert_eq!(Settings::default().location, None);
    }

    #[test]
    fn settings_saved_by_v0_1_still_load() {
        // Shapes as OpenDeck stores them (fields only appear once the
        // property inspector has written them).
        let saved: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/fixtures/settings-v0.1.json")).unwrap();
        let loaded: Vec<Settings> = saved
            .into_iter()
            .map(|v| serde_json::from_value(v).unwrap())
            .collect();
        assert_eq!(
            loaded,
            vec![
                Settings {
                    location: Some(lisbon()),
                    ..Settings::default()
                },
                Settings {
                    location: Some(lisbon()),
                    aqi_scale: AqiScale::European,
                    ..Settings::default()
                },
                Settings {
                    location: Some(lisbon()),
                    units: Units::Imperial,
                    day: 3,
                    aqi_scale: AqiScale::Us,
                },
            ]
        );
    }

    #[test]
    fn one_bad_field_falls_back_alone_and_keeps_the_location() {
        // openaction replaces the *whole* settings object with Default when
        // deserialization fails, which would silently drop the location.
        let loc = r#""location":{"name":"Lisbon","latitude":38.72509,"longitude":-9.1498}"#;
        for bad in [
            r#""units":"Metric""#,
            r#""day":"1""#,
            r#""day":-1"#,
            r#""aqi_scale":42"#,
        ] {
            let s = parse(&format!("{{{loc},{bad}}}"));
            assert_eq!(s.location, Some(lisbon()), "{bad}");
        }
        assert_eq!(
            parse(&format!(r#"{{{loc},"units":"Metric"}}"#)).units,
            Units::Metric
        );
        assert_eq!(parse(&format!(r#"{{{loc},"day":-1}}"#)).day, 1);
        assert_eq!(
            parse(r#"{"units":"imperial","day":2}"#),
            Settings {
                units: Units::Imperial,
                day: 2,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn a_location_missing_coordinates_is_dropped_but_other_fields_survive() {
        let s = parse(r#"{"location":{"name":"Lisbon","latitude":38.7},"units":"imperial"}"#);
        assert_eq!(s.location, None);
        assert_eq!(s.units, Units::Imperial);
    }

    #[test]
    fn settings_that_are_not_an_object_are_the_default() {
        for json in ["null", "[]", "\"x\"", "3"] {
            assert_eq!(parse(json), Settings::default(), "{json}");
        }
    }

    #[test]
    fn a_location_label_round_trips_and_is_optional() {
        let s = parse(
            r#"{"location":{"name":"Lisbon","latitude":38.72509,"longitude":-9.1498,"label":"Lisbon, Lisbon District, Portugal"}}"#,
        );
        let location = s.location.clone().unwrap();
        assert_eq!(
            location.label.as_deref(),
            Some("Lisbon, Lisbon District, Portugal")
        );
        assert_eq!(parse(&serde_json::to_string(&s).unwrap()), s);
        let unlabelled = serde_json::to_value(lisbon()).unwrap();
        assert!(unlabelled.get("label").is_none());
    }

    #[test]
    fn global_settings_are_lenient_too() {
        let g: GlobalSettings = serde_json::from_str(
            r#"{"default_location":{"name":"Lisbon","latitude":38.72509,"longitude":-9.1498}}"#,
        )
        .unwrap();
        assert_eq!(g.default_location, Some(lisbon()));
        for json in ["{}", "null", r#"{"default_location":"nope"}"#, "[1]"] {
            let g: GlobalSettings = serde_json::from_str(json).unwrap();
            assert_eq!(g, GlobalSettings::default(), "{json}");
        }
    }

    #[test]
    fn local_now_applies_the_forecasts_utc_offset() {
        let f = Forecast {
            utc_offset_seconds: 3600,
            current: None,
            hourly: vec![],
            daily: vec![],
        };
        let utc = chrono::DateTime::parse_from_rfc3339("2026-09-25T23:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(f.local_now(utc).to_string(), "2026-09-26 00:30:00");
    }

    #[test]
    fn settings_round_trip_through_serialization() {
        let s = Settings {
            location: Some(lisbon()),
            units: Units::Imperial,
            day: 3,
            aqi_scale: AqiScale::European,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(parse(&json), s);
    }
}
