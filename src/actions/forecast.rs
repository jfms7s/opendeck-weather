//! **Forecast**: one day of the 7-day forecast - high/low, condition and
//! chance of rain.
//!
//! - Key: shows the configured day (tomorrow by default). Press to step to
//!   the next day; it returns to the configured day on its own.
//! - Dial: rotate to scroll through the days, press or tap the strip for
//!   sunrise/sunset.

use super::{CardAction, handle_inspector, interact, render_soon};
use crate::card::Card;
use crate::model::Settings;
use crate::services::{Services, local_now};
use crate::tracker::Tracker;
use crate::views::{self, View};
use async_trait::async_trait;
use openaction::{Action, Instance, OpenActionResult};
use serde_json::Value;
use std::sync::Arc;

/// Open-Meteo's `forecast_days`; `views::forecast` wraps within however
/// many of these are still ahead of "today".
const DAYS: i32 = 7;

#[derive(Clone)]
pub struct ForecastAction {
    services: Arc<Services>,
    tracker: Arc<Tracker>,
}

impl ForecastAction {
    pub fn new(services: Arc<Services>) -> Self {
        Self {
            services,
            tracker: Arc::default(),
        }
    }
}

fn step_days(v: &mut View, days: i32) {
    v.offset = (v.offset + days).rem_euclid(DAYS);
    v.detail = false;
}

fn toggle_detail(v: &mut View) {
    v.detail = !v.detail;
}

#[async_trait]
impl CardAction for ForecastAction {
    fn tracker(&self) -> &Tracker {
        &self.tracker
    }

    async fn card(&self, settings: &Settings, view: View) -> Card {
        let Some(location) = &settings.location else {
            return views::no_location();
        };
        match self.services.forecast(location, settings.units).await {
            Ok(f) => views::forecast(&f, settings.day, view, local_now(&f))
                .unwrap_or_else(|| views::no_data(&location.name)),
            Err(e) => {
                log::warn!("forecast for {} failed: {e}", location.name);
                views::no_data(&location.name)
            }
        }
    }
}

#[async_trait]
impl Action for ForecastAction {
    const UUID: &'static str = "com.jfms7s.weather.forecast";
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
        interact(self, instance, |v| step_days(v, 1)).await
    }

    async fn dial_rotate(
        &self,
        instance: &Instance,
        _: &Settings,
        ticks: i16,
        _pressed: bool,
    ) -> OpenActionResult<()> {
        interact(self, instance, |v| step_days(v, i32::from(ticks))).await
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
    fn stepping_wraps_in_both_directions_and_clears_details() {
        let mut v = View {
            offset: 6,
            detail: true,
        };
        step_days(&mut v, 1);
        assert_eq!(v, View::default());
        step_days(&mut v, -1);
        assert_eq!(v.offset, 6);
    }
}
