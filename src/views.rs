//! Pure builders from fetched data + an instance's view state to the `Card`
//! it should show. No I/O and no clock reads - `now` is the location's local
//! wall-clock time, passed in - so every screen is unit-testable.
//!
//! Every builder also takes `stale`: the data is older than the cache TTL
//! (refreshing it failed). Such cards are marked, and the Weather resting
//! screen then prefers the forecast for the current hour over the stale
//! observation.

use crate::card::Card;
use crate::glyphs::Glyph;
use crate::model::{AirQuality, AqiScale, Current, Forecast, Hour, Units};
use crate::palette;
use crate::view_state::{AirQualityPage, ForecastView, WeatherView};
use crate::wmo::Condition;
use chrono::{Duration, NaiveDateTime};

/// Rounds for display. `as` saturates on overflow and maps NaN to 0, which
/// is fine for a temperature or percentage on a key.
#[allow(clippy::cast_possible_truncation)]
fn whole(x: f64) -> i64 {
    x.round() as i64
}

fn temp(t: f64) -> String {
    // `whole` turns a rounded -0.0 into plain 0, so it never shows "-0°".
    format!("{}°", whole(t))
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

fn condition(code: u8) -> String {
    Condition::from_code(code).label().to_string()
}

/// Chance of precipitation worth mentioning (at least 1%), as a whole %.
fn chance(probability: Option<f64>) -> Option<i64> {
    probability.filter(|p| *p >= 1.0).map(whole)
}

/// Precipitation chance when there is one, else the condition name - the
/// more useful of the two for a single short line.
fn chance_or_condition(probability: Option<f64>, code: u8) -> String {
    chance(probability).map_or_else(|| condition(code), |p| format!("Rain {p}%"))
}

pub fn no_location() -> Card {
    Card::new(Glyph::Pin, "--", "Set location", "in the action settings")
}

pub fn no_data(location_name: &str) -> Card {
    Card::new(Glyph::Offline, "--", "No data", location_name)
}

/// The forecast entry for the local hour containing `now`.
fn current_hour(f: &Forecast, now: NaiveDateTime) -> Option<(usize, &Hour)> {
    f.hourly
        .iter()
        .enumerate()
        .find(|(_, h)| h.time + Duration::hours(1) > now)
}

/// Chance of precipitation (percent) from which an hour counts as wet.
const RAIN_THRESHOLD: f64 = 50.0;
/// How far ahead the resting Weather card looks for rain starting or stopping.
const RAIN_LOOKAHEAD_HOURS: usize = 12;

/// When rain next starts or stops, from the hourly forecast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RainOutlook {
    /// Dry now; rain starts in the hour beginning at this time.
    StartsAt(NaiveDateTime),
    /// Raining now; the first dry hour begins at this time.
    StopsAt(NaiveDateTime),
    /// Dry now, and every hour in the lookahead is known to be dry.
    Dry,
    /// Not enough data to say: an hour with no probability (past the model's
    /// horizon), a gap in the series, or rain lasting the whole lookahead.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wetness {
    Wet,
    Dry,
    Unknown,
}

/// Wet if its condition is precipitation or the chance reaches the
/// threshold; dry only when the chance is known. A missing chance never
/// passes for dry.
fn wetness(h: &Hour) -> Wetness {
    match h.precipitation_probability {
        _ if Condition::from_code(h.code).is_precipitation() => Wetness::Wet,
        Some(p) if p >= RAIN_THRESHOLD => Wetness::Wet,
        Some(_) => Wetness::Dry,
        None => Wetness::Unknown,
    }
}

