//! The Weather dial's 12-hour timeline: one bar per hour from the current
//! one, its height the chance of rain, its fill the hour's temperature.
//! Pure: hours in, a standalone SVG document out (a pixmap for
//! `assets/layouts/timeline.json`'s `chart` item, 130 by 88).

use crate::model::{Hour, Units};
use crate::palette::{MUTED, MUTED_TEXT, TEXT};

/// Hours shown, the current one first.
pub const HOURS: usize = 12;

const WIDTH: f64 = 130.0;
const HEIGHT: f64 = 88.0;
/// Bars stand on this line and reach `BAR_TOP` at 100%.
const BASELINE: f64 = 72.0;
const BAR_TOP: f64 = 12.0;
const BAR_WIDTH: f64 = 8.0;
const PITCH: f64 = WIDTH / HOURS as f64;
/// A 0% bar is still drawn, as a sliver, so "dry" isn't "unknown".
const MIN_BAR: f64 = 2.0;

/// Temperature bands, °C upper bounds, cold blue to hot red.
const TINTS: [(f64, &str); 6] = [
    (0.0, "#bfdbfe"),
    (10.0, "#60a5fa"),
    (18.0, "#22c55e"),
    (25.0, "#facc15"),
    (32.0, "#f97316"),
    (f64::INFINITY, "#ef4444"),
];

fn celsius(t: f64, units: Units) -> f64 {
    match units {
        Units::Metric => t,
        Units::Imperial => (t - 32.0) * 5.0 / 9.0,
    }
}

/// The fill for an hour at `temperature` (in `units`).
pub fn tint(temperature: f64, units: Units) -> &'static str {
    let c = celsius(temperature, units);
    TINTS
        .iter()
        .find(|(upper, _)| c < *upper)
        .map_or(TINTS[TINTS.len() - 1].1, |(_, colour)| colour)
}

fn bar_x(i: usize) -> f64 {
    i as f64 * PITCH + (PITCH - BAR_WIDTH) / 2.0
}

/// Bar height for a chance of rain, 0-100 (clamped).
fn bar_height(probability: f64) -> f64 {
    let full = BASELINE - BAR_TOP;
    (probability.clamp(0.0, 100.0) / 100.0 * full).max(MIN_BAR)
}

fn bar(i: usize, hour: Option<&Hour>, units: Units) -> String {
    let x = bar_x(i);
    match hour.and_then(|h| h.precipitation_probability.map(|p| (h, p))) {
        Some((h, p)) => {
            let height = bar_height(p);
            let y = BASELINE - height;
            let fill = tint(h.temperature, units);
            format!(
                r#"<rect x="{x:.1}" y="{y:.1}" width="{BAR_WIDTH}" height="{height:.1}" rx="1.5" fill="{fill}"/>"#
            )
        }
        // No chance given (past the model's horizon, or the hour is
        // missing): an empty dashed outline, never a 0% bar.
        None => {
            let height = BASELINE - BAR_TOP;
            format!(
                r#"<rect class="unknown" x="{:.1}" y="{BAR_TOP}" width="{}" height="{height}" rx="1.5" fill="none" stroke="{MUTED}" stroke-width="1" stroke-dasharray="2 2"/>"#,
                x + 0.5,
                BAR_WIDTH - 1.0
            )
        }
    }
}

/// A label centred under bar `i`; the first starts at the left edge
/// instead, so it isn't clipped.
fn label(i: usize, text: &str) -> String {
    let (x, anchor) = if i == 0 {
        (0.0, "start")
    } else {
        (bar_x(i) + BAR_WIDTH / 2.0, "middle")
    };
    format!(
        r#"<text x="{x:.1}" y="{}" text-anchor="{anchor}" font-family="sans-serif" font-size="11" font-weight="500" fill="{MUTED_TEXT}">{text}</text>"#,
        HEIGHT - 3.0
    )
}

