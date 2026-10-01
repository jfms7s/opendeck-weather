//! Pure builders from fetched data + an instance's view state to the `Card`
//! it should show. No I/O and no clock reads - `now` is the location's local
//! wall-clock time, passed in - so every screen is unit-testable.

use crate::card::Card;
use crate::glyphs::Glyph;
use crate::model::{AirQuality, AqiScale, Forecast, Units};
use crate::wmo::Condition;
use chrono::{Duration, NaiveDateTime};

/// How far ahead a Weather dial can scroll, in hours.
pub const MAX_HOUR_OFFSET: i32 = 24;
/// Pages an Air Quality instance cycles through: the AQI, then pollutants.
pub const AIR_QUALITY_PAGES: i32 = 5;

/// Per-instance, in-memory-only UI state layered on top of the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct View {
    /// Hours ahead (Weather), days past the configured day (Forecast), or
    /// the page (Air Quality).
    pub offset: i32,
    /// Alternate "details" screen toggled by a key or dial press.
    pub detail: bool,
}

fn temp(t: f64) -> String {
    // `as i64` turns a rounded -0.0 into plain 0, so it never shows "-0°".
    format!("{}°", t.round() as i64)
}

fn high_low(max: f64, min: f64) -> String {
    format!("{}/{}", temp(max), temp(min))
}

fn weather_glyph(code: u8, is_day: bool) -> Glyph {
    Glyph::Weather {
        condition: Condition::from_code(code),
        is_day,
    }
}

/// Precipitation chance when there is one, else the condition name - the
/// more useful of the two for a single short line.
fn chance_or_condition(probability: Option<f64>, code: u8) -> String {
    match probability {
        Some(p) if p >= 1.0 => format!("Rain {}%", p.round() as i64),
        _ => Condition::from_code(code).label().to_string(),
    }
}

pub fn no_location() -> Card {
    Card {
        glyph: Glyph::Pin,
        value: "--".into(),
        accent: None,
        label: "Set location".into(),
        detail: "in the action settings".into(),
    }
}

pub fn no_data(location_name: &str) -> Card {
    Card {
        glyph: Glyph::Offline,
        value: "--".into(),
        accent: None,
        label: "No data".into(),
        detail: location_name.to_string(),
    }
}

/// Index of the forecast entry for the local hour containing `now`.
fn current_hour_index(f: &Forecast, now: NaiveDateTime) -> Option<usize> {
    f.hourly
        .iter()
        .position(|h| h.time + Duration::hours(1) > now)
}

/// Index of `now`'s local date in the daily forecast.
fn today_index(f: &Forecast, now: NaiveDateTime) -> Option<usize> {
    f.daily.iter().position(|d| d.date >= now.date())
}

pub fn weather(
    f: &Forecast,
    location_name: &str,
    units: Units,
    view: View,
    now: NaiveDateTime,
) -> Card {
    let c = &f.current;

    if view.detail {
        let (value, detail) = match today_index(f, now).map(|i| &f.daily[i]) {
            Some(d) => (
                high_low(d.max, d.min),
                format!(
                    "Wind {} {}",
                    c.wind_speed.round() as i64,
                    units.wind_label()
                ),
            ),
            None => (temp(c.temperature), String::new()),
        };
        return Card {
            glyph: weather_glyph(c.code, c.is_day),
            value,
            accent: None,
            label: format!("Feels {}", temp(c.apparent_temperature)),
            detail: format!("{detail} · Hum {}%", c.humidity.round() as i64),
        };
    }

    if view.offset > 0
        && let Some(hour) =
            current_hour_index(f, now).and_then(|i| f.hourly.get(i + view.offset as usize))
    {
        return Card {
            glyph: weather_glyph(hour.code, hour.is_day),
            value: temp(hour.temperature),
            accent: None,
            label: hour.time.format("%H:%M").to_string(),
            detail: chance_or_condition(hour.precipitation_probability, hour.code),
        };
    }

    Card {
        glyph: weather_glyph(c.code, c.is_day),
        value: temp(c.temperature),
        accent: None,
        label: Condition::from_code(c.code).label().to_string(),
        detail: location_name.to_string(),
    }
}

/// Days a Forecast instance can show: today plus whatever the forecast
/// still covers after it.
pub fn forecast_days(f: &Forecast, now: NaiveDateTime) -> i32 {
    today_index(f, now).map_or(0, |i| (f.daily.len() - i) as i32)
}

pub fn forecast(f: &Forecast, base_day: u8, view: View, now: NaiveDateTime) -> Option<Card> {
    let today = today_index(f, now)?;
    let available = forecast_days(f, now);
    let ahead = (i32::from(base_day) + view.offset).rem_euclid(available);
    let day = &f.daily[today + ahead as usize];

    let name = match ahead {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        _ => day.date.format("%a %-d").to_string(),
    };

    let (label, detail) = if view.detail {
        let clock = |t: Option<NaiveDateTime>| {
            t.map_or("--".to_string(), |t| t.format("%H:%M").to_string())
        };
        (
            Condition::from_code(day.code).label().to_string(),
            format!("↑{} ↓{}", clock(day.sunrise), clock(day.sunset)),
        )
    } else {
        (
            name,
            chance_or_condition(day.precipitation_probability, day.code),
        )
    };

    Some(Card {
        glyph: weather_glyph(day.code, true),
        value: high_low(day.max, day.min),
        accent: None,
        label,
        detail,
    })
}