/// Scans the hours after the one containing `now`, up to the lookahead.
/// Whether it is raining now comes from `observed_code` (the observation the
/// card shows) when there is one, else from the current hour's forecast, so
/// the line never contradicts the condition on the card.
fn next_rain(f: &Forecast, now: NaiveDateTime, observed_code: Option<u8>) -> RainOutlook {
    let Some((i, current)) = current_hour(f, now) else {
        return RainOutlook::Unknown;
    };
    let raining = match observed_code {
        Some(code) => Condition::from_code(code).is_precipitation(),
        None => match wetness(current) {
            Wetness::Wet => true,
            Wetness::Dry => false,
            Wetness::Unknown => return RainOutlook::Unknown,
        },
    };
    let (looking_for, found): (Wetness, fn(NaiveDateTime) -> RainOutlook) = if raining {
        (Wetness::Dry, RainOutlook::StopsAt)
    } else {
        (Wetness::Wet, RainOutlook::StartsAt)
    };
    for ahead in 1..=RAIN_LOOKAHEAD_HOURS {
        let expected = current.time + Duration::hours(ahead as i64);
        // The parser drops malformed entries: a missing hour is unknown.
        let w = match f.hourly.get(i + ahead) {
            Some(h) if h.time == expected => wetness(h),
            _ => Wetness::Unknown,
        };
        if w == looking_for {
            return found(expected);
        }
        if w == Wetness::Unknown {
            return RainOutlook::Unknown;
        }
    }
    if raining {
        RainOutlook::Unknown
    } else {
        RainOutlook::Dry
    }
}

/// The rain line, or `None` when the outlook is unknown.
fn rain_line(outlook: RainOutlook) -> Option<String> {
    match outlook {
        RainOutlook::StartsAt(t) => Some(format!("Rain at {}", t.format("%H:%M"))),
        RainOutlook::StopsAt(t) => Some(format!("Rain until {}", t.format("%H:%M"))),
        RainOutlook::Dry => Some(format!("Dry next {RAIN_LOOKAHEAD_HOURS}h")),
        RainOutlook::Unknown => None,
    }
}

/// Index of the first daily entry for `now`'s local date or later.
fn today_index(f: &Forecast, now: NaiveDateTime) -> Option<usize> {
    f.daily.iter().position(|d| d.date >= now.date())
}

pub fn weather(
    f: &Forecast,
    location_name: &str,
    units: Units,
    view: WeatherView,
    now: NaiveDateTime,
    stale: bool,
) -> Card {
    let hour_now = current_hour(f, now);
    // A stale observation is hours old by now, so the forecast for the
    // current hour is the better estimate of "now" (every screen agrees).
    let observed = f.current.as_ref().filter(|_| !stale || hour_now.is_none());

    match view {
        WeatherView::Details => {
            if let Some(card) = weather_details(f, observed, hour_now.map(|(_, h)| h), units, now) {
                return card.marked_stale(stale);
            }
        }
        WeatherView::Hour(ahead) => {
            if let Some(hour) = hour_now.and_then(|(i, _)| f.hourly.get(i + usize::from(ahead))) {
                return Card::new(
                    weather_glyph(hour.code, hour.is_day),
                    temp(hour.temperature),
                    hour.time.format("%H:%M").to_string(),
                    chance_or_condition(hour.precipitation_probability, hour.code),
                )
                .marked_stale(stale);
            }
        }
        WeatherView::Day(ahead) => {
            let days = ForecastView {
                days: i32::from(ahead),
                sun: false,
            };
            if let Some(card) = forecast(f, 0, days, now, stale) {
                return card;
            }
        }
        WeatherView::Now => {}
    }

    // Resting screen.
    let (code, is_day, temperature) = match (observed, hour_now) {
        (Some(c), _) => (c.code, c.is_day, c.temperature),
        (None, Some((_, h))) => (h.code, h.is_day, h.temperature),
        (None, None) => return no_data(location_name),
    };
    // The rain line takes the dial's location line whenever there is
    // something to say; a key gives up the condition only for rain starting
    // or stopping, so a dry day still shows the sky there.
    let outlook = next_rain(f, now, observed.map(|c| c.code));
    let card = Card::new(
        weather_glyph(code, is_day),
        temp(temperature),
        condition(code),
        rain_line(outlook).unwrap_or_else(|| location_name.to_string()),
    );
    let card = if matches!(outlook, RainOutlook::StartsAt(_) | RainOutlook::StopsAt(_)) {
        let line = card.detail.clone();
        card.with_key_label(line)
    } else {
        card
    };
    card.marked_stale(stale)
}

