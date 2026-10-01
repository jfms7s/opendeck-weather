//! WMO weather interpretation codes (what Open-Meteo's `weather_code`
//! returns), collapsed into the handful of conditions the icons can draw.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Clear,
    MostlyClear,
    PartlyCloudy,
    Overcast,
    Fog,
    Drizzle,
    FreezingRain,
    Rain,
    Snow,
    Showers,
    Thunderstorm,
    Unknown,
}

impl Condition {
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Condition::Clear,
            1 => Condition::MostlyClear,
            2 => Condition::PartlyCloudy,
            3 => Condition::Overcast,
            45 | 48 => Condition::Fog,
            51 | 53 | 55 => Condition::Drizzle,
            56 | 57 | 66 | 67 => Condition::FreezingRain,
            61 | 63 | 65 => Condition::Rain,
            71 | 73 | 75 | 77 | 85 | 86 => Condition::Snow,
            80..=82 => Condition::Showers,
            95 | 96 | 99 => Condition::Thunderstorm,
            _ => Condition::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Condition::Clear => "Clear",
            Condition::MostlyClear => "Mostly clear",
            Condition::PartlyCloudy => "Partly cloudy",
            Condition::Overcast => "Overcast",
            Condition::Fog => "Fog",
            Condition::Drizzle => "Drizzle",
            Condition::FreezingRain => "Freezing rain",
            Condition::Rain => "Rain",
            Condition::Snow => "Snow",
            Condition::Showers => "Showers",
            Condition::Thunderstorm => "Thunderstorm",
            Condition::Unknown => "Unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_documented_code_to_a_known_condition() {
        let documented = [
            0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81, 82,
            85, 86, 95, 96, 99,
        ];
        for code in documented {
            assert_ne!(
                Condition::from_code(code),
                Condition::Unknown,
                "code {code}"
            );
        }
    }

    #[test]
    fn undocumented_codes_are_unknown() {
        assert_eq!(Condition::from_code(4), Condition::Unknown);
        assert_eq!(Condition::from_code(100), Condition::Unknown);
    }
}
