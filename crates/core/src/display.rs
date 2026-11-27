//! Display controls shared by the Touch Bar and the settings preview.

use serde::Deserialize;

pub const TEXT_SIZES: [u8; 7] = [9, 10, 11, 12, 14, 16, 20];
const SCALE_PRESETS: [u32; 6] = [100, 125, 160, 200, 300, 400];

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Monitor {
    pub name: String,
    pub enabled: bool,
    pub focused: bool,
    pub width: u32,
    pub height: u32,
    /// EDID maker and model from Hyprland (`omarchy-monitor-state` leaves
    /// them out). Empty when unknown.
    #[serde(default)]
    pub make: String,
    #[serde(default)]
    pub model: String,
}

/// Omarchy's playful names for brightness bands (its monitor panel's
/// `brightnessName`), brightest first, with the lowest percent of each.
pub const BRIGHTNESS_NAMES: [(u8, &str); 8] = [
    (95, "Sun blast"),
    (80, "Solar flare"),
    (65, "Golden hour"),
    (45, "Even day"),
    (30, "Soft glow"),
    (20, "Lamp light"),
    (10, "Candlelit"),
    (0, "Night owl"),
];

pub fn brightness_name(percent: u8) -> &'static str {
    BRIGHTNESS_NAMES
        .iter()
        .find(|(min, _)| percent >= *min)
        .map_or("Night owl", |(_, name)| name)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayPanel {
    pub monitors: Vec<Monitor>,
    /// Brightness and scale belong to the focused output.
    pub brightness: Option<u8>,
    pub scale: Option<f64>,
    pub text_size: u8,
}

/// eDP, LVDS and DSI connectors are laptop panels.
pub fn is_internal(connector: &str) -> bool {
    ["eDP-", "LVDS-", "DSI-"]
        .iter()
        .any(|prefix| connector.starts_with(prefix))
}

impl DisplayPanel {
    pub fn focused(&self) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.enabled && m.focused)
    }

    pub fn can_set_enabled(&self, name: &str, enabled: bool) -> bool {
        self.monitors
            .iter()
            .any(|m| m.name == name && m.enabled != enabled)
            && (enabled || self.monitors.iter().filter(|m| m.enabled).count() > 1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayChange {
    Brightness { monitor: String, percent: u8 },
    TextSize(u8),
    Scale { monitor: String, preset: u32 },
    Enabled { monitor: String, enabled: bool },
}

impl Monitor {
    /// The laptop's own panel, as Omarchy tells them apart.
    pub fn internal(&self) -> bool {
        is_internal(&self.name)
    }

    /// What people call the screen: "Built-in Display" for the laptop
    /// panel, otherwise its maker and model ("LG ULTRAFINE",
    /// "Samsung Odyssey G7"), falling back to the connector.
    pub fn label(&self) -> String {
        if self.internal() {
            return "Built-in Display".into();
        }
        let model = self.model.trim();
        if model.is_empty() {
            return self.name.clone();
        }
        let brand = self.make.split_whitespace().next().unwrap_or_default();
        let named = brand.is_empty()
            || brand.eq_ignore_ascii_case("unknown")
            || model
                .split_whitespace()
                .next()
                .is_some_and(|word| word.eq_ignore_ascii_case(brand));
        if named {
            model.to_owned()
        } else {
            format!("{brand} {model}")
        }
    }

    /// (requested, effective) scale in hundredths. Omarchy rounds up to a
    /// 1/120 step yielding whole logical pixels on both axes. When two
    /// presets coincide, the desktop keeps the closer of the two.
    pub fn scales(&self) -> Vec<(u32, u32)> {
        let (mut w, mut h) = (u64::from(self.width), u64::from(self.height));
        if w == 0 || h == 0 {
            return vec![];
        }
        while h != 0 {
            (w, h) = (h, w % h);
        }
        let divisor = w * 120;
        let mut choices: Vec<(u32, u32)> = Vec::new();
        for preset in SCALE_PRESETS {
            let mut step = (u64::from(preset) * 120 / 100).min(divisor);
            while divisor % step != 0 {
                step += 1;
            }
            let effective = ((step * 100 + 60) / 120) as u32;
            if let Some((previous, value)) = choices.last_mut()
                && *value == effective
            {
                if preset.abs_diff(effective) < previous.abs_diff(effective) {
                    *previous = preset;
                }
            } else {
                choices.push((preset, effective));
            }
        }
        choices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_presets_match_the_desktop_for_clean_and_fractional_modes() {
        let mut m = Monitor {
            name: "eDP-1".into(),
            enabled: true,
            focused: true,
            width: 2880,
            height: 1800,
            ..Default::default()
        };
        assert_eq!(
            m.scales(),
            vec![
                (100, 100),
                (125, 125),
                (160, 160),
                (200, 200),
                (300, 300),
                (400, 400)
            ]
        );
        m.width = 1366;
        m.height = 768;
        assert_eq!(m.scales(), vec![(100, 100), (200, 200)]);
        m.width = 2560;
        m.height = 1440;
        assert!(m.scales().contains(&(300, 320)));
    }

    #[test]
    fn screens_are_named_like_the_desktop_names_them() {
        let screen = |name: &str, make: &str, model: &str| Monitor {
            name: name.into(),
            make: make.into(),
            model: model.into(),
            ..Default::default()
        };
        assert_eq!(
            screen("eDP-1", "Apple Computer Inc", "Color LCD").label(),
            "Built-in Display"
        );
        assert_eq!(
            screen("DP-1", "LG Electronics", "LG ULTRAFINE").label(),
            "LG ULTRAFINE"
        );
        assert_eq!(
            screen("DP-2", "Dell Inc.", "DELL U2720Q").label(),
            "DELL U2720Q"
        );
        assert_eq!(
            screen("HDMI-A-1", "Samsung Electric Company", "Odyssey G7").label(),
            "Samsung Odyssey G7"
        );
        assert_eq!(screen("HDMI-A-1", "", "").label(), "HDMI-A-1");
        assert_eq!(brightness_name(100), "Sun blast");
        assert_eq!(brightness_name(70), "Golden hour");
        assert_eq!(brightness_name(45), "Even day");
        assert_eq!(brightness_name(1), "Night owl");
    }
}