/// High/low, feels-like, wind and humidity. `observed` is the observation
/// the resting screen would use (none when it is missing or stale); without
/// it the screen falls back to the current hour, with the hour's condition
/// for the label and `--` for wind and humidity. `None` only when there is
/// neither, so the resting screen shows "No data".
fn weather_details(
    f: &Forecast,
    observed: Option<&Current>,
    hour_now: Option<&Hour>,
    units: Units,
    now: NaiveDateTime,
) -> Option<Card> {
    let (code, is_day, temperature) = match (observed, hour_now) {
        (Some(c), _) => (c.code, c.is_day, c.temperature),
        (None, Some(h)) => (h.code, h.is_day, h.temperature),
        (None, None) => return None,
    };
    // Today's range only when the forecast really has today; otherwise the
    // current temperature rather than another day's range.
    let value = f
        .daily
        .iter()
        .find(|d| d.date == now.date())
        .map_or_else(|| temp(temperature), |d| high_low(d.max, d.min));
    let (label, detail) = match observed {
        Some(c) => (
            format!("Feels {}", temp(c.apparent_temperature)),
            format!(
                "Wind {} {} · Hum {}%",
                whole(c.wind_speed),
                units.wind_label(),
                whole(c.humidity)
            ),
        ),
        None => (condition(code), "Wind -- · Hum --".to_string()),
    };
    Some(Card::new(weather_glyph(code, is_day), value, label, detail))
}

/// The day `base_day` days after today, stepped `view.days` further and
/// wrapped within the days the forecast still covers. `None` when it covers
/// none (every entry is in the past).
pub fn forecast(
    f: &Forecast,
    base_day: u8,
    view: ForecastView,
    now: NaiveDateTime,
    stale: bool,
) -> Option<Card> {
    let today = today_index(f, now)?;
    let available = (f.daily.len() - today) as i64;
    let ahead = (i64::from(base_day) + i64::from(view.days)).rem_euclid(available);
    let day = &f.daily[today + ahead as usize];

    // Named from the date itself, not the list position: the parser may have
    // dropped a day, so "one entry after today" isn't necessarily tomorrow.
    let name = match (day.date - now.date()).num_days() {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        _ => day.date.format("%a %-d").to_string(),
    };

    let card = if view.sun {
        let clock = |t: Option<NaiveDateTime>| {
            t.map_or("--".to_string(), |t| t.format("%H:%M").to_string())
        };
        Card::new(
            weather_glyph(day.code, true),
            high_low(day.max, day.min),
            condition(day.code),
            format!("↑{} ↓{}", clock(day.sunrise), clock(day.sunset)),
        )
    } else {
        // A key has one line under the value: the day, plus the chance of
        // rain when there is one (the dial has room for it on its own line).
        let key_label = match chance(day.precipitation_probability) {
            Some(p) => format!("{name} · {p}%"),
            None => name.clone(),
        };
        Card::new(
            weather_glyph(day.code, true),
            high_low(day.max, day.min),
            name,
            chance_or_condition(day.precipitation_probability, day.code),
        )
        .with_key_label(key_label)
    };
    Some(card.marked_stale(stale))
}

pub(crate) const AQI_GOOD: &str = "#22c55e";
const AQI_FAIR: &str = "#84cc16";
const AQI_MODERATE: &str = "#eab308";
const AQI_POOR: &str = "#f97316";
const AQI_VERY_POOR: &str = "#ef4444";
const AQI_HAZARDOUS: &str = "#a855f7";

/// Category name and color, using each scale's own official breakpoints.
fn aqi_category(scale: AqiScale, aqi: f64) -> (&'static str, &'static str) {
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
        Some(v) => whole(v).to_string(),
        None => "--".into(),
    }
}

