//! What each Touch Bar mode is, in words, for the mode cards and the tour.

use omatouchbar_core::config::{FnKey, Mode};

use crate::ui::glyph;

pub struct Info {
    pub glyph: &'static str,
    pub tagline: &'static str,
    pub description: &'static str,
    /// Short facts, each with a glyph, shown one by one in the tour.
    pub facts: &'static [(&'static str, &'static str)],
}

pub fn info(mode: Mode) -> Info {
    match mode {
        Mode::Keyboard => Info {
            glyph: glyph::KEYBOARD,
            tagline: "A Mac-style function row",
            description: "Brightness, keyboard light, media and volume keys, with what's playing on the right.",
            facts: &[
                (
                    glyph::BRIGHTNESS,
                    "Brightness, keyboard light, media and volume, like a Mac keyboard's top row",
                ),
                (
                    glyph::MUSIC,
                    "What's playing sits on the right. Tap it for the Sound panel",
                ),
                (
                    glyph::METER,
                    "A group's keys fill up to its level as it changes",
                ),
            ],
        },
        Mode::Bar => Info {
            glyph: glyph::BAR,
            tagline: "Omarchy's bar, on the strip",
            description: "The Omarchy menu, workspaces, clock and status widgets. Tap one to open its panel.",
            facts: &[
                (
                    glyph::MENU,
                    "The Omarchy menu, workspaces, clock and status widgets, laid out like your top bar",
                ),
                (
                    glyph::TAP,
                    "Tap a widget for its panel: Wi-Fi, Bluetooth, Sound, Battery and more",
                ),
                (
                    glyph::EYE_OFF,
                    "Can hide the on-screen bar to give the screen its top edge back",
                ),
            ],
        },
        Mode::Controls => Info {
            glyph: glyph::CONTROLS,
            tagline: "Big keys for light and sound",
            description: "The whole strip for brightness, keyboard light, playback and volume.",
            facts: &[
                (
                    glyph::BRIGHTNESS,
                    "Brightness, keyboard light, playback and volume across the whole strip",
                ),
                (glyph::TAP, "Wide keys you can hit without looking"),
                (
                    glyph::CONTROLS,
                    "Nothing opens over it: no panels, no menus",
                ),
            ],
        },
        Mode::Function => Info {
            glyph: glyph::FUNCTION,
            tagline: "Esc and F1–F12, always",
            description: "Twelve full-size keys for the shortcuts in your apps, terminals and games.",
            facts: &[
                (
                    glyph::FUNCTION,
                    "Twelve full-size keys for shortcuts in your apps, terminals and games",
                ),
                (
                    glyph::TERMINAL,
                    "Apps see real F-key presses, like any keyboard's",
                ),
                (glyph::KEYBOARD, "Nothing moves or opens under your fingers"),
            ],
        },
    }
}

/// What holding Fn shows in `mode` with the `fn_key` setting, if anything.
pub fn fn_layer(mode: Mode, fn_key: FnKey) -> Option<&'static str> {
    match (fn_key, mode) {
        (FnKey::Function, Mode::Function) => Some("the media controls"),
        (FnKey::Function, _) => Some("F1–F12"),
        (FnKey::Controls, Mode::Controls | Mode::Keyboard) => Some("F1–F12"),
        (FnKey::Controls, _) => Some("the media controls"),
        (FnKey::None, _) => None,
    }
}

pub fn fn_sentence(mode: Mode, fn_key: FnKey) -> String {
    match fn_layer(mode, fn_key) {
        Some(layer) => format!("Hold fn for {layer}"),
        None => "fn does nothing on the strip".into(),
    }
}
