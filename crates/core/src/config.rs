//! User settings, stored at `~/.config/omatouchbar/config.toml`.
//!
//! The settings app writes this file; the renderer watches it and applies
//! changes live. Every field has a default, so a partial file is valid.

use std::{
    io::{self, Write},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// A keyboard's function row: brightness, keyboard light, media and
    /// volume keys, with the now-playing player on the right that opens
    /// the Sound panel.
    #[default]
    Keyboard,
    /// The Omarchy top bar, moved onto the Touch Bar.
    Bar,
    /// Classic media and brightness controls.
    Controls,
    /// Esc and F1–F12.
    Function,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Keyboard, Mode::Bar, Mode::Controls, Mode::Function];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Bar => "Omarchy Bar",
            Mode::Keyboard => "Keyboard",
            Mode::Controls => "Media Controls",
            Mode::Function => "Function Keys",
        }
    }

    /// Whether panels and the Omarchy menu can open over this mode's home
    /// page (and return to it).
    pub fn opens_panels(self) -> bool {
        matches!(self, Mode::Bar | Mode::Keyboard)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FnKey {
    /// Holding Fn shows F1–F12 (or the controls when the mode already is
    /// function keys).
    #[default]
    Function,
    /// Holding Fn shows the media controls (or F1–F12 in the controls and
    /// keyboard modes, which already show them).
    Controls,
    /// Fn does nothing on the Touch Bar.
    None,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutSource {
    /// Mirror the widget layout from `~/.config/omarchy/shell.json`.
    #[default]
    Omarchy,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundMode {
    /// Pure black, which blends into the OLED bezel.
    #[default]
    Black,
    /// The theme's darkest background color.
    Theme,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ButtonStyle {
    #[default]
    Filled,
    Flat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceStyle {
    #[default]
    Numbers,
    Dots,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub source: LayoutSource,
    pub left: Vec<String>,
    pub center_begin: Vec<String>,
    pub center: Vec<String>,
    pub center_end: Vec<String>,
    pub right: Vec<String>,
}

impl Default for Layout {
    fn default() -> Self {
        let list = |items: &[&str]| items.iter().map(|s| s.to_string()).collect();
        Layout {
            source: LayoutSource::Omarchy,
            left: list(&["menu", "workspaces"]),
            center_begin: list(&["indicators"]),
            center: list(&["clock"]),
            center_end: Vec::new(),
            right: list(&[
                "keyboard",
                "bluetooth",
                "network",
                "audio",
                "brightness",
                "battery",
            ]),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub background: BackgroundMode,
    /// `"theme"` or a `#rrggbb` color.
    pub accent: String,
    pub button_style: ButtonStyle,
    pub text_scale: f32,
    pub workspace_style: WorkspaceStyle,
    pub min_workspaces: u32,
    /// Qt-style clock format; empty follows the Omarchy bar.
    pub clock_format: String,
    pub battery_percentage: bool,
    pub volume_percentage: bool,
    pub brightness_percentage: bool,
    pub weather_temperature: bool,
    /// Show inactive indicators directly on the bar for one-tap toggling
    /// (mirrors Omarchy's `alwaysShow` option).
    pub always_show_indicators: bool,
    /// Show the playing track next to the clock (custom layouts can also
    /// place the `media` widget anywhere).
    pub now_playing: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            background: BackgroundMode::Black,
            accent: "theme".into(),
            button_style: ButtonStyle::Filled,
            text_scale: 1.0,
            workspace_style: WorkspaceStyle::Numbers,
            min_workspaces: 5,
            clock_format: String::new(),
            battery_percentage: true,
            volume_percentage: true,
            brightness_percentage: true,
            weather_temperature: true,
            always_show_indicators: false,
            now_playing: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Motion {
    pub enabled: bool,
    /// Multiplies animation speed (0.25–3).
    pub speed: f32,
    /// 0 is critically damped; 1 is very springy.
    pub bounce: f32,
    /// Ink ripple from the touch point.
    pub ripple: bool,
    /// Stagger delay between items entering, in milliseconds.
    pub stagger_ms: f32,
    pub fps: u32,
}

impl Default for Motion {
    fn default() -> Self {
        Motion {
            enabled: true,
            speed: 1.0,
            bounce: 0.3,
            ripple: true,
            stagger_ms: 16.0,
            fps: 60,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Power {
    /// Seconds without touches before dimming (0 disables).
    pub dim_after: u32,
    /// Brightness while dimmed, 0–1.
    pub dim_level: f32,
    /// Seconds before turning the strip black (0 disables).
    pub off_after: u32,
    /// Seconds before an open panel or menu returns to the bar.
    pub panel_timeout: u32,
}

impl Default for Power {
    fn default() -> Self {
        Power {
            dim_after: 60,
            dim_level: 0.35,
            off_after: 0,
            panel_timeout: 10,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mode: Mode,
    pub fn_key: FnKey,
    /// Hide the on-screen Omarchy bar while the Touch Bar shows it.
    pub hide_system_bar: bool,
    pub show_escape: bool,
    /// Show the on-screen OSD when the Touch Bar changes volume/brightness.
    pub osd: bool,
    /// Send Touch Bar feedback to the Trackpad Haptics service.
    pub haptics: bool,
    /// Play Omarchy's screensaver on the Touch Bar while it runs on the
    /// desktop.
    pub screensaver: bool,
    pub layout: Layout,
    pub appearance: Appearance,
    pub motion: Motion,
    pub power: Power,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mode: Mode::Keyboard,
            fn_key: FnKey::Function,
            hide_system_bar: true,
            show_escape: true,
            osd: false,
            haptics: true,
            screensaver: true,
            layout: Layout::default(),
            appearance: Appearance::default(),
            motion: Motion::default(),
            power: Power::default(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        paths::config_home().join("omatouchbar/config.toml")
    }

    pub fn parse(text: &str) -> Result<Config, toml::de::Error> {
        let mut config: Config = toml::from_str(text)?;
        config.sanitize();
        Ok(config)
    }

    /// Loads the config. A missing file yields defaults; a malformed one is
    /// an error so callers can keep their previous config.
    pub fn load() -> Result<Config, String> {
        match std::fs::read_to_string(Self::path()) {
            Ok(text) => Self::parse(&text).map_err(|e| e.to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn to_toml(&self) -> String {
        let body = toml::to_string_pretty(self).unwrap_or_default();
        format!(
            "# omatouchbar settings — edited by omatouchbar-settings; changes apply live.\n\n{body}"
        )
    }

    /// Atomically writes the config file.
    pub fn save(&self) -> io::Result<()> {
        let path = Self::path();
        let dir = path.parent().expect("config file has a parent directory");
        std::fs::create_dir_all(dir)?;
        let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
        temporary.write_all(self.to_toml().as_bytes())?;
        temporary.persist(path)?;
        Ok(())
    }

    pub fn sanitize(&mut self) {
        let m = &mut self.motion;
        m.speed = if m.speed.is_finite() {
            m.speed.clamp(0.25, 3.0)
        } else {
            1.0
        };
        m.bounce = if m.bounce.is_finite() {
            m.bounce.clamp(0.0, 1.0)
        } else {
            0.3
        };
        m.stagger_ms = m.stagger_ms.clamp(0.0, 80.0);
        m.fps = m.fps.clamp(20, 120);
        let a = &mut self.appearance;
        a.text_scale = if a.text_scale.is_finite() {
            a.text_scale.clamp(0.75, 1.3)
        } else {
            1.0
        };
        a.min_workspaces = a.min_workspaces.clamp(1, 10);
        let p = &mut self.power;
        p.dim_level = p.dim_level.clamp(0.05, 1.0);
        p.panel_timeout = p.panel_timeout.clamp(3, 120);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_accepts_partial_files() {
        let config = Config::default();
        let parsed = Config::parse(&config.to_toml()).unwrap();
        assert_eq!(parsed, config);
        let partial = Config::parse("mode = \"function\"\n[motion]\nspeed = 9\n").unwrap();
        assert_eq!(partial.mode, Mode::Function);
        assert_eq!(partial.motion.speed, 3.0);
        assert!(partial.show_escape);
        assert!(partial.haptics);
        assert!(partial.screensaver);
        assert!(partial.appearance.brightness_percentage);
        assert!(partial.appearance.weather_temperature);
        assert!(!partial.appearance.now_playing);
        assert!(partial.layout.center_end.is_empty());
        assert_eq!(Config::parse("").unwrap().mode, Mode::Keyboard);
        let bar = Config::parse("mode = \"bar\"\nhaptics = false\n").unwrap();
        assert_eq!(bar.mode, Mode::Bar);
        assert!(!bar.haptics);
        assert_eq!(Config::parse(&bar.to_toml()).unwrap(), bar);
    }
}
