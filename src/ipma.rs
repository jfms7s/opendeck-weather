//! IPMA (Portugal's weather service) official weather warnings, for the
//! warning badge on cards for a location in Portugal.
//!
//! The feed is one document for the whole country (no coordinates in the
//! request), so the area is picked here: a location inside Portugal maps to
//! the area of the nearest place in IPMA's own list (`AREAS`). Like
//! `open_meteo`, the HTTP call is thin and everything that can be wrong
//! about a response lives in the pure `parse_warnings`, tested against a
//! real captured response in `tests/fixtures/`.

use crate::model::Location;
use crate::open_meteo::{FetchError, OpenMeteo};
use chrono::{DateTime, NaiveDateTime, Utc};
use reqwest::Url;
use serde_json::Value;

const WARNINGS_URL: &str = "https://api.ipma.pt/open-data/forecast/warnings/warnings_www.json";
/// IPMA gives its times in UTC, without an offset.
const TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

/// A warning's level. IPMA's `green` means "no warning" and is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Yellow,
    Orange,
    Red,
}

impl Level {
    fn parse(id: &str) -> Option<Self> {
        match id {
            "yellow" => Some(Level::Yellow),
            "orange" => Some(Level::Orange),
            "red" => Some(Level::Red),
            _ => None,
        }
    }
}

/// What a warning is about (IPMA's `awarenessTypeName`, in Portuguese).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Rain,
    Wind,
    Fog,
    Snow,
    Heat,
    Cold,
    Thunder,
    RoughSea,
}

impl Kind {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "Precipitação" => Some(Kind::Rain),
            "Vento" => Some(Kind::Wind),
            "Nevoeiro" => Some(Kind::Fog),
            "Neve" => Some(Kind::Snow),
            "Tempo Quente" => Some(Kind::Heat),
            "Tempo Frio" => Some(Kind::Cold),
            "Trovoada" => Some(Kind::Thunder),
            "Agitação Marítima" => Some(Kind::RoughSea),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Rain => "Rain",
            Kind::Wind => "Wind",
            Kind::Fog => "Fog",
            Kind::Snow => "Snow",
            Kind::Heat => "Heat",
            Kind::Cold => "Cold",
            Kind::Thunder => "Thunder",
            Kind::RoughSea => "Rough sea",
        }
    }
}

/// One yellow, orange or red warning for one area.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub area: String,
    pub kind: Kind,
    pub level: Level,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// The warning a card shows: the highest level active now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Active {
    pub kind: Kind,
    pub level: Level,
    pub end: DateTime<Utc>,
}

#[cfg_attr(test, allow(dead_code))] // tests never reach the network
pub async fn fetch(http: &OpenMeteo) -> Result<Vec<Warning>, FetchError> {
    http.get(Url::parse(WARNINGS_URL).expect("static URL"))
        .await
        .and_then(|body| parse_warnings(&body))
}

fn time(entry: &Value, field: &str) -> Option<DateTime<Utc>> {
    let s = entry.get(field)?.as_str()?;
    NaiveDateTime::parse_from_str(s, TIME_FORMAT)
        .ok()
        .map(|t| t.and_utc())
}

/// The yellow, orange and red warnings in IPMA's feed. Green entries, and
/// entries with an unknown level or type, a missing field or a bad time,
/// are skipped: a format change hides a warning, it never breaks a card.
/// Only a body that isn't a JSON array is an error.
pub fn parse_warnings(body: &str) -> Result<Vec<Warning>, FetchError> {
    let entries: Vec<Value> =
        serde_json::from_str(body).map_err(|e| FetchError::Parse(format!("IPMA warnings: {e}")))?;
    let not_green = entries
        .iter()
        .filter(|e| e.get("awarenessLevelID").and_then(Value::as_str) != Some("green"));
    let mut skipped = 0;
    let mut warnings = Vec::new();
    for entry in not_green {
        match warning(entry) {
            Some(w) => warnings.push(w),
            None => skipped += 1,
        }
    }
    if skipped > 0 {
        // A format change would otherwise hide every warning silently.
        log::warn!("IPMA warnings: skipped {skipped} entries in an unknown format");
    }
    Ok(warnings)
}

