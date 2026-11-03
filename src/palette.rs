//! The colors every surface shares, so a key, a dial layout and the icons
//! can't drift apart. `assets/layouts/card.json` repeats the text colors as
//! its defaults; a test in `card.rs` keeps them equal.

/// Key tile background.
pub const BACKGROUND: &str = "#111827";
/// The big value line.
pub const TEXT: &str = "#f9fafb";
/// Label and detail lines.
pub const MUTED_TEXT: &str = "#d1d5db";
/// Placeholder glyphs (no location, no data) and "no AQI" gray.
pub const MUTED: &str = "#6b7280";
/// The value of a card showing data older than the cache TTL, and its badge.
pub const STALE: &str = "#9ca3af";

/// UV-index band colours (WHO: green, yellow, orange, red, violet), for the
/// value on the second Weather details page.
pub const UV_LOW: &str = "#22c55e";
pub const UV_MODERATE: &str = "#eab308";
pub const UV_HIGH: &str = "#f97316";
pub const UV_VERY_HIGH: &str = "#ef4444";
pub const UV_EXTREME: &str = "#a855f7";

/// IPMA warning levels (yellow, orange, red), for the warning badge.
pub const WARNING_YELLOW: &str = "#facc15";
pub const WARNING_ORANGE: &str = "#f97316";
pub const WARNING_RED: &str = "#ef4444";

/// Air-quality category colours, shared by the US and European scales
/// (`views::aqi_category` maps each scale's bands onto them) and the Air
/// Quality icon.
pub const AQI_GOOD: &str = "#22c55e";
pub const AQI_FAIR: &str = "#84cc16";
pub const AQI_MODERATE: &str = "#eab308";
pub const AQI_POOR: &str = "#f97316";
pub const AQI_VERY_POOR: &str = "#ef4444";
pub const AQI_HAZARDOUS: &str = "#a855f7";
