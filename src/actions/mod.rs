//! The OpenDeck side of every action, written once: `CardAction<B>` is the
//! `openaction::Action` for any `Behavior` - the per-action part, which is
//! just a UUID, a view type, how input changes the view, and how to build
//! the card. Also here: drawing a `Card` on whichever surface an instance
//! lives on, and the property inspector's requests.
//!
//! openaction runs event handlers one at a time on its websocket loop, so
//! no handler here awaits the network: renders and searches are spawned,
//! and a handler only updates in-memory state.
//!
//! Adding an action: see the checklist in the design note (vault,
//! `personal/projects/opendeck-weather/design`).

pub mod air_quality;
pub mod forecast;
pub mod weather;

use crate::card::{self, Card};
use crate::model::{GlobalSettings, Location, Place, Settings};
use crate::open_meteo::{FetchError, MAX_QUERY_CHARS};
use crate::scheduler::{Scheduled, VIEW_TIMEOUT};
use crate::services::Services;
use crate::tracker::{Frame, Tracker};
use crate::views;
use crate::wake::Wake;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use openaction::global_events::{DidReceiveGlobalSettingsEvent, GlobalEventHandler};
use openaction::{Action, Instance, OpenActionResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::time::Instant;

/// The wire value OpenDeck sends as `Instance::controller` for a key (vs.
/// `"Encoder"` for a dial).
const KEYPAD_CONTROLLER: &str = "Keypad";

/// What makes one action different from another.
#[async_trait]
pub trait Behavior: Send + Sync + 'static {
    /// Action UUID, as in `assets/manifest.json`.
    const UUID: &'static str;
    /// Its scrolled/toggled UI state (see `view_state`).
    type View: Copy + Default + PartialEq + Send + Sync + 'static;

    fn key_press(view: Self::View) -> Self::View;
    fn dial_rotate(view: Self::View, ticks: i16) -> Self::View;
    /// A dial press or a tap on its touch strip.
    fn dial_press(view: Self::View) -> Self::View;

    /// Fetches (through the shared cache) and builds the card for one
    /// instance. `utc_now` is the only clock read.
    async fn card(
        services: &Services,
        location: &Location,
        settings: &Settings,
        view: Self::View,
        utc_now: DateTime<Utc>,
    ) -> Result<Card, FetchError>;
}

/// Constructs and registers every action with openaction. The one list of
/// actions; the scheduler gets the same list back.
pub async fn register_all(services: &Arc<Services>, wake: &Arc<Wake>) -> Vec<Arc<dyn Scheduled>> {
    vec![
        register::<weather::Weather>(services, wake).await,
        register::<forecast::Forecast>(services, wake).await,
        register::<air_quality::AirQuality>(services, wake).await,
    ]
}

async fn register<B: Behavior>(services: &Arc<Services>, wake: &Arc<Wake>) -> Arc<dyn Scheduled> {
    let action = CardAction::<B>::new(services.clone(), wake.clone());
    openaction::register_action(action.clone()).await;
    Arc::new(action)
}

pub struct CardAction<B: Behavior> {
    services: Arc<Services>,
    tracker: Arc<Tracker<B::View>>,
    wake: Arc<Wake>,
    behavior: PhantomData<fn() -> B>,
}

impl<B: Behavior> Clone for CardAction<B> {
    fn clone(&self) -> Self {
        CardAction {
            services: self.services.clone(),
            tracker: self.tracker.clone(),
            wake: self.wake.clone(),
            behavior: PhantomData,
        }
    }
}

impl<B: Behavior> CardAction<B> {
    pub fn new(services: Arc<Services>, wake: Arc<Wake>) -> Self {
        CardAction {
            services,
            tracker: Arc::new(Tracker::new(wake.clone())),
            wake,
            behavior: PhantomData,
        }
    }

    async fn build_card(&self, settings: &Settings, view: B::View) -> Card {
        let Some(location) = self.services.location_for(settings) else {
            return views::no_location();
        };
        match B::card(&self.services, &location, settings, view, Utc::now()).await {
            Ok(card) => card,
            Err(e) => {
                log::warn!("{} for {} failed: {e}", B::UUID, location.name);
                views::no_data(&location.name)
            }
        }
    }