pub fn air_quality(aq: &AirQuality, scale: AqiScale, page: AirQualityPage, stale: bool) -> Card {
    let (aqi, scale_name) = match scale {
        AqiScale::Us => (aq.us_aqi, "US AQI"),
        AqiScale::European => (aq.european_aqi, "European AQI"),
    };
    let (category, color) = aqi.map_or(("No data", palette::MUTED), |a| aqi_category(scale, a));
    let glyph = Glyph::Ring(color);

    let pollutant =
        |value: Option<f64>, name: &str| Card::new(glyph, concentration(value), name, "µg/m³");

    let card = match page {
        AirQualityPage::Index => Card::new(
            glyph,
            aqi.map_or("--".into(), |a| whole(a).to_string()),
            category,
            scale_name,
        )
        .with_accent(aqi.map(|_| color)),
        AirQualityPage::Pm25 => pollutant(aq.pm2_5, "PM2.5"),
        AirQualityPage::Pm10 => pollutant(aq.pm10, "PM10"),
        AirQualityPage::Ozone => pollutant(aq.ozone, "Ozone"),
        AirQualityPage::NitrogenDioxide => pollutant(aq.nitrogen_dioxide, "NO₂"),
    };
    card.marked_stale(stale)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::card::key_svg;
    use crate::model::{Current, Day, Hour};
    use chrono::NaiveDate;

    fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    pub(crate) fn sample() -> Forecast {
        Forecast {
            utc_offset_seconds: 3600,
            current: Some(Current {
                temperature: 21.6,
                apparent_temperature: 20.4,
                humidity: 46.0,
                wind_speed: 12.3,
                code: 2,
                is_day: true,
            }),
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

    /// `sample()` with no rain in its hourly series: hour 16 is clear.
    fn dry_sample() -> Forecast {
        let mut f = sample();
        set_hour(&mut f, 16, 0, Some(0.0));
        f
    }

    /// Sets hour `h` (counted from 00:00 on the 25th) of `f`'s hourly series.
    fn set_hour(f: &mut Forecast, h: usize, code: u8, probability: Option<f64>) {
        f.hourly[h].code = code;
        f.hourly[h].precipitation_probability = probability;
    }

    pub(crate) fn now() -> NaiveDateTime {
        at(25, 14, 40)
    }

    /// `sample()` with its daily series replaced by `count` days starting on
    /// September `first`.
    pub(crate) fn sample_days(first: u32, count: u32) -> Forecast {
        let mut f = sample();
        f.daily = (0..count)
            .map(|d| Day {
                date: NaiveDate::from_ymd_opt(2026, 9, first + d).unwrap(),
                code: 0,
                max: 20.0 + f64::from(d),
                min: 10.0,
                precipitation_probability: Some(10.0 * f64::from(d)),
                sunrise: None,
                sunset: None,
            })
            .collect();
        f
    }

    fn lisbon(f: &Forecast, view: WeatherView, now: NaiveDateTime) -> Card {
        weather(f, "Lisbon", Units::Metric, view, now, false)
    }

    fn day(f: &Forecast, base: u8, days: i32) -> Card {
        forecast(f, base, ForecastView { days, sun: false }, now(), false).unwrap()
    }

    #[test]
    fn weather_at_rest_shows_current_temperature_and_condition() {
        let c = lisbon(&dry_sample(), WeatherView::Now, now());
        assert_eq!(c.value, "22°");
        assert_eq!(c.label, "Partly cloudy");
        assert_eq!(c.detail, "Dry next 12h");
        let key = key_svg(&c);
        assert!(
            key.contains(">22°<") && key.contains(">Partly cloudy<"),
            "{key}"
        );
    }

    #[test]
    fn rain_starting_soon_shows_on_the_dial_and_the_key() {
        // Rain (code 61) at 16:00; it is 14:40 and partly cloudy.
        let c = lisbon(&sample(), WeatherView::Now, now());
        assert_eq!(c.label, "Partly cloudy");
        assert_eq!(c.detail, "Rain at 16:00");
        let key = key_svg(&c);
        assert!(key.contains(">Rain at 16:00<"), "{key}");
        assert!(!key.contains(">Partly cloudy<"), "{key}");
    }

    #[test]
    fn rain_stopping_shows_until_when() {
        let mut f = sample();
        f.current.as_mut().unwrap().code = 63;
        for h in 14..=17 {
            set_hour(&mut f, h, 61, Some(80.0));
        }
        let c = lisbon(&f, WeatherView::Now, now());
        assert_eq!(c.label, "Rain");
        assert_eq!(c.detail, "Rain until 18:00");
        assert!(key_svg(&c).contains(">Rain until 18:00<"));
    }

    #[test]
    fn a_dry_day_keeps_the_condition_on_the_key() {
        let c = lisbon(&dry_sample(), WeatherView::Now, now());
        assert_eq!(c.key_line(), "Partly cloudy");
        assert_eq!(c.detail, "Dry next 12h");
    }

    #[test]
    fn an_unknown_outlook_keeps_the_location_and_condition() {
        let mut f = dry_sample();
        set_hour(&mut f, 15, 0, None);
        let c = lisbon(&f, WeatherView::Now, now());
        assert_eq!(c.detail, "Lisbon");
        assert_eq!(c.key_line(), "Partly cloudy");
    }

    #[test]
    fn a_stale_card_still_gets_the_rain_line_from_the_forecast() {
        let c = weather(
            &sample(),
            "Lisbon",
            Units::Metric,
            WeatherView::Now,
            now(),
            true,
        );
        assert!(c.stale);
        assert_eq!(c.detail, "Rain at 16:00");
    }

    #[test]
    fn next_rain_finds_the_first_wet_hour_when_dry_now() {
        assert_eq!(
            next_rain(&sample(), now(), Some(2)),
            RainOutlook::StartsAt(at(25, 16, 0))
        );
    }

    #[test]
    fn next_rain_finds_the_first_dry_hour_when_raining_now() {
        let mut f = sample();
        for h in 14..=17 {
            set_hour(&mut f, h, 61, Some(90.0));
        }
        assert_eq!(
            next_rain(&f, now(), None),
            RainOutlook::StopsAt(at(25, 18, 0))
        );
    }

    #[test]
    fn next_rain_is_dry_when_every_hour_in_the_window_is_known_dry() {
        assert_eq!(next_rain(&dry_sample(), now(), Some(2)), RainOutlook::Dry);
        assert_eq!(rain_line(RainOutlook::Dry).unwrap(), "Dry next 12h");
    }

    #[test]
    fn an_unknown_chance_before_any_rain_is_unknown_not_dry() {
        let mut f = sample();
        set_hour(&mut f, 15, 2, None);
        assert_eq!(next_rain(&f, now(), Some(2)), RainOutlook::Unknown);
        assert_eq!(rain_line(RainOutlook::Unknown), None);
    }

    #[test]
    fn rain_past_the_lookahead_does_not_count() {
        // The window is the 12 hours after the current one (14:00): up to
        // 02:00 on the 26th, which is hour 26 of the series.
        let mut f = dry_sample();
        set_hour(&mut f, 27, 61, Some(90.0));
        assert_eq!(next_rain(&f, now(), Some(2)), RainOutlook::Dry);
        set_hour(&mut f, 26, 61, Some(90.0));
        assert_eq!(
            next_rain(&f, now(), Some(2)),
            RainOutlook::StartsAt(at(26, 2, 0))
        );
    }

    #[test]
    fn a_gap_in_the_hourly_series_is_unknown() {
        let mut f = dry_sample();
        f.hourly.remove(17);
        assert_eq!(next_rain(&f, now(), Some(2)), RainOutlook::Unknown);
    }

    #[test]
    fn the_rain_threshold_is_fifty_percent() {
        let mut f = dry_sample();
        set_hour(&mut f, 16, 0, Some(49.0));
        assert_eq!(next_rain(&f, now(), Some(2)), RainOutlook::Dry);
        set_hour(&mut f, 16, 0, Some(50.0));
        assert_eq!(
            next_rain(&f, now(), Some(2)),
            RainOutlook::StartsAt(at(25, 16, 0))
        );
    }

    #[test]
    fn the_observation_decides_whether_it_is_raining_now() {
        // Observed rain beats a dry current hour: rain until the next dry hour.
        assert_eq!(
            next_rain(&dry_sample(), now(), Some(61)),
            RainOutlook::StopsAt(at(25, 15, 0))
        );
        // Observed clear beats a wet current hour: look for the next wet one.
        let mut f = sample();
        set_hour(&mut f, 14, 61, Some(90.0));
        assert_eq!(
            next_rain(&f, now(), Some(0)),
            RainOutlook::StartsAt(at(25, 16, 0))
        );
    }

    #[test]
    fn without_an_observation_an_unknown_current_hour_is_unknown() {
        let mut f = dry_sample();
        set_hour(&mut f, 14, 0, None);
        assert_eq!(next_rain(&f, now(), None), RainOutlook::Unknown);
    }

    #[test]
    fn rain_through_the_whole_window_is_unknown() {
        let mut f = sample();
        for h in 14..=26 {
            set_hour(&mut f, h, 61, Some(90.0));
        }
        assert_eq!(next_rain(&f, now(), None), RainOutlook::Unknown);
    }

    #[test]
    fn rain_lines_use_the_hour() {
        assert_eq!(
            rain_line(RainOutlook::StartsAt(at(25, 16, 0))).unwrap(),
            "Rain at 16:00"
        );
        assert_eq!(
            rain_line(RainOutlook::StopsAt(at(25, 18, 0))).unwrap(),
            "Rain until 18:00"
        );
    }

    #[test]
    fn weather_detail_shows_todays_range_and_feels_like() {
        let c = weather(
            &sample(),
            "Lisbon",
            Units::Imperial,
            WeatherView::Details,
            now(),
            false,
        );
        assert_eq!(c.value, "28°/18°");
        assert_eq!(c.label, "Feels 20°");
        assert_eq!(c.detail, "Wind 12 mph · Hum 46%");
        let key = key_svg(&c);
        assert!(
            key.contains(">28°/18°<") && key.contains(">Feels 20°<"),
            "{key}"
        );
    }

    #[test]
    fn weather_detail_without_todays_entry_shows_current_values_and_wind() {
        // Only tomorrow onwards: tomorrow's range must not pass for today's.
        let f = sample_days(26, 3);
        let c = lisbon(&f, WeatherView::Details, now());
        assert_eq!(c.value, "22°");
        assert_eq!(c.detail, "Wind 12 km/h · Hum 46%");
    }

    #[test]
    fn weather_details_without_current_conditions_still_differ_from_now() {
        let mut f = sample();
        f.current = None;
        let details = lisbon(&f, WeatherView::Details, now());
        assert_eq!(details.value, "28°/18°");
        assert_eq!(details.label, "Clear", "the current hour's condition");
        assert_eq!(details.detail, "Wind -- · Hum --");
        assert_ne!(details, lisbon(&f, WeatherView::Now, now()));
    }

    #[test]
    fn stale_weather_details_ignore_the_old_observation() {
        let c = weather(
            &sample(),
            "Lisbon",
            Units::Metric,
            WeatherView::Details,
            now(),
            true,
        );
        assert_eq!(c.value, "28°/18°");
        assert_eq!(c.label, "Clear", "not the old observation's feels-like");
        assert_eq!(c.detail, "Wind -- · Hum --");
        assert!(c.stale);
    }

    #[test]
    fn weather_details_without_today_or_current_use_the_current_hour() {
        let mut f = sample_days(26, 3);
        f.current = None;
        let c = lisbon(&f, WeatherView::Details, now());
        assert_eq!(c.value, "14°");
        assert_eq!(c.label, "Clear");
    }

    #[test]
    fn a_weather_day_view_shows_that_forecast_day() {
        let f = sample();
        let c = lisbon(&f, WeatherView::Day(1), now());
        let expected = day(&f, 0, 1);
        assert_eq!(c, expected, "what a Forecast dial shows for tomorrow");
        assert_eq!(c.value, "29°/18°");
        let stale = weather(
            &f,
            "Lisbon",
            Units::Metric,
            WeatherView::Day(1),
            now(),
            true,
        );
        assert!(stale.stale);
    }

    #[test]
    fn weather_offset_scrolls_hourly_from_the_current_hour() {
        let c = lisbon(&sample(), WeatherView::Hour(2), now());
        assert_eq!(c.label, "16:00");
        assert_eq!(c.value, "16°");
        assert_eq!(c.detail, "Rain 40%");
    }

    #[test]
    fn on_the_hour_the_current_hour_is_the_one_starting_now() {
        let c = lisbon(&sample(), WeatherView::Hour(1), at(25, 15, 0));
        assert_eq!(c.label, "16:00");
    }

    #[test]
    fn weather_offset_past_the_forecast_falls_back_to_now() {
        let c = lisbon(&sample(), WeatherView::Hour(24), at(26, 23, 30));
        assert_eq!(c.value, "22°");
    }

    #[test]
    fn weather_without_current_conditions_uses_the_current_hour() {
        let mut f = sample();
        f.current = None;
        let c = lisbon(&f, WeatherView::Now, now());
        assert_eq!(c.value, "14°");
        assert_eq!(c.label, "Clear");
    }

    #[test]
    fn stale_weather_prefers_the_current_hours_forecast_and_is_marked() {
        let c = weather(
            &sample(),
            "Lisbon",
            Units::Metric,
            WeatherView::Now,
            now(),
            true,
        );
        assert_eq!(
            c.value, "14°",
            "the forecast for 14:00, not the old observation"
        );
        assert!(c.stale);
        let fresh = lisbon(&sample(), WeatherView::Now, now());
        assert!(!fresh.stale);
    }

    #[test]
    fn stale_forecast_and_air_quality_cards_are_marked() {
        let f = sample();
        assert!(
            forecast(&f, 1, ForecastView::default(), now(), true)
                .unwrap()
                .stale
        );
        assert!(air_quality(&aq(), AqiScale::Us, AirQualityPage::Index, true).stale);
        assert!(!air_quality(&aq(), AqiScale::Us, AirQualityPage::Index, false).stale);
    }

    #[test]
    fn temperatures_never_render_negative_zero() {
        assert_eq!(temp(-0.4), "0°");
        assert_eq!(temp(-0.6), "-1°");
    }

    #[test]
    fn forecast_names_today_and_tomorrow_and_dates_after() {
        let f = sample();
        assert_eq!(day(&f, 0, 0).label, "Today");
        assert_eq!(day(&f, 1, 0).label, "Tomorrow");
        assert_eq!(day(&f, 1, 0).value, "29°/18°");
        assert_eq!(day(&f, 1, 0).detail, "Rain 70%");
        assert_eq!(day(&f, 1, 1).label, "Sun 27");
    }

    #[test]
    fn a_forecast_key_shows_the_day_and_the_chance_of_rain() {
        let key = key_svg(&day(&sample(), 1, 0));
        assert!(key.contains(">Tomorrow · 70%<"), "{key}");
        assert!(key.contains(">29°/18°<"), "{key}");
        // No chance worth showing: just the day.
        let key = key_svg(&day(&sample(), 0, 0));
        assert!(key.contains(">Today<"), "{key}");
    }

    #[test]
    fn forecast_labels_come_from_the_date_not_the_list_position() {
        // The parser dropped today's entry: the first day is tomorrow.
        let f = sample_days(26, 5);
        let c = day(&f, 0, 0);
        assert_eq!(c.label, "Tomorrow");
        assert_eq!(c.value, "20°/10°");
        assert_eq!(day(&f, 1, 0).label, "Sun 27");
    }

    #[test]
    fn forecast_wraps_around_the_available_days() {
        let f = sample();
        assert_eq!(day(&f, 1, 2).label, "Today");
        // A configured day beyond the horizon wraps too, rather than panicking:
        // 6 mod 3 available days is today.
        assert_eq!(day(&f, 6, 0).label, "Today");
        assert_eq!(day(&f, 7, 0).label, "Tomorrow");
        assert_eq!(day(&f, 255, i32::MAX).label, day(&f, 0, 1).label.as_str());
    }

    #[test]
    fn forecast_detail_shows_sunrise_and_sunset() {
        let view = ForecastView { days: 0, sun: true };
        let c = forecast(&sample(), 0, view, now(), false).unwrap();
        assert_eq!(c.label, "Partly cloudy");
        assert_eq!(c.detail, "↑07:26 ↓19:28");
    }

    #[test]
    fn forecast_with_only_past_days_has_nothing_to_show() {
        let c = forecast(&sample(), 0, ForecastView::default(), at(30, 0, 0), false);
        assert!(c.is_none());
    }

    #[test]
    fn us_aqi_breakpoints() {
        for (scale, at, name) in [
            (AqiScale::Us, 50.0, "Good"),
            (AqiScale::Us, 51.0, "Moderate"),
            (AqiScale::Us, 100.0, "Moderate"),
            (AqiScale::Us, 101.0, "Unhealthy (SG)"),
            (AqiScale::Us, 150.0, "Unhealthy (SG)"),
            (AqiScale::Us, 151.0, "Unhealthy"),
            (AqiScale::Us, 200.0, "Unhealthy"),
            (AqiScale::Us, 201.0, "Very unhealthy"),
            (AqiScale::Us, 300.0, "Very unhealthy"),
            (AqiScale::Us, 301.0, "Hazardous"),
            (AqiScale::European, 20.0, "Good"),
            (AqiScale::European, 21.0, "Fair"),
            (AqiScale::European, 40.0, "Fair"),
            (AqiScale::European, 41.0, "Moderate"),
            (AqiScale::European, 60.0, "Moderate"),
            (AqiScale::European, 61.0, "Poor"),
            (AqiScale::European, 80.0, "Poor"),
            (AqiScale::European, 81.0, "Very poor"),
            (AqiScale::European, 100.0, "Very poor"),
            (AqiScale::European, 101.0, "Extremely poor"),
        ] {
            assert_eq!(aqi_category(scale, at).0, name, "{scale:?} {at}");
        }
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

    pub(crate) fn aq() -> AirQuality {
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
        let c = air_quality(&aq(), AqiScale::Us, AirQualityPage::Index, false);
        assert_eq!(c.value, "43");
        assert_eq!(c.label, "Good");
        assert_eq!(c.accent, Some(AQI_GOOD));
        assert_eq!(c.glyph, Glyph::Ring(AQI_GOOD));
        let key = key_svg(&c);
        assert!(key.contains(">43<") && key.contains(">Good<"), "{key}");
        let eu = air_quality(&aq(), AqiScale::European, AirQualityPage::Index, false);
        assert_eq!((eu.value.as_str(), eu.label.as_str()), ("24", "Fair"));
    }

    #[test]
    fn air_quality_pages_through_pollutants() {
        let page = |p| air_quality(&aq(), AqiScale::Us, p, false);
        let pm25 = page(AirQualityPage::Pm25);
        assert_eq!((pm25.label.as_str(), pm25.value.as_str()), ("PM2.5", "6.5"));
        assert_eq!(page(AirQualityPage::Pm10).value, "11");
        assert_eq!(page(AirQualityPage::Ozone).label, "Ozone");
        assert_eq!(page(AirQualityPage::NitrogenDioxide).value, "--");
        assert_eq!(page(AirQualityPage::NitrogenDioxide).label, "NO₂");
        let key = key_svg(&pm25);
        assert!(key.contains(">6.5<") && key.contains(">PM2.5<"), "{key}");
    }

    #[test]
    fn missing_aqi_is_neutral_not_good() {
        let c = air_quality(
            &AirQuality::default(),
            AqiScale::Us,
            AirQualityPage::Index,
            false,
        );
        assert_eq!(c.value, "--");
        assert_eq!(c.label, "No data");
        assert_eq!(c.accent, None);
        assert_eq!(c.glyph, Glyph::Ring(palette::MUTED));
    }

    #[test]
    fn placeholder_keys_say_what_is_missing() {
        assert!(key_svg(&no_location()).contains(">Set location<"));
        assert!(key_svg(&no_data("Lisbon")).contains(">No data<"));
    }
}
