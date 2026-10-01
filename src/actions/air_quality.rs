//! **Air Quality**: the current air quality index (US or European scale),
//! colored by category, plus PM2.5, PM10, ozone and NO₂ pages.
//!
//! - Key: shows the index. Press to page through the pollutants; it
//!   returns to the index on its own.
//! - Dial: rotate to page, press or tap the strip to jump back to the index.

use super::{CardAction, handle_inspector, interact, render_soon};
use crate::card::Card;
use crate::model::Settings;
use crate::services::Services;
use crate::tracker::Tracker;
use crate::views::{self, AIR_QUALITY_PAGES, View};
use async_trait::async_trait;
use openaction::{Action, Instance, OpenActionResult};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
pub struct AirQualityAction {
    services: Arc<Services>,
    tracker: Arc<Tracker>,
}

impl AirQualityAction {
    pub fn new(services: Arc<Services>) -> Self {
        Self {
            services,
            tracker: Arc::default(),
        }
    }
}

fn page(v: &mut View, pages: i32) {
    v.offset = (v.offset + pages).rem_euclid(AIR_QUALITY_PAGES);
}

fn back_to_index(v: &mut View) {
    v.offset = 0;
}

#[async_trait]
impl CardAction for AirQualityAction {
    fn tracker(&self) -> &Tracker {
        &self.tracker
    }

    async fn card(&self, settings: &Settings, view: View) -> Card {
        let Some(location) = &settings.location else {
            return views::no_location();
        };
        match self.services.air_quality(location).await {
            Ok(aq) => views::air_quality(&aq, settings.aqi_scale, view),
            Err(e) => {
                log::warn!("air quality for {} failed: {e}", location.name);
                views::no_data(&location.name)
            }
        }
    }
}

#[async_trait]
impl Action for AirQualityAction {
    const UUID: &'static str = "com.jfms7s.weather.airquality";
    type Settings = Settings;

    async fn will_appear(&self, instance: &Instance, settings: &Settings) -> OpenActionResult<()> {
        self.tracker.track(&instance.instance_id, settings.clone());
        render_soon(self, instance);
        Ok(())
    }

    async fn did_receive_settings(
        &self,
        instance: &Instance,
        settings: &Settings,
    ) -> OpenActionResult<()> {
        self.will_appear(instance, settings).await
    }

    async fn will_disappear(&self, instance: &Instance, _: &Settings) -> OpenActionResult<()> {
        self.tracker.untrack(&instance.instance_id);
        Ok(())
    }

    async fn key_up(&self, instance: &Instance, _: &Settings) -> OpenActionResult<()> {
        interact(self, instance, |v| page(v, 1)).await
    }

    async fn dial_rotate(
        &self,
        instance: &Instance,
        _: &Settings,
        ticks: i16,
        _pressed: bool,
    ) -> OpenActionResult<()> {
        interact(self, instance, |v| page(v, i32::from(ticks))).await
    }

    async fn dial_up(&self, instance: &Instance, _: &Settings) -> OpenActionResult<()> {
        interact(self, instance, back_to_index).await
    }

    async fn touch_tap(
        &self,
        instance: &Instance,
        _: &Settings,
        _position: (u16, u16),
        _hold: bool,
    ) -> OpenActionResult<()> {
        interact(self, instance, back_to_index).await
    }

    async fn send_to_plugin(
        &self,
        instance: &Instance,
        _: &Settings,
        payload: &Value,
    ) -> OpenActionResult<()> {
        handle_inspector(&self.services, instance, payload).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paging_wraps_in_both_directions() {
        let mut v = View::default();
        page(&mut v, -1);
        assert_eq!(v.offset, AIR_QUALITY_PAGES - 1);
        page(&mut v, 1);
        assert_eq!(v.offset, 0);
    }
}