    /// Builds the card for `id`'s current state and sends it, unless the
    /// state changed meanwhile (a newer render follows) or it's unchanged.
    async fn render_on(&self, id: &str, surface: &impl Surface) {
        let Some(snapshot) = self.tracker.snapshot(id) else {
            return; // disappeared meanwhile
        };
        let card = self.build_card(&snapshot.settings, snapshot.view).await;
        let _sending = self.tracker.sending().await;
        if let Frame::Send { first } = self.tracker.claim_frame(id, snapshot.revision, &card)
            && let Err(e) = show(surface, &card, first).await
        {
            log::warn!("render failed: {e}");
            self.tracker.forget_frame(id);
        }
    }

    /// Renders in the background: building a card can wait on the network,
    /// and that mustn't hold up OpenDeck's event stream.
    fn spawn_render(&self, id: String) {
        let action = self.clone();
        tokio::spawn(async move {
            if let Some(instance) = openaction::get_instance(id.clone()).await {
                action.render_on(&id, &*instance).await;
            }
        });
    }

    // The event handlers' bodies. Plain `fn`s on plain data, not `async`:
    // awaiting the network in one is a compile error (CLAUDE.md "Handlers
    // never await the network"), and tests can call them without an
    // `openaction::Instance`, which only openaction can build.

    /// The instance appeared, or its settings changed: track the new
    /// settings and redraw it.
    fn on_settings(&self, id: &str, settings: &Settings) {
        self.tracker.track(id, settings.clone());
        self.spawn_render(id.to_string());
    }

    fn on_disappear(&self, id: &str) {
        self.tracker.untrack(id);
    }

    /// Applies an interaction to one instance's view, then re-renders it.
    fn on_interaction(&self, id: &str, f: impl FnOnce(B::View) -> B::View) {
        if self.tracker.interact(id, f) {
            self.spawn_render(id.to_string());
        }
    }

    /// Answers the property inspector in the background: a search waits on
    /// the network.
    fn on_inspector_message(&self, id: String, payload: &Value) {
        let Ok(message) = serde_json::from_value::<InspectorMessage>(payload.clone()) else {
            // The event name only: the payload could be anything, any size.
            let event: String = (payload.get("event").and_then(Value::as_str))
                .unwrap_or("?")
                .chars()
                .take(40)
                .collect();
            log::warn!("ignoring unknown property inspector message {event:?}");
            return;
        };
        let (services, wake) = (self.services.clone(), self.wake.clone());
        tokio::spawn(async move {
            let reply = answer(&services, &wake, message).await;
            if let Some(instance) = openaction::get_instance(id).await
                && let Err(e) = instance.send_to_property_inspector(reply).await
            {
                log::warn!("reply to property inspector failed: {e}");
            }
        });
    }
}

impl<B: Behavior> Scheduled for CardAction<B> {
    fn has_instances(&self) -> bool {
        !self.tracker.is_empty()
    }

    fn next_expiry(&self) -> Option<Instant> {
        self.tracker.next_expiry(VIEW_TIMEOUT)
    }

    fn expire(&self) -> Vec<String> {
        self.tracker.expire(VIEW_TIMEOUT)
    }

    fn ids(&self) -> Vec<String> {
        self.tracker.ids()
    }

    fn render_soon(&self, id: String) {
        self.spawn_render(id);
    }
}

/// Each handler is one call into the plain-`fn` bodies above.
#[async_trait]
impl<B: Behavior> Action for CardAction<B> {
    const UUID: &'static str = B::UUID;
    type Settings = Settings;

    async fn will_appear(&self, instance: &Instance, settings: &Settings) -> OpenActionResult<()> {
        self.on_settings(&instance.instance_id, settings);
        Ok(())
    }

    async fn did_receive_settings(
        &self,
        instance: &Instance,
        settings: &Settings,
    ) -> OpenActionResult<()> {
        self.on_settings(&instance.instance_id, settings);
        Ok(())
    }

    async fn will_disappear(&self, instance: &Instance, _: &Settings) -> OpenActionResult<()> {
        self.on_disappear(&instance.instance_id);
        Ok(())
    }

