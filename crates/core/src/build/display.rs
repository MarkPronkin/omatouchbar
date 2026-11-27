use super::*;
use crate::{
    display::{
        BRIGHTNESS_NAMES, DisplayChange, DisplayPanel, Monitor, TEXT_SIZES, brightness_name,
    },
    state::Indicator,
};

/// Slider widths chosen so a laptop on its own fits the strip unscrolled.
const BRIGHTNESS_W: f32 = 245.0;
const KEYBOARD_W: f32 = 195.0;

impl Scene {
    /// Omarchy's Display panel: a summary of the focused screen, its
    /// brightness, the keyboard light and Night Light, then text size,
    /// scale and (with more than one) the screens themselves.
    pub(super) fn panel_display(&self, x: &mut f32, nodes: &mut Vec<Node>) -> Option<(f32, f32)> {
        let m = self.m();
        let t = &self.theme;
        let start = *x;
        let panel = self.state.display_panel.as_ref();
        let focused = panel.and_then(|p| p.focused());
        // Until the first monitor read lands (and on systems without
        // Omarchy's monitor helper), go by the screen Hyprland has focused.
        // For a laptop that is the backlight the bar already shows, keyed
        // like the real slider, so a late read only adds Text size and
        // Scale at the end instead of reflowing the row mid-animation.
        let guess = &self.state.workspaces.monitor;
        let builtin = focused.is_none()
            && panel.is_none_or(|p| p.monitors.is_empty())
            && (guess.is_empty() || crate::display::is_internal(guess));
        let brightness = match (focused, panel) {
            (Some(monitor), Some(panel)) => panel.brightness.map(|level| {
                (
                    Key::new("p/display").with(&monitor.name),
                    Slider::Monitor(Key::new(&monitor.name)),
                    level,
                )
            }),
            _ if builtin => self
                .state
                .display_brightness
                .map(|level| (Key::new("p/display").with(guess), Slider::Display, level)),
            _ => None,
        };
        let brightness_color = t.yellow.mix(t.accent, 0.25);

        let title = match focused {
            Some(monitor) => monitor.label(),
            None if builtin => "Built-in Display".to_owned(),
            None => "Display".to_owned(),
        };
        let status = match (&brightness, focused, panel) {
            (Some((_, _, level)), _, _) => brightness_name(*level),
            (None, Some(_), _) => "Fixed brightness",
            (None, None, _) if builtin => "Fixed brightness",
            (None, None, Some(_)) => "Unavailable",
            (None, None, None) => "Reading…",
        };
        let title = self.fit(&title, 230.0, m.small * 1.15, Role::Bold);
        let icon = if panel.is_some_and(|p| p.monitors.len() > 1) {
            glyph::MONITOR_MULTIPLE
        } else {
            glyph::MONITOR
        };
        let icon_color = if brightness.is_some() {
            brightness_color
        } else {
            t.fg_dim
        };
        // Sized for the longest status so a brightness drag (or the first
        // read landing) never shifts the slider under the finger.
        let widest = BRIGHTNESS_NAMES
            .iter()
            .map(|(_, name)| *name)
            .chain(["Fixed brightness", "Unavailable", "Reading…"])
            .max_by(|a, b| {
                self.tw(a, m.small, Role::Text)
                    .total_cmp(&self.tw(b, m.small, Role::Text))
            })
            .unwrap_or(status);
        let hero_key = Key::new("p/display/hero");
        let w = self
            .info(hero_key, 0.0, (icon, icon_color), &title, widest)
            .rect
            .w;
        let mut hero = self
            .info(hero_key, 0.0, (icon, icon_color), &title, status)
            .no_ripple();
        hero.rect.w = w;
        hero.press_glow = false;
        self.row_node(x, nodes, hero);

        // Night Light leads the brightness slider the way Mute leads the
        // Sound panel's volume.
        let night = self.night_light_chip(*x);
        *x += night.rect.w + m.gap;
        nodes.push(night);
        if let Some((key, slider, percent)) = brightness {
            nodes.push(
                self.slider(
                    key,
                    *x,
                    BRIGHTNESS_W,
                    percent,
                    Some(glyph::BRIGHTNESS),
                    slider,
                    brightness_color,
                )
                .group(0),
            );
            *x += BRIGHTNESS_W + m.gap;
        }
        if let Some(percent) = self.state.keyboard_backlight {
            nodes.push(
                self.slider(
                    Key::new("p/keyboard"),
                    *x,
                    KEYBOARD_W,
                    percent,
                    Some(glyph::KEYBOARD),
                    Slider::Keyboard,
                    t.accent,
                )
                .group(0),
            );
            *x += KEYBOARD_W + m.gap;
        }

        let Some(panel) = panel else {
            return Some((start, *x - m.gap));
        };
        let text_sizes = TEXT_SIZES
            .iter()
            .map(|&size| {
                (
                    size.to_string(),
                    size.to_string(),
                    Cmd::Do(Action::ConfigureDisplay(DisplayChange::TextSize(size))),
                )
            })
            .collect::<Vec<_>>();
        let label = if TEXT_SIZES.contains(&panel.text_size) {
            "Text size".to_owned()
        } else {
            format!("Text size · {}", panel.text_size)
        };
        self.segmented(
            "p/display/text",
            x,
            Some((None, &label)),
            &text_sizes,
            &panel.text_size.to_string(),
            Some(0),
            nodes,
        );
        if let Some(monitor) = focused {
            let scales = monitor
                .scales()
                .into_iter()
                .map(|(preset, effective)| {
                    (
                        effective.to_string(),
                        format!("{}×", f64::from(effective) / 100.0),
                        Cmd::Do(Action::ConfigureDisplay(DisplayChange::Scale {
                            monitor: monitor.name.clone(),
                            preset,
                        })),
                    )
                })
                .collect::<Vec<_>>();
            let active = panel
                .scale
                .map(|s| ((s * 100.0).round() as u32).to_string())
                .unwrap_or_default();
            self.segmented(
                "p/display/scale",
                x,
                Some((None, "Scale")),
                &scales,
                &active,
                Some(0),
                nodes,
            );
        }
        if panel.monitors.len() > 1 {
            *x += m.gap;
            for monitor in &panel.monitors {
                let node = self.monitor_chip(panel, monitor);
                self.row_node(x, nodes, node);
            }
        }
        Some((start, *x - m.gap))
    }