const AQI_GOOD: &str = "#22c55e";
const AQI_FAIR: &str = "#84cc16";
const AQI_MODERATE: &str = "#eab308";
const AQI_POOR: &str = "#f97316";
const AQI_VERY_POOR: &str = "#ef4444";
const AQI_HAZARDOUS: &str = "#a855f7";
const AQI_UNKNOWN: &str = "#6b7280";

/// Category name and color, using each scale's own official breakpoints.
pub fn aqi_category(scale: AqiScale, aqi: f64) -> (&'static str, &'static str) {
    match scale {
        AqiScale::Us => match aqi {
            a if a <= 50.0 => ("Good", AQI_GOOD),
            a if a <= 100.0 => ("Moderate", AQI_MODERATE),
            a if a <= 150.0 => ("Unhealthy (SG)", AQI_POOR),
            a if a <= 200.0 => ("Unhealthy", AQI_VERY_POOR),
            a if a <= 300.0 => ("Very unhealthy", AQI_HAZARDOUS),
            _ => ("Hazardous", AQI_HAZARDOUS),
        },
        AqiScale::European => match aqi {
            a if a <= 20.0 => ("Good", AQI_GOOD),
            a if a <= 40.0 => ("Fair", AQI_FAIR),
            a if a <= 60.0 => ("Moderate", AQI_MODERATE),
            a if a <= 80.0 => ("Poor", AQI_POOR),
            a if a <= 100.0 => ("Very poor", AQI_VERY_POOR),
            _ => ("Extremely poor", AQI_HAZARDOUS),
        },
    }
}

fn concentration(v: Option<f64>) -> String {
    match v {
        Some(v) if v < 10.0 => format!("{v:.1}"),
        Some(v) => format!("{}", v.round() as i64),
        None => "--".into(),
    }
}

