//! Weather report shown on the bar and in the Weather panel.
//!
//! Mirrors Omarchy's weather panel (`shell/plugins/panels/weather`): the
//! location lives in `~/.local/state/omarchy/settings/weather.json` (owned by
//! `omarchy-weather-location`), an empty file means "auto" (geo.omarchy.org
//! places this network), reports come from Open-Meteo, condition icons use
//! Omarchy's glyph mapping and units follow Omarchy's unit rules.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde_json::Value;

/// Weather as last reported.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Weather {
    /// Condition glyph for the bar.
    pub icon: String,
    /// Localized line from `omarchy-weather-status`. Only used when no
    /// Open-Meteo report could be fetched.
    pub status: String,
    /// The last refresh failed; what is shown is an earlier report.
    pub stale: bool,
    /// A new location was chosen and its first report is on the way.
    pub loading: bool,
    /// Where the report is for.
    pub place: Place,
    /// Where geo.omarchy.org placed this network, for the Auto option.
    pub detected: Option<String>,
    pub report: Option<Report>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Place {
    pub name: String,
    /// No location is stored: it follows this network's location.
    pub auto: bool,
}

/// A place from the location search (Open-Meteo geocoding).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Location {
    pub name: String,
    /// "Region, Country".
    pub region: String,
    /// Latitude, longitude. A typed name the search did not find has none.
    pub coords: Option<(f64, f64)>,
    /// ISO country code when known.
    pub country: Option<String>,
}

/// The location stored in `weather.json` (or answered by geo.omarchy.org,
/// which uses the same shape).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredLocation {
    pub name: String,
    pub coords: Option<(f64, f64)>,
    pub country: Option<String>,
}

impl StoredLocation {
    /// Omarchy's `isAutoLocation`: nothing names a place.
    pub fn is_auto(&self) -> bool {
        self.name.is_empty() && self.coords.is_none()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// Show °F and mph.
    pub imperial: bool,
    /// Local time at the location when the report was made.
    pub time: NaiveDateTime,
    pub now: Now,
    /// Upcoming hours, starting after `time`.
    pub hours: Vec<Hour>,
    /// Today first.
    pub days: Vec<Day>,
}

/// Current conditions, metric.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Now {
    pub temp: f32,
    pub feels: f32,
    pub humidity: f32,
    /// km/h.
    pub wind: f32,
    /// Degrees the wind blows from.
    pub wind_from: Option<f32>,
    pub code: u8,
    pub day: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hour {
    pub time: NaiveDateTime,
    pub temp: f32,
    pub code: u8,
    pub day: bool,
    /// Precipitation probability, %.
    pub rain: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub max: f32,
    pub min: f32,
    pub code: u8,
    pub rain: Option<u8>,
    pub uv: Option<f32>,
    pub sunrise: Option<NaiveTime>,
    pub sunset: Option<NaiveTime>,
}

// ------------------------------------------------------------- formatting

fn degrees(celsius: f32, imperial: bool) -> i32 {
    let v = if imperial {
        celsius * 9.0 / 5.0 + 32.0
    } else {
        celsius
    };
    v.round() as i32
}

impl Report {
    /// `12°C` / `54°F`.
    pub fn temp(&self, celsius: f32) -> String {
        format!(
            "{}°{}",
            degrees(celsius, self.imperial),
            if self.imperial { "F" } else { "C" }
        )
    }

    /// `12°`, for forecast rows.
    pub fn deg(&self, celsius: f32) -> String {
        format!("{}°", degrees(celsius, self.imperial))
    }

    /// `14 km/h NE` / `9 mph NE`.
    pub fn wind(&self) -> String {
        let speed = if self.imperial {
            format!("{} mph", (self.now.wind * 0.621_371).round() as i32)
        } else {
            format!("{} km/h", self.now.wind.round() as i32)
        };
        match self.now.wind_from.filter(|_| self.now.wind >= 1.0) {
            Some(deg) => format!("{speed} {}", compass(deg)),
            None => speed,
        }
    }

    pub fn today(&self) -> Option<&Day> {
        self.days.iter().find(|d| d.date == self.time.date())
    }

