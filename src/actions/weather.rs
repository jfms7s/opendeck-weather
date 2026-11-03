//! **Weather**: current conditions at the configured location.
//!
//! - Key: temperature + condition, or `Rain at` / `Rain until HH:MM` instead
//!   of the condition when rain starts or stops within 12 hours. A press
//!   shows today's high/low and "feels like", a second one wind direction
//!   and gusts with the UV index, a third returns; it reverts on its own
//!   15 s after the last press.
//! - Dial: temperature, condition and the rain line (`Dry next 12h` too) in
//!   place of the location name. Rotate to scroll the hourly forecast (up to
//!   24h ahead), press and rotate to scroll the days (up to 6 ahead), press
//!   or tap the strip for the two details pages (the second adds pressure
//!   and the next sunrise or sunset), hold the strip to refresh now.

use super::Behavior;
use crate::card::Card;
use crate::model::{Location, Settings};
use crate::open_meteo::FetchError;
use crate::services::Services;
use crate::view_state::WeatherView;
use crate::views;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub struct Weather;

#[async_trait]
impl Behavior for Weather {
    const UUID: &'static str = "com.jfms7s.weather.current";
    type View = WeatherView;

    fn key_press(view: WeatherView) -> WeatherView {
        view.next_details()
    }

    fn dial_rotate(view: WeatherView, ticks: i16) -> WeatherView {
        view.scroll(ticks)
    }

    fn dial_rotate_pressed(view: WeatherView, ticks: i16) -> WeatherView {
        view.scroll_days(ticks)
    }

    fn dial_press(view: WeatherView) -> WeatherView {
        view.next_details()
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
        view: WeatherView,
        utc_now: DateTime<Utc>,
    ) -> Result<Card, FetchError> {
        let f = services.forecast(location, settings.units).await?;
        let now = f.value.local_now(utc_now);
        Ok(views::weather(
            &f.value,
            &location.name,
            settings.units,
            view,
            now,
            f.stale,
            services.warning(location, utc_now).as_ref(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_and_dial_presses_cycle_details_and_rotation_scrolls() {
        let v = Weather::dial_rotate(WeatherView::Now, 3);
        assert_eq!(v, WeatherView::Hour(3));
        assert_eq!(Weather::key_press(v), WeatherView::Details);
        assert_eq!(
            Weather::key_press(WeatherView::Details),
            WeatherView::MoreDetails
        );
        assert_eq!(
            Weather::dial_press(WeatherView::Details),
            WeatherView::MoreDetails
        );
        assert_eq!(
            Weather::dial_press(WeatherView::MoreDetails),
            WeatherView::Now
        );
    }

    #[test]
    fn a_pressed_turn_scrolls_days_and_a_plain_one_hours() {
        assert_eq!(
            Weather::dial_rotate_pressed(WeatherView::Now, 1),
            WeatherView::Day(1)
        );
        assert_eq!(
            Weather::dial_rotate(WeatherView::Now, 1),
            WeatherView::Hour(1)
        );
    }
}