fn warning(entry: &Value) -> Option<Warning> {
    Some(Warning {
        level: Level::parse(entry.get("awarenessLevelID")?.as_str()?)?,
        kind: Kind::parse(entry.get("awarenessTypeName")?.as_str()?)?,
        area: entry.get("idAreaAviso")?.as_str()?.to_string(),
        start: time(entry, "startTime")?,
        end: time(entry, "endTime")?,
    })
}

/// The highest-level warning for `area` active at `now` (start inclusive,
/// end exclusive); between equal levels, the one that ends last.
pub fn active(warnings: &[Warning], area: &str, now: DateTime<Utc>) -> Option<Active> {
    warnings
        .iter()
        .filter(|w| w.area == area && w.start <= now && now < w.end)
        .max_by_key(|w| (w.level, w.end))
        .map(|w| Active {
            kind: w.kind,
            level: w.level,
            end: w.end,
        })
}

/// IPMA's places with their warning area (`idAreaAviso`), from
/// `https://api.ipma.pt/open-data/distrits-islands.json` (captured in
/// `tests/fixtures/ipma-areas.json`; a test keeps the two equal). The feed
/// also has two Madeira areas with no place here, `MCN` (north coast) and
/// `MRM` (mountains): Madeira maps to `MCS` or `MPS`.
const AREAS: [(&str, f64, f64); 35] = [
    ("AVR", 40.6413, -8.6535),  // Aveiro
    ("BJA", 38.0200, -7.8700),  // Beja
    ("BRG", 41.5475, -8.4227),  // Braga
    ("BRG", 41.4434, -8.2938),  // Guimarães
    ("BGC", 41.8076, -6.7606),  // Bragança
    ("CBO", 39.8217, -7.4957),  // Castelo Branco
    ("CBR", 40.2081, -8.4194),  // Coimbra
    ("EVR", 38.5701, -7.9104),  // Évora
    ("FAR", 37.0146, -7.9331),  // Faro
    ("FAR", 37.0168, -8.9403),  // Sagres
    ("FAR", 37.1500, -8.5200),  // Portimão
    ("FAR", 37.1397, -8.0202),  // Loulé
    ("GDA", 40.5379, -7.2647),  // Guarda
    ("GDA", 40.4075, -7.5665),  // Penhas Douradas
    ("LRA", 39.7473, -8.8069),  // Leiria
    ("LSB", 38.7660, -9.1286),  // Lisboa
    ("PTG", 39.2900, -7.4200),  // Portalegre
    ("PTO", 41.1580, -8.6294),  // Porto
    ("STM", 39.2000, -8.7400),  // Santarém
    ("STB", 38.5246, -8.8856),  // Setúbal
    ("STB", 37.9560, -8.8643),  // Sines
    ("VCT", 41.6952, -8.8365),  // Viana do Castelo
    ("VRL", 41.3053, -7.7440),  // Vila Real
    ("VIS", 40.6585, -7.9120),  // Viseu
    ("MCS", 32.6485, -16.9084), // Funchal
    ("MPS", 33.0700, -16.3400), // Porto Santo
    ("AOR", 36.9563, -25.1409), // Vila do Porto
    ("AOR", 37.7415, -25.6677), // Ponta Delgada
    ("ACE", 38.6700, -27.2200), // Angra do Heroísmo
    ("ACE", 39.0800, -28.0000), // Santa Cruz da Graciosa
    ("ACE", 38.6842, -28.2133), // Velas
    ("ACE", 38.5325, -28.5237), // Madalena
    ("ACE", 38.5363, -28.6315), // Horta
    ("AOC", 39.4500, -31.1300), // Santa Cruz das Flores
    ("AOC", 39.6700, -31.1200), // Vila do Corvo
];

