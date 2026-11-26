//! Mirrors the Omarchy bar: its widget layout and per-widget options from
//! `~/.config/omarchy/shell.json`, plus Qt-style clock formatting.

use chrono::{Datelike, NaiveDateTime, Timelike};
use serde_json::Value;

use crate::{
    config::{Config, LayoutSource},
    paths,
    state::Indicator,
};

#[derive(Clone, Debug, PartialEq)]
pub struct City {
    pub name: String,
    pub zone: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Widget {
    Menu,
    Workspaces,
    Indicators {
        items: Vec<Indicator>,
        always_show: Option<bool>,
    },
    Tray {
        pinned: Vec<String>,
        hidden: Vec<String>,
    },
    Weather {
        refresh_minutes: u64,
        /// Omarchy's `unit` setting: "metric", "imperial" or empty for auto.
        unit: String,
    },
    Agents {
        disabled: Vec<String>,
    },
    Clock {
        format: String,
        alt: String,
    },
    WorldClock {
        cities: Vec<City>,
    },
    Update,
    Keyboard,
    Bluetooth,
    Network,
    Audio,
    Microphone,
    Brightness,
    Battery {
        percentage: Option<bool>,
    },
    ActiveWindow,
    Media,
    Spacer,
}

impl Widget {
    /// Short names used by custom layouts and the settings app.
    pub const NAMES: [(&'static str, &'static str); 18] = [
        ("menu", "Omarchy menu"),
        ("workspaces", "Workspaces"),
        ("indicators", "Indicators"),
        ("tray", "System tray"),
        ("weather", "Weather"),
        ("agents", "Agents"),
        ("clock", "Clock"),
        ("world-clock", "World clock"),
        ("update", "System update"),
        ("keyboard", "Keyboard layout"),
        ("bluetooth", "Bluetooth"),
        ("network", "Network"),
        ("audio", "Volume"),
        ("microphone", "Microphone"),
        ("brightness", "Display"),
        ("battery", "Battery"),
        ("active-window", "Active window"),
        ("media", "Now playing"),
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Widget::Menu => "menu",
            Widget::Workspaces => "workspaces",
            Widget::Tray { .. } => "tray",
            Widget::Weather { .. } => "weather",
            Widget::Agents { .. } => "agents",
            Widget::Indicators { .. } => "indicators",
            Widget::Clock { .. } => "clock",
            Widget::WorldClock { .. } => "world-clock",
            Widget::Update => "update",
            Widget::Keyboard => "keyboard",
            Widget::Bluetooth => "bluetooth",
            Widget::Network => "network",
            Widget::Audio => "audio",
            Widget::Microphone => "microphone",
            Widget::Brightness => "brightness",
            Widget::Battery { .. } => "battery",
            Widget::ActiveWindow => "active-window",
            Widget::Media => "media",
            Widget::Spacer => "spacer",
        }
    }
}

pub const DEFAULT_CLOCK: &str = "dddd HH:mm";

fn default_cities() -> Vec<City> {
    [
        ("London", "Europe/London"),
        ("New York", "America/New_York"),
        ("Tokyo", "Asia/Tokyo"),
    ]
    .iter()
    .map(|(n, z)| City {
        name: n.to_string(),
        zone: z.to_string(),
    })
    .collect()
}

fn resolve_cloned_id(id: &str) -> String {
    if !id.starts_with("omarchy.") && id.contains('.') {
        let manifest = paths::config_home()
            .join("omarchy/plugins")
            .join(id)
            .join("manifest.json");
        if let Ok(text) = std::fs::read_to_string(manifest)
            && let Some(source) = cloned_from_manifest(&text)
        {
            return source;
        }
    }
    id.to_string()
}

fn cloned_from_manifest(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    value
        .get("omarchy")?
        .get("clonedFrom")?
        .as_str()
        .map(str::to_string)
}

