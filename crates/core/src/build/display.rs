use super::*;
use crate::display::{DisplayChange, TEXT_SIZES};

impl Scene {
    pub(super) fn panel_display(&self, x: &mut f32, nodes: &mut Vec<Node>) -> Option<(f32, f32)> {
        let m = self.m();
        let t = &self.theme;
        let start = *x;
        let panel = self.state.display_panel.as_ref();
        let focused = panel.and_then(|p| p.focused());
        let brightness = match (focused, panel) {
            (Some(monitor), Some(panel)) => panel.brightness.map(|level| {
                (
                    Key::new("p/display").with(&monitor.name),
                    Slider::Monitor(Key::new(&monitor.name)),
                    level,
                )
            }),
            // Keep the laptop slider on systems without Omarchy's monitor helper.
            (None, Some(panel))
                if panel.monitors.is_empty() && {
                    let name = &self.state.workspaces.monitor;
                    name.is_empty()
                        || ["eDP-", "LVDS-", "DSI-"]
                            .iter()
                            .any(|prefix| name.starts_with(prefix))
                } =>
            {
                self.state
                    .display_brightness
                    .map(|level| (Key::new("p/display"), Slider::Display, level))
            }
            _ => None,
        };
        if let Some((key, slider, percent)) = brightness {
            nodes.push(
                self.slider(
                    key,
                    *x,
                    255.0,
                    percent,
                    Some(glyph::BRIGHTNESS),
                    slider,
                    t.yellow.mix(t.accent, 0.25),
                )
                .group(0),
            );
            *x += 255.0 + m.gap;
        } else {
            let status = if panel.is_none() {
                "Reading displays…"
            } else if focused.is_some() {
                "Fixed brightness"
            } else {
                "Display unavailable"
            };
            self.panel_note(Key::new("p/display/status"), x, status, nodes);
        }
        if let Some(percent) = self.state.keyboard_backlight {
            nodes.push(
                self.slider(
                    Key::new("p/keyboard"),
                    *x,
                    205.0,
                    percent,
                    Some(glyph::KEYBOARD),
                    Slider::Keyboard,
                    t.accent,
                )
                .group(0),
            );
            *x += 205.0 + m.gap;
        }
        if let Some(panel) = panel {
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
                "Text size (px)".to_owned()
            } else {
                format!("Text size ({} px)", panel.text_size)
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
                let label = if panel.monitors.iter().filter(|m| m.enabled).count() > 1 {
                    format!("Scale · {}", monitor.name)
                } else {
                    "Scale".to_owned()
                };
                let active = panel
                    .scale
                    .map(|s| ((s * 100.0).round() as u32).to_string())
                    .unwrap_or_default();
                self.segmented(
                    "p/display/scale",
                    x,
                    Some((None, &label)),
                    &scales,
                    &active,
                    Some(0),
                    nodes,
                );
            }
            if panel.monitors.len() > 1 {
                for monitor in &panel.monitors {
                    let enabled = !monitor.enabled;
                    let can_toggle = panel.can_set_enabled(&monitor.name, enabled);
                    let label = if monitor.focused {
                        format!("{} (focused)", monitor.name)
                    } else {
                        monitor.name.clone()
                    };
                    let mut chip = self.list_chip(
                        Key::new("p/display/output").with(&monitor.name),
                        x,
                        (glyph::MONITOR, t.fg_dim),
                        &label,
                        monitor.enabled.then_some((glyph::CHECK, t.accent)),
                        monitor.enabled,
                        can_toggle.then(|| {
                            Cmd::Do(Action::ConfigureDisplay(DisplayChange::Enabled {
                                monitor: monitor.name.clone(),
                                enabled,
                            }))
                        }),
                    );
                    if !can_toggle {
                        chip.opacity = 0.45;
                    }
                    nodes.push(chip);
                }
            }
        }
        Some((start, *x - m.gap))
    }
}