/// Mainland Portugal as a coarse outline, (latitude, longitude) clockwise
/// from the Minho mouth: the land border to within a few km, leaning to the
/// Spanish side except at Tui, Badajoz and Ayamonte (border towns kept
/// out), and the coast well offshore.
const MAINLAND: [(f64, f64); 30] = [
    (41.87, -8.87),   // Minho mouth
    (42.035, -8.645), // between Valença and Tui
    (42.16, -8.19),   // Melgaço, the northern tip
    (41.81, -8.10),   // Portela do Homem
    (41.92, -7.80),
    (41.98, -7.42), // north of Chaves
    (42.00, -6.95),
    (41.97, -6.55), // Rio de Onor
    (41.58, -6.17), // the eastern tip, above Miranda do Douro
    (41.50, -6.20),
    (41.09, -6.72), // the Douro at Freixo
    (41.02, -6.88), // Barca d'Alva
    (40.61, -6.78), // Vilar Formoso
    (39.99, -6.85), // Monfortinho
    (39.83, -6.95), // the Tejo at Segura
    (39.66, -7.50), // the Sever mouth
    (39.40, -7.26), // Marvão
    (39.03, -6.98), // Campo Maior
    (38.89, -7.04), // Elvas, west of Badajoz
    (38.70, -7.22),
    (38.40, -7.28),
    (38.20, -6.90), // Barrancos
    (37.97, -7.20),
    (37.55, -7.48), // Pomarão
    (37.35, -7.43),
    (37.16, -7.405), // Vila Real de Santo António, west of Ayamonte
    (36.80, -7.40),  // offshore from here on
    (36.80, -9.30),
    (39.40, -9.80),
    (41.86, -9.20),
];