    /// Days after today.
    pub fn upcoming_days(&self) -> impl Iterator<Item = &Day> {
        let today = self.time.date();
        self.days.iter().filter(move |d| d.date > today)
    }

    /// Highest precipitation chance over the next `hours`, or today's.
    pub fn rain_chance(&self, hours: usize) -> Option<u8> {
        self.hours
            .iter()
            .take(hours)
            .filter_map(|h| h.rain)
            .max()
            .or_else(|| self.today().and_then(|d| d.rain))
    }
}

/// 8-point compass name for the direction wind blows from.
pub fn compass(degrees: f32) -> &'static str {
    const NAMES: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let i = ((degrees.rem_euclid(360.0) + 22.5) / 45.0) as usize % 8;
    NAMES[i]
}

/// WHO UV index category.
pub fn uv_level(uv: f32) -> &'static str {
    match uv.round() as i32 {
        i32::MIN..=2 => "Low",
        3..=5 => "Moderate",
        6..=7 => "High",
        8..=10 => "Very high",
        _ => "Extreme",
    }
}

/// Condition glyph for a WMO weather code, as Omarchy's
/// `iconForOpenMeteoCode` picks it.
pub fn icon(code: u8, night: bool) -> &'static str {
    match code {
        0 if night => "\u{e32b}",
        0 => "\u{e30d}",
        1 | 2 if night => "\u{e32e}",
        1 | 2 => "\u{e302}",
        45 | 48 if night => "\u{e346}",
        45 | 48 => "\u{e313}",
        51 | 53 | 55 | 56 | 57 | 61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => "\u{e318}",
        71 | 73 | 75 | 77 | 85 | 86 => "\u{e31a}",
        95 | 96 | 99 => "\u{e31d}",
        _ => "\u{e33d}",
    }
}

/// Plain-language condition for a WMO weather code.
pub fn condition(code: u8, day: bool) -> &'static str {
    match code {
        0 if day => "Sunny",
        0 => "Clear",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 => "Fog",
        48 => "Freezing fog",
        51 => "Light drizzle",
        53 => "Drizzle",
        55 => "Heavy drizzle",
        56 | 57 => "Freezing drizzle",
        61 => "Light rain",
        63 => "Rain",
        65 => "Heavy rain",
        66 | 67 => "Freezing rain",
        71 => "Light snow",
        73 => "Snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 => "Light showers",
        81 => "Showers",
        82 => "Heavy showers",
        85 => "Snow showers",
        86 => "Heavy snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm, hail",
        _ => "Cloudy",
    }
}

/// Omarchy's `shouldUseImperial`: the widget's `unit` setting, else the
/// reported country, else the locale.
pub fn uses_imperial(unit: &str, locale: &str, country: Option<&str>) -> bool {
    match unit.trim().to_ascii_lowercase().as_str() {
        "imperial" => return true,
        "metric" => return false,
        _ => {}
    }
    if let Some(country) = country.map(str::trim).filter(|c| !c.is_empty()) {
        let country = country.to_ascii_lowercase().replace(['.', '_', '-'], " ");
        return matches!(
            country.as_str(),
            "us" | "usa"
                | "united states"
                | "united states of america"
                | "lr"
                | "liberia"
                | "mm"
                | "myanmar"
                | "burma"
        );
    }
    let locale = locale.replace(['.', '-'], "_");
    ["en_US", "en_LR", "my"].iter().any(|prefix| {
        locale == *prefix
            || locale
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with(['_', '@']))
    })
}

