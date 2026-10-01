//! What one action instance shows, independent of the surface it's on.
//! Every action builds a `Card`; `key_image` draws it on a keypad tile and
//! `feedback` sends it to a dial's touch strip (`assets/layouts/card.json`),
//! so a key and a dial showing the same thing can never disagree.
//!
//! Key text is drawn inside the image rather than sent as the native title:
//! OpenDeck paints native titles with each key's own font/size/alignment on
//! top of the image, which on a small key made the text hard to read and
//! inconsistent between tiles (same rationale as opendeck-claude-usage).

use crate::glyphs::{self, Glyph};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

const CARD_COLOR: &str = "#111827";
pub const TEXT_COLOR: &str = "#f9fafb";
const MUTED_TEXT_COLOR: &str = "#d1d5db";

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
    /// Second line, shown on keys and dials.
    pub label: String,
    /// Third line - dials only (keys have no room for it).
    pub detail: String,
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

pub fn key_svg(card: &Card) -> String {
    let glyph = glyphs::body(card.glyph);
    let value = text_line(
        73.0,
        26.0,
        true,
        card.accent.unwrap_or(TEXT_COLOR),
        &card.value,
    );
    let label = text_line(92.0, 14.0, false, MUTED_TEXT_COLOR, &card.label);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="{CARD_COLOR}"/><svg x="28" y="3" width="44" height="44" viewBox="0 0 24 24">{glyph}</svg>{value}{label}</svg>"#
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
        "icon": glyphs::svg(card.glyph),
        "value": { "value": card.value, "color": card.accent.unwrap_or(TEXT_COLOR) },
        "label": card.label,
        "detail": card.detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wmo::Condition;

    fn card() -> Card {
        Card {
            glyph: Glyph::Weather {
                condition: Condition::Rain,
                is_day: true,
            },
            value: "18°".into(),
            accent: None,
            label: "Rain & wind".into(),
            detail: "Lisbon".into(),
        }
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
    }

    #[test]
    fn long_key_lines_are_squeezed_to_fit() {
        let mut c = card();
        c.label = "Thunderstorm with hail".into();
        assert!(key_svg(&c).contains(r#"textLength="94""#));
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
    fn feedback_colors_the_value_with_the_accent() {
        let mut c = card();
        c.accent = Some("#22c55e");
        assert_eq!(feedback(&c)["value"]["color"], "#22c55e");
    }
}