    async fn key_up(&self, instance: &Instance, _: &Settings) -> OpenActionResult<()> {
        self.on_interaction(&instance.instance_id, B::key_press);
        Ok(())
    }

    async fn dial_rotate(
        &self,
        instance: &Instance,
        _: &Settings,
        ticks: i16,
        _pressed: bool,
    ) -> OpenActionResult<()> {
        self.on_interaction(&instance.instance_id, |v| B::dial_rotate(v, ticks));
        Ok(())
    }

    async fn dial_up(&self, instance: &Instance, _: &Settings) -> OpenActionResult<()> {
        self.on_interaction(&instance.instance_id, B::dial_press);
        Ok(())
    }

    async fn touch_tap(
        &self,
        instance: &Instance,
        _: &Settings,
        _position: (u16, u16),
        _hold: bool,
    ) -> OpenActionResult<()> {
        self.on_interaction(&instance.instance_id, B::dial_press);
        Ok(())
    }

    async fn send_to_plugin(
        &self,
        instance: &Instance,
        _: &Settings,
        payload: &Value,
    ) -> OpenActionResult<()> {
        self.on_inspector_message(instance.instance_id.clone(), payload);
        Ok(())
    }
}

/// Where a card is drawn: a key or a dial. `Instance` in production; a
/// recording fake in tests.
#[async_trait]
pub trait Surface: Send + Sync {
    fn controller(&self) -> &str;
    async fn set_title(&self, title: String) -> OpenActionResult<()>;
    async fn set_image(&self, image: String) -> OpenActionResult<()>;
    async fn set_feedback(&self, feedback: Value) -> OpenActionResult<()>;
}

#[async_trait]
impl Surface for Instance {
    fn controller(&self) -> &str {
        &self.controller
    }

    async fn set_title(&self, title: String) -> OpenActionResult<()> {
        Instance::set_title(self, Some(title), None).await
    }

    async fn set_image(&self, image: String) -> OpenActionResult<()> {
        Instance::set_image(self, Some(image), None).await
    }

    async fn set_feedback(&self, feedback: Value) -> OpenActionResult<()> {
        Instance::set_feedback(self, &feedback).await
    }
}

/// Draws `card`. `first`: the first frame since the instance appeared.
async fn show(surface: &impl Surface, card: &Card, first: bool) -> OpenActionResult<()> {
    if surface.controller() == KEYPAD_CONTROLLER {
        if first {
            // The text is drawn inside the image; clear the native title
            // (once per appearance) so OpenDeck doesn't paint a second copy.
            surface.set_title(String::new()).await?;
        }
        surface.set_image(card::key_image(card)).await
    } else {
        surface.set_feedback(card::feedback(card)).await
    }
}

/// Requests from the property inspector (`sendToPlugin`).
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "camelCase")]
enum InspectorMessage {
    /// Geocode a city or postal code - the plugin does it so the inspector
    /// needs no network access of its own.
    Search { query: String },
    /// Which location instances without their own one use.
    GetDefaultLocation,
    /// Make (or with `null`, clear) the plugin-wide default location.
    SetDefaultLocation { location: Option<Location> },
}