impl Weather {
    /// The temperature for the bar: from the report, or extracted from
    /// Omarchy's localized status line (e.g. `"12°C"`, `"-3°C"`, `"68°F"`).
    pub fn temperature(&self) -> Option<String> {
        if let Some(r) = &self.report {
            return Some(r.temp(r.now.temp));
        }
        fn find(text: &str) -> Option<&str> {
            text.split_whitespace().find_map(|raw| {
                let token = raw.trim_matches(|c: char| matches!(c, ',' | ';' | '(' | ')'));
                let degree = token.find('°')?;
                let before = &token[..degree];
                let mut start = degree;
                let mut has_digit = false;
                for (idx, ch) in before.char_indices().rev() {
                    if ch.is_numeric() {
                        has_digit = true;
                        start = idx;
                    } else if matches!(ch, '.' | ',') && has_digit {
                        start = idx;
                    } else if matches!(ch, '-' | '−') && has_digit {
                        start = idx;
                        break;
                    } else {
                        break;
                    }
                }
                has_digit.then(|| &token[start..])
            })
        }
        self.status
            .split('·')
            .nth(1)
            .and_then(find)
            .or_else(|| find(&self.status))
            .map(str::to_string)
    }
}

// ---------------------------------------------------------------- parsing

/// Query parameters for the Open-Meteo forecast request.
pub fn forecast_url(latitude: f64, longitude: f64) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={latitude}&longitude={longitude}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,wind_speed_10m,\
         wind_direction_10m,weather_code,is_day\
         &hourly=temperature_2m,weather_code,precipitation_probability,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset,\
         precipitation_probability_max,uv_index_max\
         &forecast_days=7&timezone=auto"
    )
}

/// Open-Meteo geocoding search.
pub fn geocoding_url(query: &str, count: u8) -> String {
    format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count={count}&language=en&format=json",
        percent_encode(query.trim())
    )
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn local_time(v: &Value) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(v.as_str()?, "%Y-%m-%dT%H:%M").ok()
}

fn number(v: &Value) -> Option<f32> {
    v.as_f64().map(|n| n as f32)
}

fn percent(v: &Value) -> Option<u8> {
    number(v).map(|n| n.round().clamp(0.0, 100.0) as u8)
}

fn code(v: &Value) -> u8 {
    v.as_u64().map(|c| c.min(255) as u8).unwrap_or(3)
}

/// Parses an Open-Meteo forecast made with [`forecast_url`].
pub fn parse_forecast(json: &str, imperial: bool) -> Option<Report> {
    let data: Value = serde_json::from_str(json).ok()?;
    let current = &data["current"];
    let time = local_time(&current["time"])?;
    let now = Now {
        temp: number(&current["temperature_2m"])?,
        feels: number(&current["apparent_temperature"])
            .or_else(|| number(&current["temperature_2m"]))?,
        humidity: number(&current["relative_humidity_2m"]).unwrap_or(0.0),
        wind: number(&current["wind_speed_10m"]).unwrap_or(0.0),
        wind_from: number(&current["wind_direction_10m"]),
        code: code(&current["weather_code"]),
        day: current["is_day"].as_u64() != Some(0),
    };

    let column = |section: &Value, name: &str| -> Vec<Value> {
        section[name].as_array().cloned().unwrap_or_default()
    };
    let hourly = &data["hourly"];
    let (times, temps, codes, rains, days) = (
        column(hourly, "time"),
        column(hourly, "temperature_2m"),
        column(hourly, "weather_code"),
        column(hourly, "precipitation_probability"),
        column(hourly, "is_day"),
    );
    let hours = times
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let time = local_time(t)?;
            Some(Hour {
                time,
                temp: number(temps.get(i)?)?,
                code: codes.get(i).map(code).unwrap_or(3),
                day: days.get(i).and_then(Value::as_u64) != Some(0),
                rain: rains.get(i).and_then(percent),
            })
        })
        .filter(|h| h.time > time)
        .take(24)
        .collect();

    let daily = &data["daily"];
    let (dates, maxes, mins, codes, rains, uvs, rises, sets) = (
        column(daily, "time"),
        column(daily, "temperature_2m_max"),
        column(daily, "temperature_2m_min"),
        column(daily, "weather_code"),
        column(daily, "precipitation_probability_max"),
        column(daily, "uv_index_max"),
        column(daily, "sunrise"),
        column(daily, "sunset"),
    );
    let days = dates
        .iter()
        .enumerate()
        .filter_map(|(i, d)| {
            Some(Day {
                date: NaiveDate::parse_from_str(d.as_str()?, "%Y-%m-%d").ok()?,
                max: number(maxes.get(i)?)?,
                min: number(mins.get(i)?)?,
                code: codes.get(i).map(code).unwrap_or(3),
                rain: rains.get(i).and_then(percent),
                uv: uvs.get(i).and_then(number),
                sunrise: rises.get(i).and_then(local_time).map(|t| t.time()),
                sunset: sets.get(i).and_then(local_time).map(|t| t.time()),
            })
        })
        .collect();

    Some(Report {
        imperial,
        time,
        now,
        hours,
        days,
    })
}

