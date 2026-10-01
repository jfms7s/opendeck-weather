//! **Weather**: current conditions at the configured location.
//!
//! - Key: temperature + condition. Press shows today's high/low and
//!   "feels like"; it reverts on its own after a few seconds.
//! - Dial: rotate to scroll the hourly forecast (up to 24h ahead), press or
//!   tap the strip for the details screen.

use super::{CardAction, handle_inspector, interact, render_soon};
use crate::card::Card;
use crate::model::Settings;
use crate::services::{Services, local_now};
use crate::tracker::Tracker;
use crate::views::{self, MAX_HOUR_OFFSET, View};
use async_trait::async_trait;
use openaction::{Action, Instance, OpenActionResult};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
pub struct WeatherAction {
    services: Arc<Services>,
    tracker: Arc<Tracker>,
}

impl WeatherAction {
    pub fn new(services: Arc<Services>) -> Self {
        Self {
            services,
            tracker: Arc::default(),
        }
    }
}

fn toggle_detail(v: &mut View) {
    v.detail = !v.detail;
    v.offset = 0;
}

fn scroll_hours(v: &mut View, ticks: i16) {
    v.offset = (v.offset + i32::from(ticks)).clamp(0, MAX_HOUR_OFFSET);
    v.detail = false;
}

#[async_trait]
impl CardAction for WeatherAction {
    fn tracker(&self) -> &Tracker {
        &self.tracker
    }

    async fn card(&self, settings: &Settings, view: View) -> Card {
        let Some(location) = &settings.location else {
            return views::no_location();
        };
        match self.services.forecast(location, settings.units).await {
            Ok(f) => views::weather(&f, &location.name, settings.units, view, local_now(&f)),
            Err(e) => {
                log::warn!("forecast for {} failed: {e}", location.name);
                views::no_data(&location.name)
            }
        }
    }
}

#[async_trait]
impl Action for WeatherAction {
    const UUID: &'static str = "com.jfms7s.weather.current";
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
        interact(self, instance, toggle_detail).await
    }

    async fn dial_rotate(
        &self,
        instance: &Instance,
        _: &Settings,
        ticks: i16,
        _pressed: bool,
    ) -> OpenActionResult<()> {
        interact(self, instance, |v| scroll_hours(v, ticks)).await
    }

    async fn dial_up(&self, instance: &Instance, _: &Settings) -> OpenActionResult<()> {
        interact(self, instance, toggle_detail).await
    }

    async fn touch_tap(
        &self,
        instance: &Instance,
        _: &Settings,
        _position: (u16, u16),
        _hold: bool,
    ) -> OpenActionResult<()> {
        interact(self, instance, toggle_detail).await
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
    fn scrolling_clamps_between_now_and_the_horizon() {
        let mut v = View::default();
        scroll_hours(&mut v, -3);
        assert_eq!(v.offset, 0);
        scroll_hours(&mut v, 100);
        assert_eq!(v.offset, MAX_HOUR_OFFSET);
    }

    #[test]
    fn toggling_details_returns_to_now() {
        let mut v = View {
            offset: 5,
            detail: false,
        };
        toggle_detail(&mut v);
        assert_eq!(
            v,
            View {
                offset: 0,
                detail: true
            }
        );
        toggle_detail(&mut v);
        assert_eq!(v, View::default());
    }
}
