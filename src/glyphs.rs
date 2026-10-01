//! Colored weather icons on a 24x24 grid. `body` returns bare SVG elements
//! so the key tile can nest them inside its own image; `svg` wraps them as a
//! standalone document for a touch-strip pixmap (OpenDeck's strip renderer
//! accepts an SVG string directly as a pixmap value).

use crate::wmo::Condition;

const SUN: &str = "#facc15";
const MOON: &str = "#e2e8f0";
const CLOUD: &str = "#cbd5e1";
const DARK_CLOUD: &str = "#94a3b8";
const RAIN: &str = "#60a5fa";
const SNOW: &str = "#f0f9ff";
const BOLT: &str = "#fbbf24";
const MUTED: &str = "#6b7280";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Glyph {
    Weather {
        condition: Condition,
        is_day: bool,
    },
    /// A colored ring for air quality, in the AQI category's color.
    Ring(&'static str),
    /// Map pin: no location configured yet.
    Pin,
    /// Crossed-out cloud: no data (offline, API error).
    Offline,
}

/// Cloud outline spanning roughly x 3..22, y 5..19.
fn cloud(fill: &str, dx: f64, dy: f64) -> String {
    format!(
        r#"<path transform="translate({dx} {dy})" d="M7.5 19H17a4 4 0 0 0 .6-7.95A5.5 5.5 0 0 0 7.1 10.6a4.2 4.2 0 0 0 .4 8.4Z" fill="{fill}"/>"#
    )
}

fn sun(cx: f64, cy: f64, r: f64) -> String {
    let rays: String = (0..8)
        .map(|i| {
            let a = f64::from(i) * std::f64::consts::FRAC_PI_4;
            let (inner, outer) = (r + 1.6, r + 3.4);
            format!(
                r#"<line x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}"/>"#,
                cx + inner * a.cos(),
                cy + inner * a.sin(),
                cx + outer * a.cos(),
                cy + outer * a.sin()
            )
        })
        .collect();
    format!(
        r#"<circle cx="{cx}" cy="{cy}" r="{r}" fill="{SUN}"/><g stroke="{SUN}" stroke-width="1.6" stroke-linecap="round">{rays}</g>"#
    )
}

/// Crescent moon of radius `r` centered on (cx, cy): the moon's disc minus
/// a bite taken by a second circle (radius 0.85r, centered at (0.5r, -0.3r)
/// from the moon's center). The two unit-circle points are where those
/// circles intersect, precomputed.
fn moon(cx: f64, cy: f64, r: f64) -> String {
    const TIP_TOP: (f64, f64) = (0.0175, -0.9998);
    const TIP_BOTTOM: (f64, f64) = (0.8905, 0.4550);
    let (tx, ty) = (cx + TIP_TOP.0 * r, cy + TIP_TOP.1 * r);
    let (bx, by) = (cx + TIP_BOTTOM.0 * r, cy + TIP_BOTTOM.1 * r);
    let bite = 0.85 * r;
    format!(
        r#"<path d="M{tx:.2} {ty:.2}A{r} {r} 0 1 0 {bx:.2} {by:.2}A{bite:.2} {bite:.2} 0 0 1 {tx:.2} {ty:.2}Z" fill="{MOON}"/>"#
    )
}

fn sky(is_day: bool, cx: f64, cy: f64, r: f64) -> String {
    if is_day {
        sun(cx, cy, r)
    } else {
        moon(cx, cy, r)
    }
}

fn rain_lines(color: &str, len: f64) -> String {
    let lines: String = [7.0, 11.5, 16.0]
        .iter()
        .map(|x| {
            format!(
                r#"<line x1="{x}" y1="17" x2="{:.1}" y2="{:.1}"/>"#,
                x - len * 0.3,
                17.0 + len
            )
        })
        .collect();
    format!(r#"<g stroke="{color}" stroke-width="1.8" stroke-linecap="round">{lines}</g>"#)
}

fn snow_dots() -> String {
    [(7.0, 18.5), (11.5, 21.0), (16.0, 18.5)]
        .iter()
        .map(|(x, y)| format!(r#"<circle cx="{x}" cy="{y}" r="1.4" fill="{SNOW}"/>"#))
        .collect()
}

pub fn body(glyph: Glyph) -> String {
    match glyph {
        Glyph::Weather { condition, is_day } => weather_body(condition, is_day),
        Glyph::Ring(color) => format!(
            r#"<circle cx="12" cy="12" r="8.5" fill="none" stroke="{color}" stroke-width="3"/><circle cx="12" cy="12" r="3" fill="{color}"/>"#
        ),
        Glyph::Pin => format!(
            r#"<path d="M12 22s-7-7.2-7-12.5a7 7 0 0 1 14 0C19 14.8 12 22 12 22Z" fill="none" stroke="{MUTED}" stroke-width="2" stroke-linejoin="round"/><circle cx="12" cy="9.5" r="2.5" fill="{MUTED}"/>"#
        ),
        Glyph::Offline => format!(
            r#"{}<line x1="4" y1="4" x2="20" y2="20" stroke="{MUTED}" stroke-width="2" stroke-linecap="round"/>"#,
            cloud(MUTED, 0.0, 0.0)
        ),
    }
}

fn weather_body(condition: Condition, is_day: bool) -> String {
    use Condition::*;
    match condition {
        Clear if is_day => sun(12.0, 12.0, 5.0),
        Clear => moon(11.0, 12.0, 7.5),
        MostlyClear => format!("{}{}", sky(is_day, 12.0, 10.0, 4.6), cloud(CLOUD, 3.0, 5.0)),
        PartlyCloudy => format!("{}{}", sky(is_day, 16.0, 7.0, 3.4), cloud(CLOUD, -1.5, 2.0)),
        Overcast => format!(
            "{}{}",
            cloud(DARK_CLOUD, 2.0, -3.0),
            cloud(CLOUD, -1.5, 2.0)
        ),
        Fog => format!(
            r#"{}<g stroke="{CLOUD}" stroke-width="1.8" stroke-linecap="round"><line x1="4" y1="18" x2="20" y2="18"/><line x1="6" y1="21.5" x2="18" y2="21.5"/></g>"#,
            cloud(DARK_CLOUD, 0.0, -4.0)
        ),
        Drizzle => format!("{}{}", cloud(CLOUD, 0.0, -4.0), rain_lines(RAIN, 2.0)),
        Rain => format!("{}{}", cloud(DARK_CLOUD, 0.0, -4.0), rain_lines(RAIN, 4.5)),
        Showers => format!(
            "{}{}{}",
            sky(is_day, 17.0, 5.0, 2.8),
            cloud(CLOUD, -1.5, -4.0),
            rain_lines(RAIN, 4.5)
        ),
        FreezingRain => format!(
            r#"{}<g stroke="{RAIN}" stroke-width="1.8" stroke-linecap="round"><line x1="7" y1="17" x2="5.8" y2="21"/><line x1="16" y1="17" x2="14.8" y2="21"/></g><circle cx="11.5" cy="20" r="1.4" fill="{SNOW}"/>"#,
            cloud(DARK_CLOUD, 0.0, -4.0)
        ),
        Snow => format!("{}{}", cloud(CLOUD, 0.0, -4.0), snow_dots()),
        Thunderstorm => format!(
            r#"{}<path d="M13 14.5 9 19.5h3l-1.5 4 5-6.2h-3l1.8-2.8Z" fill="{BOLT}"/>"#,
            cloud(DARK_CLOUD, 0.0, -4.0)
        ),
        Unknown => cloud(MUTED, 0.0, 0.0),
    }
}

/// Standalone SVG document, for a touch-strip pixmap.
pub fn svg(glyph: Glyph) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">{}</svg>"#,
        body(glyph)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weather(condition: Condition, is_day: bool) -> String {
        svg(Glyph::Weather { condition, is_day })
    }

    #[test]
    fn clear_sky_is_a_sun_by_day_and_a_moon_by_night() {
        assert!(weather(Condition::Clear, true).contains(SUN));
        assert!(!weather(Condition::Clear, true).contains(MOON));
        assert!(weather(Condition::Clear, false).contains(MOON));
        assert!(!weather(Condition::Clear, false).contains(SUN));
    }

    #[test]
    fn precipitation_draws_its_own_marks() {
        assert!(weather(Condition::Rain, true).contains(RAIN));
        assert!(weather(Condition::Snow, true).contains(SNOW));
        assert!(weather(Condition::Thunderstorm, true).contains(BOLT));
    }

    #[test]
    fn ring_uses_the_given_color() {
        assert!(svg(Glyph::Ring("#ef4444")).contains("#ef4444"));
    }

    #[test]
    fn every_glyph_is_a_standalone_svg_document() {
        let s = svg(Glyph::Pin);
        assert!(s.starts_with("<svg") && s.ends_with("</svg>"));
    }
}