fn time_floor(t: NaiveDateTime) -> NaiveDateTime {
    t.date()
        .and_hms_opt(chrono::Timelike::hour(&t), 0, 0)
        .unwrap_or(t)
}

/// Parses `weather.json` or a geo.omarchy.org answer (Omarchy's
/// `parseLocationFile`). Anything unreadable means auto.
pub fn parse_location(json: &str) -> StoredLocation {
    let Ok(data) = serde_json::from_str::<Value>(json) else {
        return StoredLocation::default();
    };
    let coord = |key: &str| match &data[key] {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    };
    StoredLocation {
        name: data["name"].as_str().unwrap_or("").trim().to_string(),
        coords: coord("latitude").zip(coord("longitude")),
        country: data["country"]
            .as_str()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string),
    }
}

/// Parses an Open-Meteo geocoding answer; `None` when it is not one.
pub fn parse_geocoding(json: &str) -> Option<Vec<Location>> {
    let data: Value = serde_json::from_str(json).ok()?;
    if !data.is_object() {
        return None;
    }
    let results = data["results"].as_array().cloned().unwrap_or_default();
    Some(
        results
            .iter()
            .filter_map(|r| {
                let name = r["name"].as_str()?.trim();
                if name.is_empty() {
                    return None;
                }
                let region = [&r["admin1"], &r["country"]]
                    .iter()
                    .filter_map(|v| v.as_str())
                    .filter(|s| !s.is_empty() && *s != name)
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(Location {
                    name: name.to_string(),
                    region,
                    coords: Some((r["latitude"].as_f64()?, r["longitude"].as_f64()?)),
                    country: r["country_code"].as_str().map(str::to_string),
                })
            })
            .collect(),
    )
}

// ----------------------------------------------------------------- sample

