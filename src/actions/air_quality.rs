//! **Air Quality**: the current air quality index (US or European scale),
//! colored by category, plus PM2.5, PM10, ozone and NO₂ pages.
//!
//! - Key: shows the index. Press to page through the pollutants; it
//!   returns to the index on its own.
//! - Dial: rotate to page, press or tap the strip to jump back to the index.

use super::Behavior;
use crate::card::Card;
use crate::model::{Location, Settings};
use crate::open_meteo::FetchError;
use crate::services::Services;
use crate::view_state::AirQualityPage;
use crate::views;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub struct AirQuality;

#[async_trait]
impl Behavior for AirQuality {
    const UUID: &'static str = "com.jfms7s.weather.airquality";
    type View = AirQualityPage;

    fn key_press(page: AirQualityPage) -> AirQualityPage {
        page.step(1)
    }

    fn dial_rotate(page: AirQualityPage, ticks: i16) -> AirQualityPage {
        page.step(i32::from(ticks))
    }

    fn dial_press(_: AirQualityPage) -> AirQualityPage {
        AirQualityPage::Index
    }

    async fn card(
        services: &Services,
        location: &Location,
        settings: &Settings,
        page: AirQualityPage,
        _utc_now: DateTime<Utc>,
    ) -> Result<Card, FetchError> {
        let aq = services.air_quality(location).await?;
        Ok(views::air_quality(
            &aq.value,
            settings.aqi_scale,
            page,
            aq.stale,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_page_forward_and_a_dial_press_returns_to_the_index() {
        let p = AirQuality::key_press(AirQualityPage::Index);
        assert_eq!(p, AirQualityPage::Pm25);
        assert_eq!(
            AirQuality::dial_rotate(p, -2),
            AirQualityPage::NitrogenDioxide
        );
        assert_eq!(AirQuality::dial_press(p), AirQualityPage::Index);
    }
}
