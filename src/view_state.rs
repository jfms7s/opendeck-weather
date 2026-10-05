//! Each action's in-memory UI state (what a press, turn or tap has scrolled
//! or toggled to), with the rules for changing it next to the type. Pure:
//! the actions apply these on input and `views` reads them; the tracker
//! stores them without knowing what they mean.

/// Weather: the resting screen, an hour ahead, or the details screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WeatherView {
    #[default]
    Now,
    /// Hours ahead of the current one, `1..=MAX_HOURS_AHEAD`.
    Hour(u8),
    Details,
}

impl WeatherView {
    /// How far ahead a Weather dial can scroll.
    pub const MAX_HOURS_AHEAD: u8 = 24;

    /// Scrolls the hourly forecast, clamped between now and the horizon;
    /// leaves the details screen.
    pub fn scroll(self, ticks: i16) -> Self {
        let from = match self {
            WeatherView::Hour(h) => i32::from(h),
            _ => 0,
        };
        let to = (from + i32::from(ticks)).clamp(0, i32::from(Self::MAX_HOURS_AHEAD));
        match u8::try_from(to) {
            Ok(0) | Err(_) => WeatherView::Now,
            Ok(h) => WeatherView::Hour(h),
        }
    }

    /// Opens the details screen, or closes it back to now.
    pub fn toggle_details(self) -> Self {
        match self {
            WeatherView::Details => WeatherView::Now,
            _ => WeatherView::Details,
        }
    }
}

/// Forecast: days stepped past the configured day, and the sunrise/sunset
/// screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ForecastView {
    /// Unwrapped on purpose: only `views::forecast` knows how many days the
    /// forecast still covers, so it alone wraps this.
    pub days: i32,
    pub sun: bool,
}

impl ForecastView {
    pub fn step(self, days: i32) -> Self {
        ForecastView {
            days: self.days.wrapping_add(days),
            sun: false,
        }
    }

    pub fn toggle_sun(self) -> Self {
        ForecastView {
            sun: !self.sun,
            ..self
        }
    }
}

/// Air Quality: the index, then one page per pollutant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AirQualityPage {
    #[default]
    Index,
    Pm25,
    Pm10,
    Ozone,
    NitrogenDioxide,
}

impl AirQualityPage {
    const ORDER: [AirQualityPage; 5] = [
        AirQualityPage::Index,
        AirQualityPage::Pm25,
        AirQualityPage::Pm10,
        AirQualityPage::Ozone,
        AirQualityPage::NitrogenDioxide,
    ];

    /// Pages forward (or back, for negative `pages`), wrapping around.
    pub fn step(self, pages: i32) -> Self {
        let len = Self::ORDER.len() as i32;
        let here = Self::ORDER.iter().position(|p| *p == self).unwrap_or(0) as i32;
        Self::ORDER[(here + pages).rem_euclid(len) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_scrolling_clamps_between_now_and_the_horizon() {
        assert_eq!(WeatherView::Now.scroll(-3), WeatherView::Now);
        assert_eq!(WeatherView::Now.scroll(2), WeatherView::Hour(2));
        assert_eq!(WeatherView::Hour(2).scroll(-2), WeatherView::Now);
        assert_eq!(
            WeatherView::Now.scroll(100),
            WeatherView::Hour(WeatherView::MAX_HOURS_AHEAD)
        );
        assert_eq!(WeatherView::Details.scroll(1), WeatherView::Hour(1));
    }

    #[test]
    fn weather_details_toggle_returns_to_now() {
        assert_eq!(WeatherView::Hour(5).toggle_details(), WeatherView::Details);
        assert_eq!(WeatherView::Details.toggle_details(), WeatherView::Now);
    }

    #[test]
    fn forecast_stepping_moves_the_offset_and_leaves_the_sun_screen() {
        let v = ForecastView { days: 6, sun: true }.step(1);
        assert_eq!(
            v,
            ForecastView {
                days: 7,
                sun: false
            }
        );
        assert_eq!(v.step(-8).days, -1);
        assert!(v.toggle_sun().sun);
        assert_eq!(v.toggle_sun().days, 7);
    }

    #[test]
    fn air_quality_paging_wraps_in_both_directions() {
        assert_eq!(
            AirQualityPage::Index.step(-1),
            AirQualityPage::NitrogenDioxide
        );
        assert_eq!(
            AirQualityPage::NitrogenDioxide.step(1),
            AirQualityPage::Index
        );
        assert_eq!(AirQualityPage::Index.step(2), AirQualityPage::Pm10);
        assert_eq!(AirQualityPage::Index.step(5), AirQualityPage::Index);
    }
}
