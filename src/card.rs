//! What one action instance shows, independent of the surface it's on.
//! Every action builds a `Card`; `key_image` draws it on a keypad tile and
//! `feedback` sends it to a dial's touch strip (`assets/layouts/card.json`).
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
        body
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

/// `setFeedback` payload for `assets/layouts/card.json`.
pub fn feedback(card: &Card) -> Value {
    json!({
        "icon": glyphs::document(&card.glyph_body()),
        "value": { "value": card.value, "color": card.value_color() },
        "label": card.label,
        "detail": card.detail,
    })
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
    fn feedback_keys_match_the_shipped_layout() {
        let layout: Value =
            serde_json::from_str(include_str!("../assets/layouts/card.json")).unwrap();
        let keys: Vec<&str> = layout["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["key"].as_str().unwrap())
            .collect();
        let fb = feedback(&card());
        for k in fb.as_object().unwrap().keys() {
            assert!(keys.contains(&k.as_str()), "layout has no item keyed {k}");
        }
    }

    #[test]
    fn the_shipped_layout_uses_the_same_text_colors_as_keys() {
        let layout: Value =
            serde_json::from_str(include_str!("../assets/layouts/card.json")).unwrap();
        let color = |key: &str| {
            layout["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["key"] == key)
                .unwrap()["color"]
                .clone()
        };
        assert_eq!(color("value"), palette::TEXT);
        assert_eq!(color("label"), palette::MUTED_TEXT);
        assert_eq!(color("detail"), palette::MUTED_TEXT);
    }

    #[test]
    fn feedback_colors_the_value_with_the_accent() {
        let c = card().with_accent(Some("#22c55e"));
        assert_eq!(feedback(&c)["value"]["color"], "#22c55e");
    }
}