/// Maps a widget name (`omarchy.clock`, a user clone like `alice.menu`, or `clock`)
/// plus its options.
pub fn widget(id: &str, options: Option<&serde_json::Map<String, Value>>) -> Option<Widget> {
    let resolved = resolve_cloned_id(id);
    let name = resolved.strip_prefix("omarchy.").unwrap_or(&resolved);
    let text = |key: &str| {
        options
            .and_then(|o| o.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let names = |key: &str| {
        options
            .and_then(|m| m.get(key)?.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    };
    Some(match name {
        "menu" => Widget::Menu,
        "workspaces" => Widget::Workspaces,
        "indicators" => {
            let entries = options
                .and_then(|m| {
                    m.get("items")
                        .and_then(Value::as_array)
                        .filter(|a| !a.is_empty())
                        .or_else(|| {
                            m.get("indicators")
                                .and_then(Value::as_array)
                                .filter(|a| !a.is_empty())
                        })
                })
                .map(|list| {
                    let mut items = Vec::new();
                    for entry in list {
                        let id = entry
                            .as_str()
                            .or_else(|| entry.get("id").and_then(Value::as_str));
                        if let Some(ind) = id.and_then(Indicator::from_omarchy_id)
                            && !items.contains(&ind)
                        {
                            items.push(ind);
                        }
                    }
                    items
                })
                .unwrap_or_default();
            Widget::Indicators {
                items: entries,
                always_show: options
                    .and_then(|o| o.get("alwaysShow"))
                    .and_then(Value::as_bool),
            }
        }
        "tray" => Widget::Tray {
            pinned: names("pinned"),
            hidden: names("hidden"),
        },
        "weather" => Widget::Weather {
            refresh_minutes: options
                .and_then(|m| {
                    let v = m.get("refreshMinutes")?;
                    v.as_u64().or_else(|| v.as_str()?.trim().parse().ok())
                })
                .unwrap_or(15)
                .clamp(1, 1440),
            unit: text("unit").unwrap_or_default(),
        },
        "agents" | "model-usage" => Widget::Agents {
            disabled: options
                .and_then(|m| m.get("providers")?.as_object())
                .into_iter()
                .flatten()
                .filter(|(_, v)| v["enabled"] == false)
                .map(|(id, _)| id.clone())
                .collect(),
        },
        "clock" => Widget::Clock {
            format: text("format").unwrap_or_else(|| DEFAULT_CLOCK.into()),
            alt: text("formatAlt").unwrap_or_else(|| "d MMMM yyyy".into()),
        },
        "elsewhen" | "world-clock" => {
            let cities = options
                .and_then(|o| o.get("cities"))
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|c| {
                            Some(City {
                                name: c.get("name")?.as_str()?.to_string(),
                                zone: c.get("zone")?.as_str()?.to_string(),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .filter(|c| !c.is_empty())
                .unwrap_or_else(default_cities);
            Widget::WorldClock { cities }
        }
        "system-update" | "update" => Widget::Update,
        "keyboard-layout" | "keyboard" => Widget::Keyboard,
        "bluetooth" => Widget::Bluetooth,
        "network" => Widget::Network,
        "audio" => Widget::Audio,
        "microphone" => Widget::Microphone,
        "monitor" | "display" | "brightness" => Widget::Brightness,
        "power" | "battery" => Widget::Battery {
            percentage: options
                .and_then(|o| o.get("showPercentage"))
                .and_then(Value::as_bool),
        },
        "active-window" => Widget::ActiveWindow,
        "media" => Widget::Media,
        "spacer" => Widget::Spacer,
        _ => return None,
    })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BarLayout {
    pub left: Vec<Widget>,
    pub center_begin: Vec<Widget>,
    pub center: Vec<Widget>,
    pub center_end: Vec<Widget>,
    pub right: Vec<Widget>,
    pub tray: Vec<Widget>,
}

impl BarLayout {
    pub fn shell_json_path() -> std::path::PathBuf {
        paths::config_home().join("omarchy/shell.json")
    }

    /// Reads the Omarchy bar layout.
    pub fn from_omarchy() -> Option<BarLayout> {
        let text = std::fs::read_to_string(Self::shell_json_path())
            .or_else(|_| {
                std::fs::read_to_string(paths::omarchy_root().join("config/omarchy/shell.json"))
            })
            .ok()?;
        Self::parse_shell_json(&text)
    }

    pub fn parse_shell_json(text: &str) -> Option<BarLayout> {
        let value: Value = serde_json::from_str(text).ok()?;
        let layout = value.get("bar")?.get("layout")?;
        let section = |name: &str| -> Vec<Widget> {
            layout
                .get(name)
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|entry| {
                            if let Some(id) = entry.as_str() {
                                return widget(id, None);
                            }
                            let map = entry.as_object()?;
                            let id = map.get("id")?.as_str()?;
                            widget(id, Some(map))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(BarLayout {
            left: section("left"),
            center_begin: section("centerBegin"),
            center: section("center"),
            center_end: section("centerEnd"),
            right: section("right"),
            tray: section("tray"),
        })
    }

    pub fn from_names(config: &crate::config::Layout) -> BarLayout {
        let list = |names: &[String]| names.iter().filter_map(|n| widget(n, None)).collect();
        BarLayout {
            left: list(&config.left),
            center_begin: list(&config.center_begin),
            center: list(&config.center),
            center_end: list(&config.center_end),
            right: list(&config.right),
            tray: Vec::new(),
        }
    }

    /// The layout the Touch Bar should use for a config. `omarchy` is the
    /// already-read Omarchy layout (if any).
    pub fn resolve(config: &Config, omarchy: Option<&BarLayout>) -> BarLayout {
        let mut layout = match (config.layout.source, omarchy) {
            (LayoutSource::Omarchy, Some(o)) => o.clone(),
            _ => BarLayout::from_names(&config.layout),
        };
        if config.layout.source == LayoutSource::Omarchy
            && config.appearance.now_playing
            && !layout.contains("media")
        {
            layout.center_end.insert(0, Widget::Media);
        }
        if !config.appearance.clock_format.is_empty() {
            for w in layout.sections_mut() {
                for widget in w.iter_mut() {
                    if let Widget::Clock { format, .. } = widget {
                        *format = config.appearance.clock_format.clone();
                    }
                }
            }
        }
        layout
    }

    pub fn contains(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    pub fn find(&self, name: &str) -> Option<&Widget> {
        self.sections()
            .into_iter()
            .flatten()
            .find(|w| w.name() == name)
    }

    pub fn sections(&self) -> [&Vec<Widget>; 6] {
        [
            &self.left,
            &self.center_begin,
            &self.center,
            &self.center_end,
            &self.right,
            &self.tray,
        ]
    }

    fn sections_mut(&mut self) -> [&mut Vec<Widget>; 6] {
        [
            &mut self.left,
            &mut self.center_begin,
            &mut self.center,
            &mut self.center_end,
            &mut self.right,
            &mut self.tray,
        ]
    }

    pub fn world_cities(&self) -> Vec<City> {
        for section in self.sections() {
            for w in section {
                if let Widget::WorldClock { cities } = w {
                    return cities.clone();
                }
            }
        }
        default_cities()
    }

    /// Configured indicator list from `shell.json`, or all Omarchy indicators
    /// in default order when omitted or empty.
    pub fn indicator_items(&self) -> Vec<Indicator> {
        let mut combined = Vec::new();
        let mut saw_unfiltered = false;
        for section in self.sections() {
            for w in section {
                if let Widget::Indicators { items, .. } = w {
                    if items.is_empty() {
                        saw_unfiltered = true;
                    } else {
                        for &ind in items {
                            if !combined.contains(&ind) {
                                combined.push(ind);
                            }
                        }
                    }
                }
            }
        }
        if saw_unfiltered || combined.is_empty() {
            Indicator::ALL.to_vec()
        } else {
            combined
        }
    }
}

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Formats a time with Qt's date/time format tokens (`dddd HH:mm`, `h:mm AP`,
/// `d MMMM 'W'ww yyyy`, ...).
pub fn format_qt(format: &str, t: NaiveDateTime) -> String {
    let ampm = format.contains("AP")
        || format.contains("ap")
        || format.contains('A')
        || format.contains('a');
    let chars: Vec<char> = format.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            if chars.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        out.push('\'');
                        i += 2;
                        continue;
                    }
                    break;
                }
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == 'A' || c == 'a' {
            let upper = c == 'A';
            let text = match (t.hour() < 12, upper) {
                (true, true) => "AM",
                (false, true) => "PM",
                (true, false) => "am",
                (false, false) => "pm",
            };
            out.push_str(text);
            let pair = if upper { 'P' } else { 'p' };
            i += if chars.get(i + 1) == Some(&pair) {
                2
            } else {
                1
            };
            continue;
        }
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        let hour12 = match t.hour() % 12 {
            0 => 12,
            h => h,
        };
        let weekday = t.weekday().num_days_from_monday() as usize;
        let month = t.month0() as usize;
        let piece = match (c, n) {
            ('d', 1) => t.day().to_string(),
            ('d', 2) => format!("{:02}", t.day()),
            ('d', 3) => WEEKDAYS[weekday][..3].to_string(),
            ('d', _) => WEEKDAYS[weekday].to_string(),
            ('M', 1) => t.month().to_string(),
            ('M', 2) => format!("{:02}", t.month()),
            ('M', 3) => MONTHS[month][..3].to_string(),
            ('M', _) => MONTHS[month].to_string(),
            ('y', 2) => format!("{:02}", t.year() % 100),
            ('y', 4) => t.year().to_string(),
            ('H', 1) => t.hour().to_string(),
            ('H', _) => format!("{:02}", t.hour()),
            ('h', 1) => {
                if ampm {
                    hour12.to_string()
                } else {
                    t.hour().to_string()
                }
            }
            ('h', _) => {
                if ampm {
                    format!("{hour12:02}")
                } else {
                    format!("{:02}", t.hour())
                }
            }
            ('m', 1) => t.minute().to_string(),
            ('m', _) => format!("{:02}", t.minute()),
            ('s', 1) => t.second().to_string(),
            ('s', _) => format!("{:02}", t.second()),
            ('w', 1) => t.iso_week().week().to_string(),
            ('w', _) => format!("{:02}", t.iso_week().week()),
            _ => std::iter::repeat_n(c, n).collect(),
        };
        out.push_str(&piece);
        i += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn t() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 26)
            .unwrap()
            .and_hms_opt(14, 5, 9)
            .unwrap()
    }

    #[test]
    fn formats_omarchy_presets() {
        assert_eq!(format_qt("dddd HH:mm", t()), "Thursday 14:05");
        assert_eq!(format_qt("dddd h:mm AP", t()), "Thursday 2:05 PM");
        assert_eq!(format_qt("d MMMM 'W'ww yyyy", t()), "26 November W48 2026");
        assert_eq!(format_qt("ddd d MMM HH:mm", t()), "Thu 26 Nov 14:05");
        assert_eq!(format_qt("yyyy-MM-dd HH:mm:ss", t()), "2026-11-26 14:05:09");
    }

    #[test]
    fn new_widgets_resolve_in_custom_layouts_and_in_the_shell_tray() {
        let custom = crate::config::Layout {
            right: vec!["tray".into(), "weather".into(), "agents".into()],
            ..Default::default()
        };
        let layout = BarLayout::from_names(&custom);
        assert_eq!(
            layout.right.iter().map(Widget::name).collect::<Vec<_>>(),
            ["tray", "weather", "agents"]
        );
        let layout = BarLayout::parse_shell_json(
            r#"{"bar":{"layout":{
            "right":[{"id":"omarchy.tray","hidden":["some-app"],"pinned":["new-app"]}],
            "tray":[{"id":"omarchy.weather","refreshMinutes":7},
                   {"id":"omarchy.agents","providers":{"other":{"enabled":false}}}]
        }}}"#,
        )
        .unwrap();
        assert_eq!(
            layout.find("tray"),
            Some(&Widget::Tray {
                hidden: vec!["some-app".into()],
                pinned: vec!["new-app".into()]
            })
        );
        assert_eq!(
            layout.find("weather"),
            Some(&Widget::Weather {
                refresh_minutes: 7,
                unit: String::new()
            })
        );
        assert_eq!(
            layout.find("agents"),
            Some(&Widget::Agents {
                disabled: vec!["other".into()]
            })
        );
    }

    #[test]
    fn parses_shell_json_layout() {
        let json = r#"{"bar":{"layout":{"left":[{"id":"omarchy.menu"},{"id":"omarchy.workspaces"}],
            "center":[{"id":"omarchy.clock","format":"HH:mm"}],"right":[{"id":"omarchy.tray"},{"id":"omarchy.power","showPercentage":true}]}}}"#;
        let layout = BarLayout::parse_shell_json(json).unwrap();
        assert_eq!(layout.left, vec![Widget::Menu, Widget::Workspaces]);
        assert_eq!(
            layout.center,
            vec![Widget::Clock {
                format: "HH:mm".into(),
                alt: "d MMMM yyyy".into()
            }]
        );
        assert_eq!(
            layout.right,
            vec![
                widget("tray", None).unwrap(),
                Widget::Battery {
                    percentage: Some(true)
                }
            ]
        );
    }

    #[test]
    fn parses_indicator_widget_items_and_always_show() {
        let json = r#"{"bar":{"layout":{
            "centerBegin":[{"id":"omarchy.indicators","items":["PasswordlessSudo",{"id":"StayAwake"},"NightLight"],"alwaysShow":true}]
        }}}"#;
        let layout = BarLayout::parse_shell_json(json).unwrap();
        assert_eq!(
            layout.center_begin,
            vec![Widget::Indicators {
                items: vec![
                    Indicator::PasswordlessSudo,
                    Indicator::StayAwake,
                    Indicator::NightLight,
                ],
                always_show: Some(true),
            }]
        );
        assert_eq!(
            layout.indicator_items(),
            vec![
                Indicator::PasswordlessSudo,
                Indicator::StayAwake,
                Indicator::NightLight,
            ]
        );

        let default_json = r#"{"bar":{"layout":{"centerBegin":[{"id":"omarchy.indicators"}]}}}"#;
        let default_layout = BarLayout::parse_shell_json(default_json).unwrap();
        assert_eq!(default_layout.indicator_items(), Indicator::ALL.to_vec());
    }

    #[test]
    fn reads_cloned_from_plugin_manifest() {
        let manifest = r#"{"id":"alice.menu","omarchy":{"clonedFrom":"omarchy.menu"}}"#;
        assert_eq!(
            cloned_from_manifest(manifest).as_deref(),
            Some("omarchy.menu")
        );
        assert_eq!(cloned_from_manifest(r#"{"id":"acme.custom"}"#), None);
    }
}