/// Replies to the property inspector (`sendToPropertyInspector`).
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "event", rename_all = "camelCase")]
enum PluginMessage {
    SearchResults {
        query: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        results: Option<Vec<Place>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    DefaultLocation {
        location: Option<Location>,
    },
}

async fn answer(services: &Services, wake: &Wake, message: InspectorMessage) -> PluginMessage {
    match message {
        InspectorMessage::Search { query } => {
            let (results, error) = if query.chars().count() > MAX_QUERY_CHARS {
                (
                    None,
                    Some(format!("search text over {MAX_QUERY_CHARS} characters")),
                )
            } else {
                match services.search(&query).await {
                    Ok(results) => (Some(results), None),
                    Err(e) => {
                        log::warn!("location search failed: {e}");
                        (None, Some(e.to_string()))
                    }
                }
            };
            PluginMessage::SearchResults {
                query,
                results,
                error,
            }
        }
        InspectorMessage::GetDefaultLocation => PluginMessage::DefaultLocation {
            location: services.default_location(),
        },
        InspectorMessage::SetDefaultLocation { location } => {
            let global = GlobalSettings {
                default_location: location.clone(),
            };
            if let Err(e) = openaction::set_global_settings(&global).await {
                log::warn!("saving the default location failed: {e}");
            }
            apply_default_location(services, wake, location.clone());
            PluginMessage::DefaultLocation { location }
        }
    }
}

fn apply_default_location(services: &Services, wake: &Wake, location: Option<Location>) {
    if services.set_default_location(location) {
        wake.refresh_all();
    }
}

/// Loads the plugin-wide settings when the plugin connects, and follows
/// changes to them.
pub struct GlobalSettingsHandler {
    pub services: Arc<Services>,
    pub wake: Arc<Wake>,
}

impl GlobalSettingsHandler {
    /// The stored global settings arrived. Plain `fn`, like the action
    /// handlers' bodies. They are the source of truth: unreadable ones mean
    /// no default, the same as `null`.
    fn on_global_settings(&self, settings: Value) {
        let global: GlobalSettings = serde_json::from_value(settings).unwrap_or_default();
        apply_default_location(&self.services, &self.wake, global.default_location);
    }
}

#[async_trait]
impl GlobalEventHandler for GlobalSettingsHandler {
    async fn plugin_ready(&self) -> OpenActionResult<()> {
        openaction::get_global_settings().await
    }