/// The chart for `hours[0]` (the current hour) and the 11 after it; slots
/// past the end of `hours` are drawn as unknown.
pub fn timeline(hours: &[Hour], units: Units) -> String {
    let bars: String = (0..HOURS).map(|i| bar(i, hours.get(i), units)).collect();
    // The current hour: a small triangle over its bar.
    let mid = bar_x(0) + BAR_WIDTH / 2.0;
    let marker = format!(r#"<path d="M{:.1} 3h7l-3.5 5Z" fill="{TEXT}"/>"#, mid - 3.5);
    let baseline = format!(
        r#"<line x1="0" y1="{BASELINE}" x2="{WIDTH}" y2="{BASELINE}" stroke="{MUTED}" stroke-width="1"/>"#
    );
    let mut labels = label(0, "Now");
    for i in [3, 6, 9] {
        if let Some(h) = hours.get(i) {
            labels.push_str(&label(i, &h.time.format("%H").to_string()));
        }
    }
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WIDTH} {HEIGHT}">{baseline}{bars}{marker}{labels}</svg>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::tests::assert_well_formed;
    use chrono::NaiveDate;

    fn hours(probabilities: &[Option<f64>]) -> Vec<Hour> {
        probabilities
            .iter()
            .enumerate()
            .map(|(i, p)| Hour {
                time: NaiveDate::from_ymd_opt(2026, 9, 25)
                    .unwrap()
                    .and_hms_opt((14 + u32::try_from(i).unwrap()) % 24, 0, 0)
                    .unwrap(),
                temperature: 20.0,
                code: 3,
                precipitation_probability: *p,
                is_day: true,
            })
            .collect()
    }

    /// The `height` of every known bar, in order.
    fn heights(svg: &str) -> Vec<f64> {
        svg.split("<rect ")
            .skip(1)
            .filter(|r| !r.starts_with("class=\"unknown\""))
            .map(|r| {
                let h = &r[r.find("height=\"").unwrap() + 8..];
                h[..h.find('"').unwrap()].parse().unwrap()
            })
            .collect()
    }

    #[test]
    fn twelve_bars_scale_with_the_chance_of_rain() {
        let svg = timeline(
            &hours(
                &[
                    0.0, 25.0, 50.0, 100.0, 10.0, 20.0, 30.0, 40.0, 60.0, 70.0, 80.0, 90.0,
                ]
                .map(Some),
            ),
            Units::Metric,
        );
        assert_well_formed(&svg);
        let h = heights(&svg);
        assert_eq!(h.len(), HOURS);
        let full = BASELINE - BAR_TOP;
        assert_eq!(h[0], MIN_BAR);
        assert_eq!(h[1], full / 4.0);
        assert_eq!(h[2], full / 2.0);
        assert_eq!(h[3], full);
        assert!(h[4..].windows(2).all(|w| w[0] < w[1]), "{h:?}");
    }

    #[test]
    fn an_hour_without_a_chance_is_drawn_unknown_not_dry() {
        let mut probabilities = [Some(10.0); HOURS];
        probabilities[2] = None;
        let svg = timeline(&hours(&probabilities), Units::Metric);
        assert_eq!(svg.matches("class=\"unknown\"").count(), 1);
        assert_eq!(heights(&svg).len(), HOURS - 1);
        assert!(svg.contains("stroke-dasharray"));
        assert_well_formed(&svg);
    }

    #[test]
    fn missing_hours_past_the_end_are_unknown() {
        let svg = timeline(&hours(&[Some(10.0); 5]), Units::Metric);
        assert_eq!(svg.matches("class=\"unknown\"").count(), HOURS - 5);
        // Only the labels the data has.
        assert!(svg.contains(">17</text>"), "{svg}");
        assert!(!svg.contains(">20</text>"), "{svg}");
        assert_well_formed(&svg);
    }

    #[test]
    fn the_current_hour_is_marked_and_later_hours_labelled() {
        let svg = timeline(&hours(&[Some(10.0); HOURS]), Units::Metric);
        assert!(svg.contains(">Now</text>"), "{svg}");
        // Hours 3, 6 and 9 after 14:00.
        for hour in ["17", "20", "23"] {
            assert!(svg.contains(&format!(">{hour}</text>")), "{hour} in {svg}");
        }
        assert_eq!(svg.matches("<path ").count(), 1, "one marker");
        let marker_x: f64 = {
            let m = &svg[svg.find("<path d=\"M").unwrap() + 10..];
            m[..m.find(' ').unwrap()].parse().unwrap()
        };
        assert!(marker_x < PITCH, "marker over the first bar: {marker_x}");
    }

    #[test]
    fn bars_are_tinted_by_temperature_in_either_unit() {
        assert_eq!(tint(-3.0, Units::Metric), "#bfdbfe");
        assert_eq!(tint(5.0, Units::Metric), "#60a5fa");
        assert_eq!(tint(20.0, Units::Metric), "#facc15");
        assert_eq!(tint(40.0, Units::Metric), "#ef4444");
        // 68 °F = 20 °C, 23 °F = -5 °C.
        assert_eq!(tint(68.0, Units::Imperial), tint(20.0, Units::Metric));
        assert_eq!(tint(23.0, Units::Imperial), tint(-5.0, Units::Metric));
        let mut hot = hours(&[Some(50.0); HOURS]);
        hot[0].temperature = 35.0;
        assert!(timeline(&hot, Units::Metric).contains("fill=\"#ef4444\""));
    }

    #[test]
    fn out_of_range_chances_are_clamped() {
        let svg = timeline(&hours(&[Some(150.0), Some(-20.0)]), Units::Metric);
        let h = heights(&svg);
        assert_eq!(h, [BASELINE - BAR_TOP, MIN_BAR]);
    }
}
