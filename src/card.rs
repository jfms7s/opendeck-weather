//! What one action instance shows, independent of the surface it's on.
//! Every action builds a `Card`; `key_image` draws it on a keypad tile and
//! `feedback` sends it to a dial's touch strip (`assets/layouts/card.json`,
//! or `timeline.json` for a card with a chart - see `Layout`).
//!
//! The two surfaces don't have the same room: a dial shows the value plus
//! two lines (`label`, `detail`), a key the value plus one. A key draws
//! `key_line()` - `label` unless the view chose something else on purpose
//! with `with_key_label` - so what a key shows is a decision each view
//! makes, and the per-view tests in `views.rs` pin it.
//!
//! Key text is drawn inside the image rather than sent as the native title:
//! OpenDeck paints native titles with each key's own font/size/alignment on
//! top of the image, which on a small key made the text hard to read and
//! inconsistent between tiles (same rationale as opendeck-claude-usage).

use crate::glyphs::{self, Glyph};
use crate::ipma::Level;
use crate::palette;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

/// Widest a key text line may render, in viewBox units (of 100).
const MAX_TEXT_WIDTH: f64 = 94.0;
/// Rough average glyph advance as a fraction of font size - only used to
/// decide when a line needs squeezing, so erring wide is the safe side.
const REGULAR_CHAR_WIDTH: f64 = 0.58;
const BOLD_CHAR_WIDTH: f64 = 0.64;

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub glyph: Glyph,
    /// The big line: a temperature, a high/low pair, an AQI.
    pub value: String,
    /// Color for `value`; `None` = default text color.
    pub accent: Option<&'static str>,
    /// Second line on a dial, and on a key unless `key_label` is set.
    pub label: String,
    /// Third line - dials only (keys have no room for it).
    pub detail: String,
    /// What a key shows instead of `label`, when the two should differ.
    pub key_label: Option<String>,
    /// The data is older than the cache TTL (refreshing it failed): drawn
    /// muted, with a small clock badge, on both surfaces.
    pub stale: bool,
    /// The highest IPMA warning active at the location (Portugal only): a
    /// small triangle in its colour, top-left, on both surfaces.
    pub warning: Option<Level>,
    /// A chart for the dial's timeline layout (a standalone SVG document,
    /// `chart::timeline`). Keys never show one.
    pub timeline: Option<String>,
}

/// The touch-strip layouts a dial can show. A dial starts on the
/// manifest's (`Card`); switching needs a `setFeedbackLayout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Card,
    Timeline,
}

impl Layout {
    /// The path `setFeedbackLayout` takes, relative to the bundle.
    pub fn path(self) -> &'static str {
        match self {
            Layout::Card => "layouts/card.json",
            Layout::Timeline => "layouts/timeline.json",
        }
    }
}

impl Card {
    pub fn new(
        glyph: Glyph,
        value: impl Into<String>,
        label: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Card {
            glyph,
            value: value.into(),
            accent: None,
            label: label.into(),
            detail: detail.into(),
            key_label: None,
            stale: false,
            warning: None,
            timeline: None,
        }
    }

    pub fn with_accent(self, accent: Option<&'static str>) -> Self {
        Card { accent, ..self }
    }

    pub fn with_key_label(self, key_label: impl Into<String>) -> Self {
        Card {
            key_label: Some(key_label.into()),
            ..self
        }
    }

    pub fn marked_stale(self, stale: bool) -> Self {
        Card { stale, ..self }
    }

    pub fn with_warning(self, warning: Option<Level>) -> Self {
        Card { warning, ..self }
    }

    pub fn with_timeline(self, chart: String) -> Self {
        Card {
            timeline: Some(chart),
            ..self
        }
    }

    /// The layout a dial needs for this card.
    pub fn layout(&self) -> Layout {
        if self.timeline.is_some() {
            Layout::Timeline
        } else {
            Layout::Card
        }
    }

    /// The one text line a key has room for under the value.
    pub fn key_line(&self) -> &str {
        self.key_label.as_deref().unwrap_or(&self.label)
    }

    fn value_color(&self) -> &'static str {
        if self.stale {
            palette::STALE
        } else {
            self.accent.unwrap_or(palette::TEXT)
        }
    }

    fn glyph_body(&self) -> String {
        let mut body = glyphs::body(self.glyph);
        if self.stale {
            body.push_str(&glyphs::stale_badge());
        }
        if let Some(level) = self.warning {
            body.push_str(&glyphs::warning_badge(warning_colour(level)));
        }
        body
    }
}