impl Report {
    /// Plausible data for previews and snapshots.
    pub fn sample(now: NaiveDateTime) -> Report {
        let hour0 = time_floor(now);
        let hours = (1..=24)
            .map(|i| {
                let time = hour0 + chrono::Duration::hours(i);
                let h = chrono::Timelike::hour(&time) as f32;
                let warmth = ((h - 14.0) / 24.0 * std::f32::consts::TAU).cos();
                Hour {
                    time,
                    temp: 9.0 + 4.0 * warmth,
                    code: [2, 2, 3, 3, 61, 63, 61, 3, 2, 1, 0, 0][i as usize % 12],
                    day: (8..17).contains(&(h as u32)),
                    rain: Some([5, 10, 20, 35, 70, 80, 60, 30, 10, 5, 0, 0][i as usize % 12]),
                }
            })
            .collect();
        let today = now.date();
        let days = [
            (13.0, 6.0, 61, 70, 2.1),
            (11.0, 4.0, 3, 20, 1.8),
            (9.0, 2.0, 71, 60, 1.0),
            (12.0, 5.0, 2, 10, 2.6),
            (14.0, 7.0, 0, 0, 3.2),
            (15.0, 8.0, 1, 5, 3.4),
            (12.0, 6.0, 95, 55, 2.0),
        ]
        .iter()
        .enumerate()
        .map(|(i, &(max, min, code, rain, uv))| Day {
            date: today + chrono::Duration::days(i as i64),
            max,
            min,
            code,
            rain: Some(rain),
            uv: Some(uv),
            sunrise: NaiveTime::from_hms_opt(7, 58, 0),
            sunset: NaiveTime::from_hms_opt(15, 47, 0),
        })
        .collect();
        Report {
            imperial: false,
            time: now,
            now: Now {
                temp: 12.0,
                feels: 9.4,
                humidity: 81.0,
                wind: 14.0,
                wind_from: Some(225.0),
                code: 2,
                day: true,
            },
            hours,
            days,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORECAST: &str = r#"{
        "current": {"time": "2026-11-26T15:15", "temperature_2m": 1.9,
            "apparent_temperature": -1.6, "relative_humidity_2m": 82,
            "wind_speed_10m": 6.1, "wind_direction_10m": 70, "weather_code": 1, "is_day": 1},
        "hourly": {
            "time": ["2026-11-26T14:00", "2026-11-26T15:00", "2026-11-26T16:00", "2026-11-26T17:00"],
            "temperature_2m": [2.0, 1.9, 1.2, null],
            "weather_code": [1, 1, 2, 3],
            "precipitation_probability": [0, 5, 40, 10],
            "is_day": [1, 1, 0, 0]},
        "daily": {
            "time": ["2026-11-26", "2026-11-27", "2026-11-28"],
            "weather_code": [1, 61, 71],
            "temperature_2m_max": [3.4, 4.6, -0.4],
            "temperature_2m_min": [-1.2, 0.2, -5.5],
            "sunrise": ["2026-11-26T08:12", "2026-11-27T08:14", "2026-11-28T08:16"],
            "sunset": ["2026-11-26T16:05", "2026-11-27T16:04", "2026-11-28T16:03"],
            "precipitation_probability_max": [10, 85, null],
            "uv_index_max": [0.6, 0.4, null]}
    }"#;

    #[test]
    fn parses_open_meteo_forecast() {
        let r = parse_forecast(FORECAST, false).expect("report");
        assert_eq!(r.temp(r.now.temp), "2°C");
        assert_eq!(r.temp(r.now.feels), "-2°C");
        assert_eq!(r.wind(), "6 km/h E");
        assert_eq!(r.now.humidity, 82.0);
        // Only hours after the reading, and only complete ones.
        assert_eq!(r.hours.len(), 1);
        assert_eq!(r.hours[0].rain, Some(40));
        assert!(!r.hours[0].day);
        assert_eq!(r.days.len(), 3);
        assert_eq!(r.today().map(|d| r.deg(d.max)), Some("3°".into()));
        assert_eq!(r.upcoming_days().count(), 2);
        assert_eq!(r.days[2].rain, None);
        assert_eq!(r.days[0].sunrise, NaiveTime::from_hms_opt(8, 12, 0));
        assert_eq!(r.rain_chance(12), Some(40));
        assert_eq!(r.deg(r.days[2].max), "0°");
    }

    #[test]
    fn converts_to_imperial() {
        let r = parse_forecast(FORECAST, true).expect("report");
        assert_eq!(r.temp(r.now.temp), "35°F");
        assert_eq!(r.wind(), "4 mph E");
        assert_eq!(r.deg(r.days[1].max), "40°");
    }

    #[test]
    fn rejects_incomplete_forecasts() {
        assert!(parse_forecast("{}", false).is_none());
        assert!(parse_forecast("not json", false).is_none());
    }

    #[test]
    fn maps_codes_like_omarchy() {
        assert_eq!(icon(0, false), "\u{e30d}");
        assert_eq!(icon(0, true), "\u{e32b}");
        assert_eq!(icon(2, true), "\u{e32e}");
        assert_eq!(icon(63, false), "\u{e318}");
        assert_eq!(icon(86, false), "\u{e31a}");
        assert_eq!(icon(99, false), "\u{e31d}");
        assert_eq!(icon(42, false), "\u{e33d}");
        assert_eq!(condition(0, false), "Clear");
        assert_eq!(condition(0, true), "Sunny");
    }

    #[test]
    fn picks_units_like_omarchy() {
        assert!(uses_imperial("imperial", "de_DE.UTF-8", Some("DE")));
        assert!(!uses_imperial("Metric", "en_US.UTF-8", Some("US")));
        assert!(uses_imperial("", "de_DE.UTF-8", Some("US")));
        assert!(uses_imperial("", "", Some("United States of America")));
        assert!(!uses_imperial("", "en_US.UTF-8", Some("PT")));
        assert!(uses_imperial("", "en_US.UTF-8", None));
        assert!(!uses_imperial("", "en_GB.UTF-8", None));
        assert!(!uses_imperial("", "en_USX", None));
        assert!(uses_imperial("", "my_MM", None));
    }

    #[test]
    fn reads_location_files() {
        let auto = parse_location("");
        assert!(auto.is_auto());
        let stored = parse_location(
            r#"{"name": " Kaliningrad ", "latitude": 54.70639, "longitude": "20.51102"}"#,
        );
        assert_eq!(stored.name, "Kaliningrad");
        assert_eq!(stored.coords, Some((54.70639, 20.51102)));
        assert!(!stored.is_auto());
        let named = parse_location(r#"{"name": "Malibu"}"#);
        assert_eq!(named.coords, None);
        assert!(!named.is_auto());
        let geo = parse_location(
            r#"{"name":"Lisbon","latitude":38.72509,"longitude":-9.1498,"country":"PT"}"#,
        );
        assert_eq!(geo.country.as_deref(), Some("PT"));
    }

    #[test]
    fn reads_geocoding_results() {
        let found = parse_geocoding(
            r#"{"results":[{"name":"Kaliningrad","latitude":54.70639,"longitude":20.51102,
                "country_code":"RU","country":"Russia","admin1":"Kaliningrad Oblast"},
                {"name":"Broken"}]}"#,
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].region, "Kaliningrad Oblast, Russia");
        assert_eq!(found[0].country.as_deref(), Some("RU"));
        assert_eq!(
            parse_geocoding(r#"{"generationtime_ms":0.2}"#),
            Some(vec![])
        );
        assert_eq!(parse_geocoding("<html>"), None);
    }

    #[test]
    fn encodes_search_queries() {
        assert_eq!(
            geocoding_url(" São Paulo ", 5),
            "https://geocoding-api.open-meteo.com/v1/search?name=S%C3%A3o%20Paulo&count=5&language=en&format=json"
        );
    }

    #[test]
    fn names_wind_directions() {
        assert_eq!(compass(0.0), "N");
        assert_eq!(compass(350.0), "N");
        assert_eq!(compass(70.0), "E");
        assert_eq!(compass(225.0), "SW");
        assert_eq!(uv_level(0.6), "Low");
        assert_eq!(uv_level(6.4), "High");
    }

    #[test]
    fn extracts_temperature_from_localized_weather_status() {
        let temp = |status: &str| {
            Weather {
                status: status.into(),
                ..Weather::default()
            }
            .temperature()
        };
        assert_eq!(
            temp("Copenhagen  ·  Temperature 12°C  ·  Wind 14 km/h").as_deref(),
            Some("12°C")
        );
        assert_eq!(
            temp("Kaliningrad  ·  Temp +4°C  ·  Wind ↘15km/h").as_deref(),
            Some("4°C")
        );
        assert_eq!(
            temp("Moscow  ·  Темп. -5°C  ·  Ветер ↑9km/h").as_deref(),
            Some("-5°C")
        );
        assert_eq!(
            temp("Oslo  ·  Temp −3°C  ·  Vind 5km/h").as_deref(),
            Some("−3°C")
        );
        assert_eq!(
            temp("New York  ·  Temp 68°F  ·  Wind 8mph").as_deref(),
            Some("68°F")
        );
        assert_eq!(
            temp("Tokyo  ·  気温 18°C  ·  風 12km/h").as_deref(),
            Some("18°C")
        );
        assert_eq!(temp("Weather unavailable"), None);
        assert_eq!(temp(""), None);
    }

    #[test]
    fn prefers_the_report_temperature() {
        let weather = Weather {
            status: "Copenhagen  ·  Temperature 12°C".into(),
            report: parse_forecast(FORECAST, false),
            ..Weather::default()
        };
        assert_eq!(weather.temperature().as_deref(), Some("2°C"));
    }
}
