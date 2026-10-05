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