/// Whether (lat, lon) is inside `polygon` (ray casting).
fn inside(polygon: &[(f64, f64)], lat: f64, lon: f64) -> bool {
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for (i, &(lat_i, lon_i)) in polygon.iter().enumerate() {
        let (lat_j, lon_j) = polygon[j];
        if (lat_i > lat) != (lat_j > lat)
            && lon < (lon_j - lon_i) * (lat - lat_i) / (lat_j - lat_i) + lon_i
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn in_portugal(lat: f64, lon: f64) -> bool {
    let madeira = (32.3..=33.2).contains(&lat) && (-17.4..=-16.0).contains(&lon);
    let azores = (36.85..=39.8).contains(&lat) && (-31.4..=-24.9).contains(&lon);
    madeira || azores || inside(&MAINLAND, lat, lon)
}

/// The IPMA warning area for a location, or `None` outside Portugal (no
/// warnings are fetched for it then).
pub fn area_for(location: &Location) -> Option<&'static str> {
    let (lat, lon) = (location.latitude, location.longitude);
    if !in_portugal(lat, lon) {
        return None;
    }
    // Equirectangular distance: plenty for "nearest of a few dozen places".
    let distance = |(_, a_lat, a_lon): &(&str, f64, f64)| {
        let dx = (a_lon - lon) * lat.to_radians().cos();
        let dy = a_lat - lat;
        dx * dx + dy * dy
    };
    AREAS
        .iter()
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        .map(|(area, _, _)| *area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(latitude: f64, longitude: f64) -> Location {
        Location {
            name: "x".into(),
            latitude,
            longitude,
            label: None,
        }
    }

    fn utc(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, hour, 0, 0).unwrap()
    }

    fn entry(area: &str, kind: &str, level: &str, start: &str, end: &str) -> String {
        format!(
            r#"{{"text": "", "awarenessTypeName": "{kind}", "idAreaAviso": "{area}", "startTime": "{start}", "awarenessLevelID": "{level}", "endTime": "{end}"}}"#
        )
    }

    #[test]
    fn the_captured_feed_parses_and_all_green_means_no_warnings() {
        // The real 2026-11-03 capture: 200 entries, every one green.
        let body = include_str!("../tests/fixtures/ipma-warnings.json");
        let raw: Vec<Value> = serde_json::from_str(body).unwrap();
        assert_eq!(raw.len(), 200);
        assert!(
            raw.iter()
                .all(|e| Kind::parse(e["awarenessTypeName"].as_str().unwrap()).is_some())
        );
        assert_eq!(parse_warnings(body).unwrap(), vec![]);
    }

    #[test]
    fn the_captured_feeds_entries_parse_once_coloured() {
        // Same entries, recoloured: every type name and time format in the
        // real feed is understood.
        let body = include_str!("../tests/fixtures/ipma-warnings.json").replace(
            r#""awarenessLevelID": "green""#,
            r#""awarenessLevelID": "yellow""#,
        );
        let warnings = parse_warnings(&body).unwrap();
        assert_eq!(warnings.len(), 200);
        let lisbon_rain = warnings
            .iter()
            .find(|w| w.area == "LSB" && w.kind == Kind::Rain)
            .unwrap();
        assert_eq!(
            lisbon_rain.start,
            Utc.with_ymd_and_hms(2026, 11, 3, 7, 24, 0).unwrap()
        );
        assert_eq!(lisbon_rain.end, utc(6, 7));
        assert_eq!(lisbon_rain.level, Level::Yellow);
    }

    #[test]
    fn unknown_levels_types_and_bad_entries_are_skipped_not_errors() {
        let body = format!(
            "[{}, {}, {}, {}, {}, {}]",
            entry(
                "LSB",
                "Vento",
                "orange",
                "2026-11-03T06:00:00",
                "2026-11-03T18:00:00"
            ),
            entry(
                "LSB",
                "Granizo",
                "red",
                "2026-11-03T06:00:00",
                "2026-11-03T18:00:00"
            ),
            entry(
                "LSB",
                "Vento",
                "purple",
                "2026-11-03T06:00:00",
                "2026-11-03T18:00:00"
            ),
            entry("LSB", "Vento", "red", "tomorrow", "2026-11-03T18:00:00"),
            r#"{"awarenessLevelID": "red"}"#,
            "42",
        );
        let warnings = parse_warnings(&body).unwrap();
        assert_eq!(
            warnings,
            vec![Warning {
                area: "LSB".into(),
                kind: Kind::Wind,
                level: Level::Orange,
                start: utc(3, 6),
                end: utc(3, 18),
            }]
        );
    }

    #[test]
    fn a_body_that_is_not_an_array_is_an_error() {
        assert!(matches!(
            parse_warnings(r#"{"error": "down"}"#),
            Err(FetchError::Parse(_))
        ));
        assert!(matches!(
            parse_warnings("<html>"),
            Err(FetchError::Parse(_))
        ));
    }

    fn feed() -> Vec<Warning> {
        let body = format!(
            "[{}, {}, {}, {}, {}]",
            entry(
                "LSB",
                "Precipitação",
                "yellow",
                "2026-11-03T06:00:00",
                "2026-11-03T21:00:00"
            ),
            entry(
                "LSB",
                "Vento",
                "orange",
                "2026-11-03T09:00:00",
                "2026-11-03T15:00:00"
            ),
            entry(
                "LSB",
                "Trovoada",
                "orange",
                "2026-11-03T09:00:00",
                "2026-11-03T18:00:00"
            ),
            entry(
                "LSB",
                "Neve",
                "red",
                "2026-11-02T00:00:00",
                "2026-11-03T00:00:00"
            ),
            entry(
                "PTO",
                "Vento",
                "red",
                "2026-11-03T00:00:00",
                "2026-11-04T00:00:00"
            ),
        );
        parse_warnings(&body).unwrap()
    }

    #[test]
    fn the_highest_active_level_for_the_area_wins() {
        let w = feed();
        // 10:00: rain (yellow), wind and thunder (orange) active; snow
        // expired; Porto's red is another area. Thunder ends last.
        assert_eq!(
            active(&w, "LSB", utc(3, 10)),
            Some(Active {
                kind: Kind::Thunder,
                level: Level::Orange,
                end: utc(3, 18)
            })
        );
        // 07:00: only the rain has started.
        assert_eq!(active(&w, "LSB", utc(3, 7)).unwrap().level, Level::Yellow);
        // 18:00: thunder ended (end is exclusive), rain still on.
        assert_eq!(active(&w, "LSB", utc(3, 18)).unwrap().kind, Kind::Rain);
        assert_eq!(active(&w, "LSB", utc(3, 21)), None);
        assert_eq!(active(&w, "FAR", utc(3, 10)), None);
    }

    #[test]
    fn expired_warnings_are_ignored() {
        let w = feed();
        // The red snow warning ended at midnight.
        assert_eq!(active(&w, "LSB", utc(3, 0)), None);
        assert_eq!(
            active(
                &w,
                "LSB",
                Utc.with_ymd_and_hms(2026, 11, 2, 23, 59, 0).unwrap()
            )
            .unwrap()
            .level,
            Level::Red
        );
    }

    #[test]
    fn the_area_list_matches_the_captured_one() {
        let raw: Value =
            serde_json::from_str(include_str!("../tests/fixtures/ipma-areas.json")).unwrap();
        let captured: Vec<(String, f64, f64)> = raw["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["idAreaAviso"].as_str().unwrap().to_string(),
                    p["latitude"].as_str().unwrap().parse().unwrap(),
                    p["longitude"].as_str().unwrap().parse().unwrap(),
                )
            })
            .collect();
        let ours: Vec<(String, f64, f64)> = AREAS
            .iter()
            .map(|(a, lat, lon)| ((*a).to_string(), *lat, *lon))
            .collect();
        assert_eq!(ours, captured);
    }

    #[test]
    fn every_area_place_is_in_portugal_and_maps_to_its_own_area() {
        for (area, lat, lon) in AREAS {
            assert_eq!(area_for(&at(lat, lon)), Some(area), "{area} {lat} {lon}");
        }
    }

    #[test]
    fn places_in_portugal_map_to_their_district() {
        for (name, lat, lon, area) in [
            ("Lisbon", 38.72509, -9.1498, "LSB"),
            ("Sintra", 38.8029, -9.3817, "LSB"),
            ("Porto", 41.1496, -8.6110, "PTO"),
            ("Faro", 37.0194, -7.9304, "FAR"),
            ("Bragança", 41.8058, -6.7572, "BGC"),
            ("Elvas", 38.8810, -7.1630, "PTG"),
            ("Miranda do Douro", 41.4960, -6.2740, "BGC"),
            ("Vila Real de Santo António", 37.1950, -7.4150, "FAR"),
            ("Funchal", 32.6669, -16.9241, "MCS"),
            ("Ponta Delgada", 37.7412, -25.6756, "AOR"),
            ("Santa Cruz das Flores", 39.4530, -31.1270, "AOC"),
        ] {
            assert_eq!(area_for(&at(lat, lon)), Some(area), "{name}");
        }
    }

    #[test]
    fn places_outside_portugal_have_no_area() {
        for (name, lat, lon) in [
            ("Madrid", 40.4168, -3.7038),
            ("Badajoz", 38.8794, -6.9707),
            ("Tui", 42.0470, -8.6440),
            ("Vigo", 42.2406, -8.7207),
            ("Ayamonte", 37.2130, -7.4040),
            ("Seville", 37.3891, -5.9845),
            ("Las Palmas", 28.1235, -15.4363),
            ("Rabat", 34.0209, -6.8416),
            ("London", 51.5072, -0.1276),
            ("Lisbon, Ohio", 40.7723, -80.7681),
        ] {
            assert_eq!(area_for(&at(lat, lon)), None, "{name}");
        }
    }
}