    async fn did_receive_global_settings(
        &self,
        event: DidReceiveGlobalSettingsEvent,
    ) -> OpenActionResult<()> {
        self.on_global_settings(event.payload.settings);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::Glyph;
    use crate::model::{AqiScale, Units};
    use crate::open_meteo::FORECAST_DAYS;
    use serde_json::json;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Notify;

    const ALL_UUIDS: [&str; 3] = [
        weather::Weather::UUID,
        forecast::Forecast::UUID,
        air_quality::AirQuality::UUID,
    ];

    #[test]
    fn parses_the_inspectors_requests() {
        let msg: InspectorMessage =
            serde_json::from_value(json!({"event": "search", "query": "Porto"})).unwrap();
        assert_eq!(
            msg,
            InspectorMessage::Search {
                query: "Porto".into()
            }
        );
        let msg: InspectorMessage =
            serde_json::from_value(json!({"event": "setDefaultLocation", "location": null}))
                .unwrap();
        assert_eq!(msg, InspectorMessage::SetDefaultLocation { location: None });
        let msg: InspectorMessage =
            serde_json::from_value(json!({"event": "getDefaultLocation"})).unwrap();
        assert_eq!(msg, InspectorMessage::GetDefaultLocation);
    }

    #[test]
    fn replies_have_the_shape_the_inspector_reads() {
        let ok = PluginMessage::SearchResults {
            query: "Porto".into(),
            results: Some(vec![]),
            error: None,
        };
        assert_eq!(
            serde_json::to_value(ok).unwrap(),
            json!({"event": "searchResults", "query": "Porto", "results": []})
        );
        let failed = PluginMessage::SearchResults {
            query: "Porto".into(),
            results: None,
            error: Some("timed out".into()),
        };
        assert_eq!(
            serde_json::to_value(failed).unwrap(),
            json!({"event": "searchResults", "query": "Porto", "error": "timed out"})
        );
        let default = PluginMessage::DefaultLocation { location: None };
        assert_eq!(
            serde_json::to_value(default).unwrap(),
            json!({"event": "defaultLocation", "location": null})
        );
    }

    #[tokio::test]
    async fn an_overlong_search_is_refused_without_a_request() {
        let services = Services::default();
        let reply = answer(
            &services,
            &Wake::default(),
            InspectorMessage::Search {
                query: "x".repeat(MAX_QUERY_CHARS + 1),
            },
        )
        .await;
        let PluginMessage::SearchResults { results, error, .. } = reply else {
            panic!("{reply:?}");
        };
        assert_eq!(results, None);
        assert!(error.unwrap().contains("characters"));
    }

    #[test]
    fn a_new_default_location_is_stored_and_redraws_everything() {
        let services = Services::default();
        let wake = Wake::default();
        let lisbon = Location {
            name: "Lisbon".into(),
            latitude: 38.7,
            longitude: -9.1,
            label: None,
        };
        apply_default_location(&services, &wake, Some(lisbon.clone()));
        assert_eq!(services.default_location(), Some(lisbon.clone()));
        assert!(wake.take_refresh_all());
        apply_default_location(&services, &wake, Some(lisbon));
        assert!(!wake.take_refresh_all(), "unchanged: nothing to redraw");
    }

    #[test]
    fn every_action_uuid_is_in_the_shipped_manifest() {
        let manifest: Value =
            serde_json::from_str(include_str!("../../assets/manifest.json")).unwrap();
        let uuids: Vec<&str> = manifest["Actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["UUID"].as_str().unwrap())
            .collect();
        assert_eq!(uuids.len(), ALL_UUIDS.len());
        for uuid in ALL_UUIDS {
            assert!(uuids.contains(&uuid), "{uuid} missing from manifest");
        }
    }

    /// The property inspector repeats a few facts from the Rust side; pin
    /// them so neither side can drift alone.
    #[test]
    fn the_property_inspector_agrees_with_the_plugin() {
        let html = include_str!("../../assets/propertyInspector/index.html");

        // Per-action fields name real action UUIDs.
        let named: Vec<&str> = html
            .split("data-action=\"")
            .skip(1)
            .map(|rest| &rest[..rest.find('"').unwrap()])
            .collect();
        assert!(!named.is_empty());
        for uuid in &named {
            assert!(
                ALL_UUIDS.contains(uuid),
                "unknown action {uuid} in the inspector"
            );
        }
        assert!(named.contains(&forecast::Forecast::UUID));
        assert!(named.contains(&air_quality::AirQuality::UUID));

        // Its fallbacks for unset fields are the plugin's defaults.
        let d = Settings::default();
        let name = |v: &dyn erased::Named| v.wire_name();
        assert!(html.contains(&format!("s.day ?? {}", d.day)), "day default");
        assert!(
            html.contains(&format!("s.units || \"{}\"", name(&d.units))),
            "units default"
        );
        assert!(
            html.contains(&format!("s.aqi_scale || \"{}\"", name(&d.aqi_scale))),
            "AQI default"
        );

        // One Day option per forecast day.
        let day_select = &html[html.find("<select id=\"day\">").unwrap()..];
        let day_select = &day_select[..day_select.find("</select>").unwrap()];
        assert_eq!(
            day_select.matches("<option").count(),
            usize::from(FORECAST_DAYS)
        );

        // The search box can't send more than the plugin accepts.
        assert!(html.contains(&format!("maxlength=\"{MAX_QUERY_CHARS}\"")));
    }

    /// Serde wire names of the settings enums, for the contract test above.
    mod erased {
        pub trait Named {
            fn wire_name(&self) -> String;
        }
        impl<T: serde::Serialize> Named for T {
            fn wire_name(&self) -> String {
                serde_json::to_value(self)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            }
        }
    }

    #[test]
    fn wire_names_are_what_the_inspector_sends() {
        let name = |v: &dyn erased::Named| v.wire_name();
        assert_eq!(name(&Units::Imperial), "imperial");
        assert_eq!(name(&AqiScale::European), "european");
    }

    /// Records what was sent to it.
    struct FakeSurface {
        controller: &'static str,
        sent: Mutex<Vec<(&'static str, String)>>,
        /// The next `set_image` fails (and records nothing).
        fail_next_image: AtomicBool,
    }

    impl FakeSurface {
        fn new(controller: &'static str) -> Self {
            FakeSurface {
                controller,
                sent: Mutex::default(),
                fail_next_image: AtomicBool::new(false),
            }
        }

        fn take(&self) -> Vec<(&'static str, String)> {
            std::mem::take(&mut *self.sent.lock().unwrap())
        }
    }

    #[async_trait]
    impl Surface for FakeSurface {
        fn controller(&self) -> &str {
            self.controller
        }
        async fn set_title(&self, title: String) -> OpenActionResult<()> {
            self.sent.lock().unwrap().push(("title", title));
            Ok(())
        }
        async fn set_image(&self, image: String) -> OpenActionResult<()> {
            if self.fail_next_image.swap(false, Ordering::SeqCst) {
                let broken = serde_json::from_str::<Value>("{").unwrap_err();
                return Err(openaction::OpenActionError::Serde(broken));
            }
            self.sent.lock().unwrap().push(("image", image));
            Ok(())
        }
        async fn set_feedback(&self, feedback: Value) -> OpenActionResult<()> {
            self.sent
                .lock()
                .unwrap()
                .push(("feedback", feedback.to_string()));
            Ok(())
        }
    }

    fn echo_card(view: i32) -> Card {
        Card::new(Glyph::Pin, view.to_string(), "label", "detail")
    }

    /// Shows its view as the value; view 0 waits for `GATE` first, so a
    /// test can change the state while a render is in flight.
    struct Echo;
    static GATE: std::sync::LazyLock<Notify> = std::sync::LazyLock::new(Notify::new);

    #[async_trait]
    impl Behavior for Echo {
        const UUID: &'static str = "test.echo";
        type View = i32;
        fn key_press(v: i32) -> i32 {
            v + 1
        }
        fn dial_rotate(v: i32, ticks: i16) -> i32 {
            v + i32::from(ticks)
        }
        fn dial_press(_: i32) -> i32 {
            0
        }
        async fn card(
            _: &Services,
            _: &Location,
            _: &Settings,
            view: i32,
            _: DateTime<Utc>,
        ) -> Result<Card, FetchError> {
            if view == 100 {
                GATE.notified().await;
            }
            Ok(echo_card(view))
        }
    }

    fn located() -> Settings {
        Settings {
            location: Some(Location {
                name: "Lisbon".into(),
                latitude: 38.7,
                longitude: -9.1,
                label: None,
            }),
            ..Settings::default()
        }
    }

    fn echo() -> CardAction<Echo> {
        CardAction::new(Arc::default(), Arc::default())
    }

    #[tokio::test]
    async fn a_key_gets_its_title_cleared_once_then_only_changed_images() {
        let action = echo();
        let key = FakeSurface::new("Keypad");
        action.tracker.track("k", located());

        action.render_on("k", &key).await;
        assert_eq!(
            key.take(),
            [
                ("title", String::new()),
                ("image", card::key_image(&echo_card(0)))
            ]
        );

        action.render_on("k", &key).await;
        assert!(key.take().is_empty(), "an unchanged card is not re-sent");

        action.tracker.interact("k", Echo::key_press);
        action.render_on("k", &key).await;
        assert_eq!(key.take(), [("image", card::key_image(&echo_card(1)))]);
    }

    #[tokio::test]
    async fn a_dial_gets_feedback_and_never_an_image_or_title() {
        let action = echo();
        let dial = FakeSurface::new("Encoder");
        action.tracker.track("d", located());
        action.render_on("d", &dial).await;
        assert_eq!(
            dial.take(),
            [("feedback", card::feedback(&echo_card(0)).to_string())]
        );
    }

    #[tokio::test]
    async fn an_instance_without_a_location_asks_for_one() {
        let action = echo();
        let key = FakeSurface::new("Keypad");
        action.tracker.track("k", Settings::default());
        action.render_on("k", &key).await;
        let sent = key.take();
        assert_eq!(sent[1], ("image", card::key_image(&views::no_location())));
    }

    #[tokio::test]
    async fn the_default_location_stands_in_for_a_missing_one() {
        let action = echo();
        let key = FakeSurface::new("Keypad");
        action.services.set_default_location(located().location);
        action.tracker.track("k", Settings::default());
        action.render_on("k", &key).await;
        assert_eq!(key.take()[1], ("image", card::key_image(&echo_card(0))));
    }

    #[tokio::test]
    async fn a_render_overtaken_by_an_interaction_is_dropped() {
        let action = echo();
        let key = Arc::new(FakeSurface::new("Keypad"));
        action.tracker.track("k", located());
        action.tracker.interact("k", |_| 100);

        // Starts building view 100's card, which waits on the gate.
        let slow = {
            let (action, key) = (action.clone(), key.clone());
            tokio::spawn(async move { action.render_on("k", &*key).await })
        };
        tokio::task::yield_now().await;

        action.tracker.interact("k", |_| 7);
        action.render_on("k", &*key).await;
        GATE.notify_one();
        slow.await.unwrap();

        let images: Vec<String> = key
            .take()
            .into_iter()
            .filter(|(kind, _)| *kind == "image")
            .map(|(_, image)| image)
            .collect();
        assert_eq!(images, [card::key_image(&echo_card(7))]);
    }

    fn lisbon() -> Location {
        located().location.unwrap()
    }

    fn porto() -> Location {
        Location {
            name: "Porto".into(),
            latitude: 41.1,
            longitude: -8.6,
            label: None,
        }
    }

    #[tokio::test]
    async fn new_settings_replace_the_tracked_ones() {
        let action = echo();
        action.on_settings("k", &located());
        let moved = Settings {
            location: Some(porto()),
            ..Settings::default()
        };
        action.on_settings("k", &moved);
        assert_eq!(action.tracker.snapshot("k").unwrap().settings, moved);
    }

    #[tokio::test]
    async fn a_disappeared_instance_is_forgotten() {
        let action = echo();
        action.on_settings("k", &located());
        assert!(action.has_instances());
        action.on_disappear("k");
        assert!(action.tracker.snapshot("k").is_none());
        assert!(!action.has_instances(), "the scheduler can park");
    }

    #[tokio::test]
    async fn an_interaction_changes_only_a_tracked_instance() {
        let action = echo();
        action.on_settings("k", &located());
        action.on_interaction("k", Echo::key_press);
        assert_eq!(action.tracker.snapshot("k").unwrap().view, 1);
        action.on_interaction("gone", Echo::key_press);
        assert!(action.tracker.snapshot("gone").is_none());
    }

    #[tokio::test]
    async fn inspector_messages_return_without_waiting_for_an_answer() {
        // A search would go to the network; the handler only spawns it.
        let action = echo();
        action.on_inspector_message("k".into(), &json!({"event": "search", "query": "Porto"}));
        action.on_inspector_message("k".into(), &json!({"event": "nonsense"}));
        action.on_inspector_message("k".into(), &json!(42));
    }

    fn global_settings() -> GlobalSettingsHandler {
        GlobalSettingsHandler {
            services: Arc::default(),
            wake: Arc::default(),
        }
    }

    #[test]
    fn the_saved_default_location_is_loaded_and_redraws_everything() {
        let handler = global_settings();
        // The wire shape OpenDeck sends, through openaction's own type.
        let event: DidReceiveGlobalSettingsEvent = serde_json::from_value(json!({
            "payload": {"settings": {"default_location": {
                "name": "Lisbon", "latitude": 38.7, "longitude": -9.1
            }}}
        }))
        .unwrap();
        handler.on_global_settings(event.payload.settings);
        assert_eq!(handler.services.default_location(), Some(lisbon()));
        assert!(handler.wake.take_refresh_all());
    }

    #[test]
    fn unreadable_global_settings_mean_no_default_and_never_panic() {
        let handler = global_settings();
        for garbage in [
            json!(null),
            json!("x"),
            json!([1]),
            json!({"default_location": 7}),
        ] {
            handler.on_global_settings(garbage);
            assert_eq!(handler.services.default_location(), None);
            assert!(!handler.wake.take_refresh_all(), "nothing changed");
        }
    }

    #[tokio::test]
    async fn a_frame_that_failed_to_send_is_sent_again() {
        let action = echo();
        let key = FakeSurface::new("Keypad");
        action.tracker.track("k", located());
        key.fail_next_image.store(true, Ordering::SeqCst);
        action.render_on("k", &key).await;
        assert_eq!(key.take(), [("title", String::new())], "the image failed");

        action.render_on("k", &key).await;
        assert_eq!(
            key.take(),
            [
                ("title", String::new()),
                ("image", card::key_image(&echo_card(0)))
            ],
            "the same card again, in full"
        );
    }
}
