//! Plumbing shared by the three actions: rendering a `Card` to whichever
//! surface an instance lives on, the background refresh/revert loop, and
//! the property inspector's location search.

pub mod air_quality;
pub mod forecast;
pub mod weather;

use crate::card::{self, Card};
use crate::model::Settings;
use crate::services::Services;
use crate::tracker::Tracker;
use crate::views::View;
use async_trait::async_trait;
use openaction::{Instance, OpenActionResult};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::Instant;

/// The wire value OpenDeck sends as `Instance::controller` for a key (vs.
/// `"Encoder"` for a dial).
const KEYPAD_CONTROLLER: &str = "Keypad";

/// How long a scrolled/toggled screen stays up after the last interaction.
const VIEW_TIMEOUT: Duration = Duration::from_secs(15);
/// How often every instance re-renders. Mostly cache hits (the cache TTL is
/// longer); it keeps the "current hour" rolling and picks up fresh data
/// soon after the cache expires.
const REFRESH_EVERY: Duration = Duration::from_secs(60);

#[async_trait]
pub trait CardAction: Clone + Send + Sync + 'static {
    fn tracker(&self) -> &Tracker;
    /// Builds the card for one instance. Fetches through the shared cache.
    async fn card(&self, settings: &Settings, view: View) -> Card;
}

async fn show(instance: &Instance, card: &Card) -> OpenActionResult<()> {
    if instance.controller == KEYPAD_CONTROLLER {
        // The text is drawn inside the image; clear the native title so
        // OpenDeck doesn't paint a second copy on top.
        instance.set_title(Some(String::new()), None).await?;
        instance.set_image(Some(card::key_image(card)), None).await
    } else {
        instance.set_feedback(&card::feedback(card)).await
    }
}

pub async fn render(action: &impl CardAction, instance: &Instance) -> OpenActionResult<()> {
    let Some((settings, view)) = action.tracker().get(&instance.instance_id) else {
        return Ok(()); // disappeared meanwhile
    };
    let card = action.card(&settings, view).await;
    show(instance, &card).await
}

async fn render_id(action: &impl CardAction, id: String) {
    let Some(instance) = openaction::get_instance(id).await else {
        return;
    };
    if let Err(e) = render(action, &instance).await {
        log::warn!("render failed: {e}");
    }
}

/// Renders in the background: the first render of a new location has to
/// wait on the network, and that shouldn't hold up OpenDeck's event stream.
pub fn render_soon(action: &impl CardAction, instance: &Instance) {
    let action = action.clone();
    let id = instance.instance_id.clone();
    tokio::spawn(async move { render_id(&action, id).await });
}

/// Apply an interaction to one instance's view, then re-render it.
pub async fn interact(
    action: &impl CardAction,
    instance: &Instance,
    f: impl FnOnce(&mut View),
) -> OpenActionResult<()> {
    if action.tracker().interact(&instance.instance_id, f) {
        render(action, instance).await?;
    }
    Ok(())
}

/// Runs forever: each second, re-renders instances whose scrolled/toggled
/// view just timed out; every `REFRESH_EVERY`, re-renders all of them.
pub async fn tick_loop(action: impl CardAction) {
    let mut last_refresh = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let due = if last_refresh.elapsed() >= REFRESH_EVERY {
            last_refresh = Instant::now();
            action.tracker().expire(VIEW_TIMEOUT);
            action.tracker().ids()
        } else {
            action.tracker().expire(VIEW_TIMEOUT)
        };
        for id in due {
            render_id(&action, id).await;
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
enum InspectorMessage {
    Search { query: String },
}

/// Handles `sendToPlugin` from the property inspector. Its only request is
/// a location search, answered with `searchResults` - the plugin does the
/// geocoding so the inspector needs no network access of its own.
pub async fn handle_inspector(
    services: &Services,
    instance: &Instance,
    payload: &Value,
) -> OpenActionResult<()> {
    let Ok(InspectorMessage::Search { query }) = serde_json::from_value(payload.clone()) else {
        log::warn!("ignoring unknown property inspector message: {payload}");
        return Ok(());
    };
    let reply = match services.search(&query).await {
        Ok(results) => json!({ "event": "searchResults", "query": query, "results": results }),
        Err(e) => {
            log::warn!("location search failed: {e}");
            json!({ "event": "searchResults", "query": query, "error": e.to_string() })
        }
    };
    instance.send_to_property_inspector(reply).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_inspectors_search_request() {
        let msg: InspectorMessage =
            serde_json::from_value(json!({"event": "search", "query": "Porto"})).unwrap();
        let InspectorMessage::Search { query } = msg;
        assert_eq!(query, "Porto");
    }

    #[test]
    fn every_action_uuid_is_in_the_shipped_manifest() {
        use openaction::Action;
        let manifest: Value =
            serde_json::from_str(include_str!("../../assets/manifest.json")).unwrap();
        let uuids: Vec<&str> = manifest["Actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["UUID"].as_str().unwrap())
            .collect();
        for uuid in [
            <weather::WeatherAction as Action>::UUID,
            <forecast::ForecastAction as Action>::UUID,
            <air_quality::AirQualityAction as Action>::UUID,
        ] {
            assert!(uuids.contains(&uuid), "{uuid} missing from manifest");
        }
    }
}
