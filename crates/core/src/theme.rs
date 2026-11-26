//! Touch Bar palette derived from the active Omarchy theme.
//!
//! Omarchy writes the active theme to
//! `~/.local/state/omarchy/current/theme/colors.toml` with named entries
//! (`accent`, `background`, `lighter_background`, `foreground`, ...).

use std::{collections::HashMap, path::PathBuf};

use crate::{
    color::Rgba,
    config::{BackgroundMode, Config},
    paths,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    /// Strip background (pure black by default, for the OLED panel).
    pub bar: Rgba,
    /// Button surface.
    pub surface: Rgba,
    /// Raised/pressed surface.
    pub surface_hi: Rgba,
    /// Container surface behind grouped controls (workspaces, segments).
    pub well: Rgba,
    pub border: Rgba,
    pub fg: Rgba,
    pub fg_dim: Rgba,
    pub fg_faint: Rgba,
    pub accent: Rgba,
    pub on_accent: Rgba,
    pub red: Rgba,
    pub yellow: Rgba,
    pub green: Rgba,
    pub blue: Rgba,
    pub orange: Rgba,
    pub cyan: Rgba,
    pub magenta: Rgba,
}

/// Raw named colors from a theme file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub colors: HashMap<String, Rgba>,
}

impl Palette {
    pub fn path() -> PathBuf {
        paths::state_home().join("omarchy/current/theme/colors.toml")
    }

    pub fn load() -> Option<Palette> {
        let text = std::fs::read_to_string(Self::path()).ok()?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Option<Palette> {
        let table: toml::Table = text.parse().ok()?;
        let mut colors = HashMap::new();
        for (key, value) in &table {
            if let Some(color) = value.as_str().and_then(Rgba::hex) {
                colors.insert(key.clone(), color);
            }
        }
        let background = colors.get("background")?;
        let dark = match table.get("mode").and_then(toml::Value::as_str) {
            Some("light") => false,
            Some("dark") => true,
            _ => background.luminance() < 0.5,
        };
        Some(Palette { dark, colors })
    }

    fn get(&self, names: &[&str]) -> Option<Rgba> {
        names.iter().find_map(|n| self.colors.get(*n).copied())
    }

    /// The bundled fallback: Omarchy's default dark palette.
    pub fn fallback() -> Palette {
        let text = r##"
            mode = "dark"
            accent = "#7aa2f7"
            background = "#1a1b26"
            darker_background = "#101018"
            lighter_background = "#292e42"
            foreground = "#c0caf5"
            dark_foreground = "#a9b1d6"
            muted = "#565f89"
            red = "#f7768e"
            yellow = "#e0af68"
            green = "#9ece6a"
            blue = "#7aa2f7"
            orange = "#ff9e64"
            cyan = "#7dcfff"
            magenta = "#bb9af7"
        "##;
        Self::parse(text).expect("valid fallback palette")
    }
}

impl Theme {
    pub fn from_palette(palette: &Palette, config: &Config) -> Theme {
        let p = palette;
        let background = p.get(&["background", "color0"]).unwrap_or(Rgba::BLACK);
        let fg = p.get(&["foreground", "color7"]).unwrap_or(Rgba::WHITE);
        let lighter = p
            .get(&["lighter_background", "selection"])
            .unwrap_or_else(|| background.mix(fg, 0.1));
        let light_on_black = config.appearance.background == BackgroundMode::Black && !p.dark;
        // Light palettes supply dark inks for a paper-colored desktop. Lift
        // those inks on OLED while retaining their hues and relative shades.
        let ink = |color: Rgba| {
            if light_on_black {
                color.lighten(0.45)
            } else {
                color
            }
        };
        let pick = |names: &[&str], default: Rgba| ink(p.get(names).unwrap_or(default));
        let accent = match config.appearance.accent.as_str() {
            "" | "theme" => pick(&["accent", "blue", "color4"], Rgba::rgb(0.48, 0.64, 0.97)),
            custom => Rgba::hex(custom).unwrap_or(Rgba::rgb(0.48, 0.64, 0.97)),
        };
        let bar = match config.appearance.background {
            BackgroundMode::Black => Rgba::BLACK,
            BackgroundMode::Theme => p
                .get(&["darker_background", "dark_background"])
                .unwrap_or(background),
        };
        let (lighter, fg, fg_dim, fg_faint) = if light_on_black {
            let text = background.mix(Rgba::WHITE, 0.5);
            (
                lighter.darken(0.88),
                text.darken(0.08),
                text.darken(0.26),
                text.darken(0.44),
            )
        } else {
            let dim = p
                .get(&["dark_foreground"])
                .unwrap_or_else(|| fg.mix(background, 0.35));
            let faint = p.get(&["muted"]).unwrap_or_else(|| fg.mix(background, 0.6));
            (lighter, fg, dim, faint)
        };
        // Buttons sit a step above the strip, tinted by the theme's
        // lighter background so each theme keeps its character.
        let surface = lighter.mix(fg, 0.07);
        let surface_hi = lighter.mix(fg, 0.24);
        let well = bar.mix(lighter, 0.55);
        let on_accent = if light_on_black {
            if accent.luminance() > 0.45 {
                Rgba::BLACK
            } else {
                Rgba::WHITE
            }
        } else if accent.luminance() > 0.45 {
            p.get(&["darker_background", "background"])
                .unwrap_or(Rgba::BLACK)
                .darken(0.2)
        } else {
            p.get(&["bright_foreground", "foreground"])
                .unwrap_or(Rgba::WHITE)
        };
        Theme {
            dark: p.dark || light_on_black,
            bar,
            surface,
            surface_hi,
            well,
            border: fg.alpha(0.10),
            fg,
            fg_dim,
            fg_faint,
            accent,
            on_accent,
            red: pick(&["red", "color1"], Rgba::rgb(0.94, 0.45, 0.5)),
            yellow: pick(&["yellow", "color3"], Rgba::rgb(0.88, 0.69, 0.41)),
            green: pick(&["green", "color2"], Rgba::rgb(0.6, 0.8, 0.45)),
            blue: pick(&["blue", "color4"], Rgba::rgb(0.48, 0.64, 0.97)),
            orange: pick(&["orange", "bright_red"], Rgba::rgb(1.0, 0.62, 0.4)),
            cyan: pick(&["cyan", "color6"], Rgba::rgb(0.5, 0.8, 1.0)),
            magenta: pick(&["magenta", "color5"], Rgba::rgb(0.73, 0.6, 0.97)),
        }
    }