fn warning_colour(level: Level) -> &'static str {
    match level {
        Level::Yellow => palette::WARNING_YELLOW,
        Level::Orange => palette::WARNING_ORANGE,
        Level::Red => palette::WARNING_RED,
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// One horizontally-centered line with its baseline at `y`, squeezed via
/// `textLength` rather than clipped when it would overflow the key.
fn text_line(y: f64, size: f64, bold: bool, color: &str, content: &str) -> String {
    let char_width = if bold {
        BOLD_CHAR_WIDTH
    } else {
        REGULAR_CHAR_WIDTH
    };
    let estimated_width = content.chars().count() as f64 * size * char_width;
    let fit = if estimated_width > MAX_TEXT_WIDTH {
        format!(r#" textLength="{MAX_TEXT_WIDTH}" lengthAdjust="spacingAndGlyphs""#)
    } else {
        String::new()
    };
    let weight = if bold { "700" } else { "500" };
    let escaped = escape_xml(content);
    format!(
        r#"<text x="50" y="{y}" text-anchor="middle" font-family="sans-serif" font-size="{size}" font-weight="{weight}" fill="{color}"{fit}>{escaped}</text>"#
    )
}

pub(crate) fn key_svg(card: &Card) -> String {
    let background = palette::BACKGROUND;
    let glyph = card.glyph_body();
    let value = text_line(73.0, 26.0, true, card.value_color(), &card.value);
    let label = text_line(92.0, 14.0, false, palette::MUTED_TEXT, card.key_line());
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="{background}"/><svg x="28" y="3" width="44" height="44" viewBox="0 0 24 24">{glyph}</svg>{value}{label}</svg>"#
    )
}

/// The `image` string OpenDeck's `setImage` expects: it only treats `image`
/// as inline data when it starts with `data:` - anything else is read as a
/// file path inside the plugin bundle.
pub fn key_image(card: &Card) -> String {
    let encoded = STANDARD.encode(key_svg(card).as_bytes());
    format!("data:image/svg+xml;base64,{encoded}")
}

/// `setFeedback` payload for the card's `layout()`: `card.json`, or
/// `timeline.json` (icon, value and label on the left, the chart on the
/// right, no detail line).
pub fn feedback(card: &Card) -> Value {
    let icon = glyphs::document(&card.glyph_body());
    let value = json!({ "value": card.value, "color": card.value_color() });
    match &card.timeline {
        None => json!({
            "icon": icon,
            "value": value,
            "label": card.label,
            "detail": card.detail,
        }),
        Some(chart) => json!({
            "icon": icon,
            "value": value,
            "label": card.label,
            "chart": chart,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::tests::assert_well_formed;
    use crate::wmo::Condition;

    fn card() -> Card {
        Card::new(
            Glyph::Weather {
                condition: Condition::Rain,
                is_day: true,
            },
            "18°",
            "Rain & wind",
            "Lisbon",
        )
    }

    fn decode(uri: &str) -> String {
        let prefix = "data:image/svg+xml;base64,";
        assert!(uri.starts_with(prefix), "got: {uri}");
        String::from_utf8(STANDARD.decode(&uri[prefix.len()..]).unwrap()).unwrap()
    }

    #[test]
    fn key_image_draws_value_and_escaped_label_but_not_detail() {
        let svg = decode(&key_image(&card()));
        assert!(svg.contains(">18°</text>"), "{svg}");
        assert!(svg.contains(">Rain &amp; wind</text>"), "{svg}");
        assert!(!svg.contains("Lisbon"), "{svg}");
        assert_well_formed(&svg);
    }

    #[test]
    fn a_key_label_replaces_the_label_on_keys_only() {
        let c = card().with_key_label("Tomorrow · 70%");
        let svg = key_svg(&c);
        assert!(svg.contains(">Tomorrow · 70%</text>"), "{svg}");
        assert!(!svg.contains("Rain &amp; wind"), "{svg}");
        assert_eq!(feedback(&c)["label"], "Rain & wind");
    }

    #[test]
    fn long_key_lines_are_squeezed_to_fit() {
        let mut c = card();
        c.label = "Thunderstorm with hail".into();
        assert!(key_svg(&c).contains(r#"textLength="94""#));
    }

    #[test]
    fn stale_cards_are_muted_and_badged_on_both_surfaces() {
        let fresh = card().with_accent(Some("#22c55e"));
        let stale = fresh.clone().marked_stale(true);
        let badge = glyphs::stale_badge();

        assert!(!key_svg(&fresh).contains(&badge));
        assert!(key_svg(&stale).contains(&badge));
        assert!(key_svg(&stale).contains(&format!(r#"fill="{}">18°<"#, palette::STALE)));
        assert_well_formed(&key_svg(&stale));

        assert_eq!(feedback(&fresh)["value"]["color"], "#22c55e");
        assert_eq!(feedback(&stale)["value"]["color"], palette::STALE);
        let icon = feedback(&stale)["icon"].as_str().unwrap().to_string();
        assert!(icon.contains(&badge));
        assert_well_formed(&icon);
    }

    #[test]
    fn a_warning_badges_both_surfaces_in_its_level_colour() {
        let plain = card();
        assert!(!key_svg(&plain).contains(palette::WARNING_ORANGE));
        let icon = |c: &Card| feedback(c)["icon"].as_str().unwrap().to_string();
        assert!(!icon(&plain).contains("M4 0.4"));

        for (level, colour) in [
            (Level::Yellow, palette::WARNING_YELLOW),
            (Level::Orange, palette::WARNING_ORANGE),
            (Level::Red, palette::WARNING_RED),
        ] {
            let warned = card().with_warning(Some(level));
            let badge = glyphs::warning_badge(colour);
            assert!(key_svg(&warned).contains(&badge), "{level:?}");
            assert!(icon(&warned).contains(&badge), "{level:?}");
            assert_well_formed(&key_svg(&warned));
            assert_well_formed(&icon(&warned));
            // The text is untouched.
            assert_eq!(feedback(&warned)["value"], feedback(&plain)["value"]);
        }
    }

    #[test]
    fn a_warning_and_the_stale_clock_show_together() {
        let both = card().marked_stale(true).with_warning(Some(Level::Red));
        for svg in [
            key_svg(&both),
            feedback(&both)["icon"].as_str().unwrap().to_string(),
        ] {
            assert!(svg.contains(&glyphs::stale_badge()), "{svg}");
            assert!(
                svg.contains(&glyphs::warning_badge(palette::WARNING_RED)),
                "{svg}"
            );
            assert_well_formed(&svg);
        }
    }

    fn shipped_layout(layout: Layout) -> Value {
        let json = match layout {
            Layout::Card => include_str!("../assets/layouts/card.json"),
            Layout::Timeline => include_str!("../assets/layouts/timeline.json"),
        };
        serde_json::from_str(json).unwrap()
    }

    fn timeline_card() -> Card {
        card().with_timeline(crate::chart::timeline(&[], crate::model::Units::Metric))
    }

    #[test]
    fn feedback_keys_match_the_shipped_layout() {
        for c in [card(), timeline_card()] {
            let layout = shipped_layout(c.layout());
            let items = layout["items"].as_array().unwrap();
            let keys: Vec<&str> = items.iter().map(|i| i["key"].as_str().unwrap()).collect();
            let fb = feedback(&c);
            let sent: Vec<&String> = fb.as_object().unwrap().keys().collect();
            for k in &sent {
                assert!(
                    keys.contains(&k.as_str()),
                    "{:?} has no item keyed {k}",
                    c.layout()
                );
            }
            // Every item is filled, so nothing from another layout lingers.
            for k in &keys {
                assert!(
                    sent.iter().any(|s| s == k),
                    "{:?}: {k} never sent",
                    c.layout()
                );
            }
        }
    }

    #[test]
    fn the_layouts_paths_are_shipped_and_the_manifest_starts_on_the_card() {
        for layout in [Layout::Card, Layout::Timeline] {
            let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join(layout.path());
            assert!(file.is_file(), "{}", file.display());
            assert!(shipped_layout(layout)["id"].as_str().is_some());
        }
        let manifest: Value =
            serde_json::from_str(include_str!("../assets/manifest.json")).unwrap();
        for action in manifest["Actions"].as_array().unwrap() {
            assert_eq!(action["Encoder"]["layout"], Layout::Card.path());
        }
    }

    #[test]
    fn the_shipped_layouts_use_the_same_text_colors_as_keys() {
        for layout in [Layout::Card, Layout::Timeline] {
            let shipped = shipped_layout(layout);
            let color = |key: &str| {
                shipped["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|i| i["key"] == key)
                    .map(|i| i["color"].clone())
            };
            assert_eq!(
                color("value"),
                Some(Value::from(palette::TEXT)),
                "{layout:?}"
            );
            assert_eq!(
                color("label"),
                Some(Value::from(palette::MUTED_TEXT)),
                "{layout:?}"
            );
            if layout == Layout::Card {
                assert_eq!(color("detail"), Some(Value::from(palette::MUTED_TEXT)));
            }
        }
    }

    #[test]
    fn a_timeline_card_sends_its_chart_and_draws_a_plain_key() {
        let c = timeline_card();
        assert_eq!(c.layout(), Layout::Timeline);
        assert_eq!(card().layout(), Layout::Card);
        let fb = feedback(&c);
        assert_eq!(fb["chart"], Value::from(c.timeline.clone().unwrap()));
        assert!(fb.get("detail").is_none());
        assert_eq!(key_svg(&c), key_svg(&card()));
    }

    #[test]
    fn feedback_colors_the_value_with_the_accent() {
        let c = card().with_accent(Some("#22c55e"));
        assert_eq!(feedback(&c)["value"]["color"], "#22c55e");
    }
}
