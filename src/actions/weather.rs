//! **Weather**: current conditions at the configured location.
//!
//! - Key: temperature + condition. Press shows today's high/low and
//!   "feels like"; it reverts on its own 15 s after the last press.
//! - Dial: rotate to scroll the hourly forecast (up to 24h ahead), press or
//!   tap the strip for the details screen.

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
        view.toggle_details()
    }

    fn dial_rotate(view: WeatherView, ticks: i16) -> WeatherView {
        view.scroll(ticks)
    }

    fn dial_press(view: WeatherView) -> WeatherView {
        view.toggle_details()
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
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_and_dial_presses_toggle_details_and_rotation_scrolls() {
        let v = Weather::dial_rotate(WeatherView::Now, 3);
        assert_eq!(v, WeatherView::Hour(3));
        assert_eq!(Weather::key_press(v), WeatherView::Details);
        assert_eq!(Weather::dial_press(WeatherView::Details), WeatherView::Now);
    }
}