pub fn air_quality(aq: &AirQuality, scale: AqiScale, view: View) -> Card {
    let (aqi, scale_name) = match scale {
        AqiScale::Us => (aq.us_aqi, "US AQI"),
        AqiScale::European => (aq.european_aqi, "European AQI"),
    };
    let (category, color) = aqi.map_or(("No data", AQI_UNKNOWN), |a| aqi_category(scale, a));
    let glyph = Glyph::Ring(color);

    let pollutant = |value: Option<f64>, name: &str| Card {
        glyph,
        value: concentration(value),
        accent: None,
        label: name.to_string(),
        detail: "µg/m³".into(),
    };

    match view.offset.rem_euclid(AIR_QUALITY_PAGES) {
        1 => pollutant(aq.pm2_5, "PM2.5"),
        2 => pollutant(aq.pm10, "PM10"),
        3 => pollutant(aq.ozone, "Ozone"),
        4 => pollutant(aq.nitrogen_dioxide, "NO₂"),
        _ => Card {
            glyph,
            value: aqi.map_or("--".into(), |a| (a.round() as i64).to_string()),
            accent: aqi.map(|_| color),
            label: category.to_string(),
            detail: scale_name.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Current, Day, Hour};
    use chrono::NaiveDate;

    fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    fn sample() -> Forecast {
        Forecast {
            utc_offset_seconds: 3600,
            current: Current {
                time: at(25, 14, 15),
                temperature: 21.6,
                apparent_temperature: 20.4,
                humidity: 46.0,
                wind_speed: 12.3,
                code: 2,
                is_day: true,
            },
            hourly: (0..48)
                .map(|h| Hour {
                    time: at(25 + h / 24, h % 24, 0),
                    temperature: f64::from(h),
                    code: if h == 16 { 61 } else { 0 },
                    precipitation_probability: Some(if h == 16 { 40.0 } else { 0.0 }),
                    is_day: (7..20).contains(&(h % 24)),
                })
                .collect(),
            daily: (0..3)
                .map(|d| Day {
                    date: NaiveDate::from_ymd_opt(2026, 9, 25 + d).unwrap(),
                    code: [2, 61, 95][d as usize],
                    max: 28.0 + f64::from(d),
                    min: 17.6,
                    precipitation_probability: Some([0.0, 70.0, 90.0][d as usize]),
                    sunrise: Some(at(25 + d, 7, 26)),
                    sunset: Some(at(25 + d, 19, 28)),
                })
                .collect(),
        }
    }

    fn now() -> NaiveDateTime {
        at(25, 14, 40)
    }

    #[test]
    fn weather_at_rest_shows_current_temperature_and_condition() {
        let c = weather(&sample(), "Lisbon", Units::Metric, View::default(), now());
        assert_eq!(c.value, "22°");
        assert_eq!(c.label, "Partly cloudy");
        assert_eq!(c.detail, "Lisbon");
    }

    #[test]
    fn weather_detail_shows_todays_range_and_feels_like() {
        let view = View {
            offset: 0,
            detail: true,
        };
        let c = weather(&sample(), "Lisbon", Units::Imperial, view, now());
        assert_eq!(c.value, "28°/18°");
        assert_eq!(c.label, "Feels 20°");
        assert_eq!(c.detail, "Wind 12 mph · Hum 46%");
    }

    #[test]
    fn weather_offset_scrolls_hourly_from_the_current_hour() {
        let view = View {
            offset: 2,
            detail: false,
        };
        let c = weather(&sample(), "Lisbon", Units::Metric, view, now());
        assert_eq!(c.label, "16:00");
        assert_eq!(c.value, "16°");
        assert_eq!(c.detail, "Rain 40%");
    }

    #[test]
    fn weather_offset_past_the_forecast_falls_back_to_now() {
        let view = View {
            offset: 1000,
            detail: false,
        };
        let c = weather(&sample(), "Lisbon", Units::Metric, view, now());
        assert_eq!(c.value, "22°");
    }

    #[test]
    fn temperatures_never_render_negative_zero() {
        assert_eq!(temp(-0.4), "0°");
        assert_eq!(temp(-0.6), "-1°");
    }

    #[test]
    fn forecast_names_today_and_tomorrow_and_dates_after() {
        let f = sample();
        let day = |base, offset| {
            forecast(
                &f,
                base,
                View {
                    offset,
                    detail: false,
                },
                now(),
            )
            .unwrap()
        };
        assert_eq!(day(0, 0).label, "Today");
        assert_eq!(day(1, 0).label, "Tomorrow");
        assert_eq!(day(1, 0).value, "29°/18°");
        assert_eq!(day(1, 0).detail, "Rain 70%");
        assert_eq!(day(1, 1).label, "Sun 27");
    }

    #[test]
    fn forecast_wraps_around_the_available_days() {
        let f = sample();
        let c = forecast(
            &f,
            1,
            View {
                offset: 2,
                detail: false,
            },
            now(),
        )
        .unwrap();
        assert_eq!(c.label, "Today");
        // A configured day beyond the horizon wraps too, rather than panicking.
        assert!(forecast(&f, 6, View::default(), now()).is_some());
    }

    #[test]
    fn forecast_detail_shows_sunrise_and_sunset() {
        let c = forecast(
            &sample(),
            0,
            View {
                offset: 0,
                detail: true,
            },
            now(),
        )
        .unwrap();
        assert_eq!(c.label, "Partly cloudy");
        assert_eq!(c.detail, "↑07:26 ↓19:28");
    }

    #[test]
    fn forecast_with_only_past_days_has_nothing_to_show() {
        assert!(forecast(&sample(), 0, View::default(), at(30, 0, 0)).is_none());
    }

    #[test]
    fn us_aqi_breakpoints() {
        assert_eq!(aqi_category(AqiScale::Us, 50.0).0, "Good");
        assert_eq!(aqi_category(AqiScale::Us, 51.0).0, "Moderate");
        assert_eq!(aqi_category(AqiScale::Us, 151.0).0, "Unhealthy");
        assert_eq!(aqi_category(AqiScale::Us, 301.0).0, "Hazardous");
    }

    #[test]
    fn european_aqi_breakpoints() {
        assert_eq!(aqi_category(AqiScale::European, 20.0).0, "Good");
        assert_eq!(aqi_category(AqiScale::European, 24.0).0, "Fair");
        assert_eq!(aqi_category(AqiScale::European, 101.0).0, "Extremely poor");
    }

    fn aq() -> AirQuality {
        AirQuality {
            us_aqi: Some(43.0),
            european_aqi: Some(24.0),
            pm2_5: Some(6.54),
            pm10: Some(10.9),
            ozone: Some(67.0),
            nitrogen_dioxide: None,
        }
    }

    #[test]
    fn air_quality_first_page_is_the_colored_index() {
        let c = air_quality(&aq(), AqiScale::Us, View::default());
        assert_eq!(c.value, "43");
        assert_eq!(c.label, "Good");
        assert_eq!(c.accent, Some(AQI_GOOD));
        assert_eq!(c.glyph, Glyph::Ring(AQI_GOOD));
        let eu = air_quality(&aq(), AqiScale::European, View::default());
        assert_eq!((eu.value.as_str(), eu.label.as_str()), ("24", "Fair"));
    }

    #[test]
    fn air_quality_pages_through_pollutants_and_wraps() {
        let page = |offset| {
            air_quality(
                &aq(),
                AqiScale::Us,
                View {
                    offset,
                    detail: false,
                },
            )
        };
        assert_eq!(
            (page(1).label.as_str(), page(1).value.as_str()),
            ("PM2.5", "6.5")
        );
        assert_eq!(page(2).value, "11");
        assert_eq!(page(4).value, "--");
        assert_eq!(page(5).label, "Good");
        assert_eq!(page(-1).label, "NO₂");
    }

    #[test]
    fn missing_aqi_is_neutral_not_good() {
        let c = air_quality(&AirQuality::default(), AqiScale::Us, View::default());
        assert_eq!(c.value, "--");
        assert_eq!(c.label, "No data");
        assert_eq!(c.accent, None);
    }
}
