//! **Forecast**: one day of the 7-day forecast - high/low, condition and
//! chance of rain.
//!
//! - Key: shows the configured day (tomorrow by default) and its chance of
//!   rain. Press to step to the next day; it returns to the configured day
//!   on its own.
//! - Dial: rotate to scroll through the days, press or tap the strip for
//!   sunrise/sunset, hold the strip to refresh now.

use super::Behavior;
use crate::card::Card;
use crate::ipma;
use crate::model::{Location, Settings};
use crate::open_meteo::{FORECAST_DAYS, FetchError};
use crate::services::Services;
use crate::view_state::ForecastView;
use crate::views;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub struct Forecast;

#[async_trait]
impl Behavior for Forecast {
    const UUID: &'static str = "com.jfms7s.weather.forecast";
    type View = ForecastView;

    fn key_press(view: ForecastView) -> ForecastView {
        view.step(1)
    }

    fn dial_rotate(view: ForecastView, ticks: i16) -> ForecastView {
        view.step(i32::from(ticks))
    }

    fn dial_press(view: ForecastView) -> ForecastView {
        view.toggle_sun()
    }

    async fn refresh(
        services: &Services,
        location: &Location,
        settings: &Settings,
    ) -> Result<(), FetchError> {
        services
            .refresh_forecast(location, settings.units)
            .await
            .map(drop)
    }

    async fn card(
        services: &Services,
        location: &Location,
        settings: &Settings,
        view: ForecastView,
        utc_now: DateTime<Utc>,
        _warning: Option<ipma::Active>,
    ) -> Result<Card, FetchError> {
        let f = services.forecast(location, settings.units).await?;
        let now = f.value.local_now(utc_now);
        Ok(
            views::forecast(&f.value, resting_day(settings), view, now, f.stale)
                .unwrap_or_else(|| views::no_data(&location.name)),
        )
    }
}

/// The configured day, clamped to the forecast: a hand-edited `day: 9`
/// shows the last day rather than wrapping round to day 2.
fn resting_day(settings: &Settings) -> u8 {
    settings.day.min(FORECAST_DAYS - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_out_of_range_day_setting_is_clamped_to_the_last_day() {
        let day = |day| {
            resting_day(&Settings {
                day,
                ..Settings::default()
            })
        };
        assert_eq!(day(9), 6);
        assert_eq!(day(6), 6);
        assert_eq!(day(3), 3);
        assert_eq!(day(0), 0);
    }
    use crate::views::tests::sample_days;

    #[test]
    fn every_press_changes_the_card_when_fewer_than_seven_days_are_left() {
        // A cached 7-day forecast served the day after it was fetched: only
        // 6 days are still ahead of "today".
        let f = sample_days(24, 7);
        let now = chrono::NaiveDate::from_ymd_opt(2026, 9, 25)
            .unwrap()
            .and_hms_opt(14, 40, 0)
            .unwrap();
        let label = |v| views::forecast(&f, 1, v, now, false).unwrap().label;
        let mut v = ForecastView::default();
        let mut labels = vec![label(v)];
        for _ in 0..8 {
            v = Forecast::key_press(v);
            labels.push(label(v));
        }
        for pair in labels.windows(2) {
            assert_ne!(pair[0], pair[1], "a press did nothing: {labels:?}");
        }
        // Rotating back from the resting day changes it too.
        let back = Forecast::dial_rotate(ForecastView::default(), -1);
        assert_ne!(label(back), labels[0]);
    }

    #[test]
    fn a_dial_press_toggles_sunrise_and_sunset() {
        assert!(Forecast::dial_press(ForecastView::default()).sun);
    }
}