    /// Night Light as an icon button, tinted while it warms the screen.
    fn night_light_chip(&self, x: f32) -> Node {
        let m = self.m();
        let t = &self.theme;
        let on = self.state.indicators.nightlight;
        let mut node = self
            .chip(
                Key::new("p/display/night"),
                Some((
                    glyph::NIGHTLIGHT,
                    Role::Icon,
                    if on { t.yellow } else { t.fg_dim },
                )),
                None,
                Fill {
                    color: t.surface,
                    emph: Some(t.yellow.alpha(0.2).over(t.surface)),
                    radius: m.radius,
                    border: None,
                    sheen: true,
                },
            )
            .tap(Cmd::TriggerIndicator(Indicator::NightLight))
            .group(0);
        node.rect.x = x;
        node.emph = if on { 1.0 } else { 0.0 };
        node
    }

    /// A screen as a two-line card, like a Bluetooth device: its name over
    /// whether it is on. Tapping turns it on or off, except the last one on.
    fn monitor_chip(&self, panel: &DisplayPanel, monitor: &Monitor) -> Node {
        let m = self.m();
        let t = &self.theme;
        let enabled = !monitor.enabled;
        let can_toggle = panel.can_set_enabled(&monitor.name, enabled);
        let state = if !monitor.enabled {
            "Off"
        } else if monitor.focused {
            "Focused"
        } else {
            "On"
        };
        let status = if monitor.internal() {
            state.to_owned()
        } else {
            format!("{state} · {}", monitor.name)
        };
        let icon_color = match (monitor.enabled, monitor.focused) {
            (true, true) => t.accent,
            (true, false) => t.fg,
            _ => t.fg_dim,
        };
        let icon = if monitor.internal() {
            glyph::LAPTOP
        } else {
            glyph::MONITOR
        };
        let name = self.fit(&monitor.label(), 240.0, m.small * 1.15, Role::Bold);
        let mut node = self.two_line_chip_colored(
            Key::new("p/display/output").with(&monitor.name),
            (icon, icon_color),
            &name,
            (&status, t.fg_dim),
            monitor.enabled.then_some((glyph::CHECK, t.accent)),
        );
        node.emph = if monitor.enabled { 1.0 } else { 0.0 };
        if can_toggle {
            node = node.tap(Cmd::Do(Action::ConfigureDisplay(DisplayChange::Enabled {
                monitor: monitor.name.clone(),
                enabled,
            })));
        } else {
            node = node.no_ripple();
            node.press_glow = false;
        }
        node
    }
}