    pub fn lerp(a: &Theme, b: &Theme, t: f32) -> Theme {
        let m = |x: Rgba, y: Rgba| x.mix(y, t);
        Theme {
            dark: if t < 0.5 { a.dark } else { b.dark },
            bar: m(a.bar, b.bar),
            surface: m(a.surface, b.surface),
            surface_hi: m(a.surface_hi, b.surface_hi),
            well: m(a.well, b.well),
            border: m(a.border, b.border),
            fg: m(a.fg, b.fg),
            fg_dim: m(a.fg_dim, b.fg_dim),
            fg_faint: m(a.fg_faint, b.fg_faint),
            accent: m(a.accent, b.accent),
            on_accent: m(a.on_accent, b.on_accent),
            red: m(a.red, b.red),
            yellow: m(a.yellow, b.yellow),
            green: m(a.green, b.green),
            blue: m(a.blue, b.blue),
            orange: m(a.orange, b.orange),
            cyan: m(a.cyan, b.cyan),
            magenta: m(a.magenta, b.magenta),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_background_adapts_light_controls() {
        let colors =
            "background = '#fff'\nforeground = '#000'\naccent = '#6e6e6e'\nblue = '#1a1a1a'";
        // Older palettes can omit the mode.
        for mode in ["", "mode = 'light'\n"] {
            let palette = Palette::parse(&format!("{mode}{colors}")).unwrap();
            let mut config = Config::default();
            let strip = Theme::from_palette(&palette, &config);
            assert_eq!(strip.bar, Rgba::BLACK);
            assert!(strip.surface.luminance() < 0.25);
            assert!(strip.fg.luminance() > strip.fg_dim.luminance());
            assert!(strip.fg_dim.luminance() > strip.fg_faint.luminance());
            assert!(strip.fg_faint.luminance() > strip.surface_hi.luminance());
            assert!(strip.blue.luminance() > strip.surface_hi.luminance());
            assert_eq!(strip.on_accent, Rgba::BLACK);

            config.appearance.background = BackgroundMode::Theme;
            let desktop = Theme::from_palette(&palette, &config);
            assert_eq!(desktop.bar, Rgba::WHITE);
            assert_eq!(desktop.fg, Rgba::BLACK);
        }
    }
}
